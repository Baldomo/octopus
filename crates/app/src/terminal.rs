use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use octopus_client::{CallError, Connection};
use octopus_proto::types::{PtyId, Stream};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::lock;

const OUTPUT_BYTES: usize = 64 * 1024;

type Inputs = Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>>;

/// The terminals on screen, each attached over its own `pty:<id>` data stream. A terminal off screen
/// holds no attachment.
#[derive(Default)]
pub struct Terminals {
    next: AtomicU64,
    inputs: Inputs,
}

impl Terminals {
    /// Streams the terminal's scrollback, then its live output, into `output`; an empty message ends it.
    pub async fn attach(
        &self,
        connection: &Connection,
        pty: PtyId,
        output: Channel<InvokeResponseBody>,
    ) -> Result<u64, CallError> {
        let stream = connection.open_data_stream(&Stream::Pty(pty.clone())).await?;
        let (mut from_pty, mut to_pty) = tokio::io::split(stream);
        let (input, mut typed) = mpsc::unbounded_channel::<Vec<u8>>();
        let attachment = self.next.fetch_add(1, Ordering::Relaxed);
        lock(&self.inputs).insert(attachment, input);
        let inputs = Arc::clone(&self.inputs);
        tauri::async_runtime::spawn(async move {
            let mut buffer = vec![0u8; OUTPUT_BYTES];
            loop {
                tokio::select! {
                    read = from_pty.read(&mut buffer) => match read {
                        Ok(0) => break,
                        Ok(read) => {
                            if output.send(InvokeResponseBody::Raw(buffer[..read].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            eprintln!("octopus app: reading {pty}: {error}");
                            break;
                        }
                    },
                    keys = typed.recv() => match keys {
                        Some(keys) => {
                            if let Err(error) = to_pty.write_all(&keys).await {
                                eprintln!("octopus app: writing {pty}: {error}");
                                break;
                            }
                        }
                        None => break,
                    },
                }
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
