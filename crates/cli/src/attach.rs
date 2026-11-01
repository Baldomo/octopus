use std::io::{Read, Write};

use octopus_client::Connection;
use octopus_proto::term::Piece;
use octopus_proto::types::Stream;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

const CTRL_RIGHT_BRACKET_DETACHES: u8 = 0x1d;
const KEY_BYTES: usize = 1024;
/// RIS: the terminal clears its screen, history and modes, as a client does before it draws a screen.
const RESET: &[u8] = b"\x1bc";

struct RawMode;

impl RawMode {
    fn enter() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _restored_or_not_a_tty = crossterm::terminal::disable_raw_mode();
    }
}

/// Finishes the stream on the way out, so the server reads a detach, not a lost connection.
pub async fn run(connection: &Connection, stream: &Stream) -> anyhow::Result<()> {
    let opened = connection.open_data_stream(stream).await.map_err(crate::call_error)?;
    let (from_pty, mut to_pty) = tokio::io::split(opened);
    let _raw = RawMode::enter()?;
    let (keys, mut typed) = mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0u8; KEY_BYTES];
        while let Ok(read) = stdin.read(&mut buffer) {
            if read == 0 || keys.send(buffer[..read].to_vec()).is_err() {
                break;
            }
        }
    });
    let showing = async {
        let mut from_pty = BufReader::new(from_pty);
        let mut stdout = std::io::stdout();
        while let Some(piece) = Piece::read_from(&mut from_pty).await? {
            match piece {
                Piece::Screen(screen) => {
                    stdout.write_all(RESET)?;
                    stdout.write_all(&screen.bytes)?;
                }
                Piece::Output(bytes) => stdout.write_all(&bytes)?,
            }
            stdout.flush()?;
        }
        anyhow::Ok(())
    };
    let typing = async {
        while let Some(keys) = typed.recv().await {
            if keys.contains(&CTRL_RIGHT_BRACKET_DETACHES) {
                break;
            }
            to_pty.write_all(&keys).await?;
        }
        anyhow::Ok(())
    };
    let ended = tokio::select! {
        shown = showing => shown,
        typed = typing => typed,
    };
    let _server_gone = to_pty.shutdown().await;
    ended
}
