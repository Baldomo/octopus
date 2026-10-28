//! This client's microphone, sent to every listening terminal whose lock it holds.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use octopus_client::Connection;
use octopus_proto::types::{MIC_FORMAT, PtyId, Stream};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

use crate::lock;

/// Chunks of captured audio waiting for a slow stream; one that falls further behind skips ahead.
const BACKLOG: usize = 64;

/// A terminal this client sends its microphone to.
pub type Listener = (String, PtyId);

pub type Chunk = Arc<[u8]>;

/// Linear resampling of mono `f32` to `MIC_FORMAT`, carrying its position across calls.
pub struct Resampler {
    step: f64,
    position: f64,
    previous: f32,
}

impl Resampler {
    pub fn new(rate: u32) -> Self {
        Self { step: f64::from(rate) / f64::from(MIC_FORMAT.rate), position: 0.0, previous: 0.0 }
    }

    /// Takes `frames` of `channels` interleaved samples, mixes them to mono, and returns `MIC_FORMAT` bytes.
    pub fn convert(&mut self, frames: &[f32], channels: usize) -> Vec<u8> {
        let mono: Vec<f32> =
            frames.chunks_exact(channels).map(|frame| frame.iter().sum::<f32>() / channels as f32).collect();
        let mut out = Vec::new();
        // `position` counts input frames from `previous`, the last frame of the call before.
        while self.position < mono.len() as f64 {
            let at = self.position.floor() as usize;
            let before = if at == 0 { self.previous } else { mono[at - 1] };
            let after = mono[at];
            let fraction = (self.position - self.position.floor()) as f32;
            let sample = before + (after - before) * fraction;
            out.extend_from_slice(&((sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16).to_le_bytes());
            self.position += self.step;
        }
        self.position -= mono.len() as f64;
        if let Some(last) = mono.last() {
            self.previous = *last;
        }
        out
    }
}

/// The default input device, captured on a thread of its own, since a capture stream stays on the thread that made
/// it. Dropping it stops the capture.
struct Capture {
    audio: broadcast::Sender<Chunk>,
    _stop: std::sync::mpsc::Sender<()>,
}

impl Capture {
    fn start() -> Result<Self, String> {
        let (audio, _) = broadcast::channel(BACKLOG);
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let (started, outcome) = std::sync::mpsc::channel::<Result<(), String>>();
        let sender = audio.clone();
        std::thread::spawn(move || match open_input(sender) {
            Ok(stream) => {
                let _caller_gone = started.send(Ok(()));
                let _until_dropped = stopped.recv();
                drop(stream);
            }
            Err(error) => {
                let _caller_gone = started.send(Err(error));
            }
        });
        outcome.recv().map_err(|_| "the capture thread ended".to_string())??;
        Ok(Self { audio, _stop: stop })
    }
}

/// The device a remote holder makes this machine's default input, which carries a terminal's audio, not this user's.
const OCTOPUS_MICROPHONE: &str = "Octopus Microphone";

fn is_octopus(device: &cpal::Device) -> bool {
    device.description().is_ok_and(|description| description.name() == OCTOPUS_MICROPHONE)
}

/// The default input, or the first other one while the default is the Octopus Microphone.
fn microphone() -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match host.default_input_device() {
        Some(device) if !is_octopus(&device) => Ok(device),
        _ => host
            .input_devices()
            .map_err(|error| format!("list the microphones: {error}"))?
            .find(|device| !is_octopus(device))
            .ok_or_else(|| "this machine has no microphone".to_string()),
    }
}

fn open_input(audio: broadcast::Sender<Chunk>) -> Result<cpal::Stream, String> {
    let device = microphone()?;
    let supported = device.default_input_config().map_err(|error| format!("read the microphone's format: {error}"))?;
    let channels = usize::from(supported.channels());
    let mut resampler = Resampler::new(supported.sample_rate());
    let failed = |error: cpal::Error| eprintln!("octopus app: the microphone failed: {error}");
    let mut send = move |frames: &[f32]| {
        let _no_listener = audio.send(Chunk::from(resampler.convert(frames, channels)));
    };
    let config = supported.config();
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            device.build_input_stream::<f32, _, _>(config, move |frames, _| send(frames), failed, None)
        }
        cpal::SampleFormat::I16 => device.build_input_stream::<i16, _, _>(
            config,
            move |frames, _| {
                send(&frames.iter().map(|sample| f32::from(*sample) / f32::from(i16::MAX)).collect::<Vec<_>>())
            },
            failed,
            None,
        ),
        format => return Err(format!("the microphone gives {format} samples, which octopus does not read")),
    }
    .map_err(|error| format!("open the microphone: {error}"))?;
    stream.play().map_err(|error| format!("start the microphone: {error}"))?;
    Ok(stream)
}

/// A capture that failed to start is tried again once no terminal listens.
#[derive(Default)]
enum Capturing {
    #[default]
    Idle,
    Running(Capture),
    Failed,
}

#[derive(Default)]
pub struct Microphone {
    capture: Mutex<Capturing>,
    sending: Mutex<BTreeMap<Listener, tauri::async_runtime::JoinHandle<()>>>,
}

impl Microphone {
    /// Sends the microphone to exactly `listeners`, each over its server's `connection`; the capture runs while any
    /// listener does.
    pub fn send_to(&self, listeners: BTreeMap<Listener, Arc<Connection>>) {
        let mut sending = lock(&self.sending);
        sending.retain(|listener, task| {
            let keep = listeners.contains_key(listener) && !task.inner().is_finished();
            if !keep {
                task.abort();
            }
            keep
        });
        let started: BTreeSet<&Listener> = sending.keys().collect();
        let fresh: Vec<(Listener, Arc<Connection>)> =
            listeners.into_iter().filter(|(listener, _)| !started.contains(listener)).collect();
        let mut capture = lock(&self.capture);
        if sending.is_empty() && fresh.is_empty() {
            *capture = Capturing::Idle;
            return;
        }
        if !fresh.is_empty() && matches!(*capture, Capturing::Idle) {
            *capture = Capture::start().map_or_else(
                |error| {
                    eprintln!("octopus app: {error}");
                    Capturing::Failed
                },
                Capturing::Running,
            );
        }
        let Capturing::Running(running) = &*capture else { return };
        for (listener, connection) in fresh {
            let pty = listener.1.clone();
            sending.insert(listener, tauri::async_runtime::spawn(send(connection, pty, running.audio.subscribe())));
        }
    }
}

/// Ends when the server closes the stream, as it does once the lock leaves this client.
async fn send(connection: Arc<Connection>, pty: PtyId, mut audio: broadcast::Receiver<Chunk>) {
    let mut stream = match connection.open_data_stream(&Stream::Mic(pty.clone())).await {
        Ok(stream) => stream,
        Err(error) => return eprintln!("octopus app: send the microphone to {pty}: {error}"),
    };
    loop {
        match audio.recv().await {
            Ok(chunk) => {
                if stream.write_all(&chunk).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
    let _server_gone = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stereo_44k1_capture_becomes_48k_mono_at_its_level() {
        let mut resampler = Resampler::new(44_100);
        let second: Vec<f32> = (0..44_100).flat_map(|_| [0.5, 0.5]).collect();
        let bytes: usize = second.chunks(441 * 2).map(|chunk| resampler.convert(chunk, 2).len()).sum();
        assert!((bytes as i64 - 96_000).abs() <= 2, "{bytes} bytes for one second");
        let tail = resampler.convert(&[0.5, 0.5], 2);
        let sample = i16::from_le_bytes([tail[0], tail[1]]);
        assert_eq!(sample, (0.5 * f32::from(i16::MAX)) as i16);
    }
}
