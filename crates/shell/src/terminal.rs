use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use octopus_client::{CallError, Connection};
use octopus_proto::term::{Piece, Screen};
use octopus_proto::types::{PtyId, Stream};
use serde::Serialize;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use ts_rs::TS;

use crate::lock;

type Inputs = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>>;

/// Opens a screen: the UI resets its terminal to cols×rows, then draws the bytes that follow.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
pub struct TermScreen {
    pub cols: u16,
    pub rows: u16,
}

/// The terminals on screen, each attached over its own `pty:<id>` data stream. A terminal off screen
/// holds no attachment.
#[derive(Default)]
pub struct Terminals {
    next: AtomicU64,
    inputs: Inputs,
}

impl Terminals {
    /// Streams the terminal into `output`: a `TermScreen` followed by its bytes, or output bytes; an empty message
    /// ends it.
    pub async fn attach(
        &self,
        connection: &Connection,
        pty: PtyId,
        output: Channel<InvokeResponseBody>,
    ) -> Result<u64, CallError> {
        let stream = connection.open_data_stream(&Stream::Pty(pty.clone())).await?;
        let (from_pty, mut to_pty) = tokio::io::split(stream);
        let (input, mut typed) = mpsc::unbounded_channel::<Vec<u8>>();
        let attachment = self.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.inputs).insert(attachment, input);
        let inputs = Arc::clone(&self.inputs);
        tauri::async_runtime::spawn(async move {
            let showing = async {
                let mut from_pty = BufReader::new(from_pty);
                loop {
                    let bytes = match Piece::read_from(&mut from_pty).await {
                        Ok(Some(Piece::Screen(Screen { cols, rows, bytes }))) => {
                            let screen =
                                serde_json::to_string(&TermScreen { cols, rows }).expect("a screen serializes");
                            if output.send(InvokeResponseBody::Json(screen)).is_err() {
                                return;
                            }
                            bytes
                        }
                        Ok(Some(Piece::Output(bytes))) => bytes,
                        Ok(None) => return,
                        Err(error) => return eprintln!("octopus app: reading {pty}: {error}"),
                    };
                    if output.send(InvokeResponseBody::Raw(bytes.to_vec())).is_err() {
                        return;
                    }
                }
            };
            let typing = async {
                while let Some(keys) = typed.recv().await {
                    if let Err(error) = to_pty.write_all(&keys).await {
                        return eprintln!("octopus app: writing {pty}: {error}");
                    }
                }
            };
            tokio::select! {
                () = showing => {}
                () = typing => {}
            }
            lock(&inputs).remove(&attachment);
            let _server_gone = to_pty.shutdown().await;
            let _ui_gone = output.send(InvokeResponseBody::Raw(Vec::new()));
        });
        Ok(attachment)
    }

    pub fn input(&self, attachment: u64, keys: Vec<u8>) -> Result<(), String> {
        lock(&self.inputs)
            .get(&attachment)
            .and_then(|input| input.send(keys).ok())
            .ok_or_else(|| format!("terminal attachment {attachment} has ended"))
    }

    /// Dropping the input ends the attachment's task, which finishes its data stream: the server reads that as this
    /// client no longer showing the terminal.
    pub fn detach(&self, attachment: u64) {
        lock(&self.inputs).remove(&attachment);
    }
}
