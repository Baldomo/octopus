use std::io::{Read, Write};

use octopus_client::Connection;
use octopus_proto::types::Stream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

const CTRL_RIGHT_BRACKET_DETACHES: u8 = 0x1d;
const KEY_BYTES: usize = 1024;
const OUTPUT_BYTES: usize = 8192;

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
    let (mut from_pty, mut to_pty) = tokio::io::split(opened);
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
    let mut buffer = [0u8; OUTPUT_BYTES];
    let mut stdout = std::io::stdout();
    loop {
        tokio::select! {
            read = from_pty.read(&mut buffer) => match read {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    stdout.write_all(&buffer[..read])?;
                    stdout.flush()?;
                }
            },
            keys = typed.recv() => match keys {
                Some(keys) if keys.contains(&CTRL_RIGHT_BRACKET_DETACHES) => break,
                Some(keys) => to_pty.write_all(&keys).await?,
                None => break,
            },
        }
    }
    let _server_gone = to_pty.shutdown().await;
    Ok(())
}
