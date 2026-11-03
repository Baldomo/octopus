//! This client's microphone, sent to every listening terminal whose lock it holds.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use octopus_client::Connection;
use octopus_proto::types::{MIC_DEVICE, MIC_FORMAT, PtyId, Stream};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

use crate::lock;

/// Whether the OS refuses this client its microphone, which the UI marks on a listening terminal it holds.
pub(crate) const MICROPHONE_DENIED_EVENT: &str = "microphone_denied";

/// Chunks of captured audio waiting for a slow stream; one that falls further behind skips ahead.
const BACKLOG: usize = 64;

/// A terminal this client sends its microphone to.
pub type Listener = (String, PtyId);

pub type Chunk = Arc<[u8]>;

/// Linear resampling of mono `f32` to `MIC_FORMAT`, carrying its position across calls.
pub(crate) struct Resampler {
    step: f64,
    position: f64,
    previous: f32,
}

impl Resampler {
    pub fn new(rate: u32) -> Self {
        Self { step: f64::from(rate) / f64::from(MIC_FORMAT.rate), position: 0.0, previous: 0.0 }
    }

    /// Takes `frames` of `channels` interleaved samples, mixes them to mono, and returns `MIC_FORMAT` bytes.
    pub(crate) fn convert(&mut self, frames: &[f32], channels: usize) -> Vec<u8> {
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

/// A source of `MIC_FORMAT` audio, which runs until it is dropped.
pub struct Capture {
    audio: broadcast::Sender<Chunk>,
    _running: Box<dyn Send>,
}

/// Opens the source a capture sends.
pub(crate) type OpenCapture = Box<dyn Fn() -> Result<Capture, String> + Send + Sync>;

impl Capture {
    /// A source whose audio comes on `audio` while `running` lives.
    pub fn new(audio: broadcast::Sender<Chunk>, running: impl Send + 'static) -> Self {
        Self { audio, _running: Box::new(running) }
    }

    /// The default input device, captured on a thread of its own, since a capture stream stays on the thread that
    /// made it.
    pub fn microphone() -> Result<Self, String> {
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
        Ok(Self::new(audio, stop))
    }
}

/// Whether `device` is the Octopus Microphone, which a remote holder makes this machine's default input and which
/// carries a terminal's audio, not this user's.
fn is_octopus(device: &cpal::Device) -> bool {
    device.description().is_ok_and(|description| description.name() == MIC_DEVICE.name)
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

/// Whether the OS lets this client record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Granted,
    Denied,
}

/// Asks the OS whether this client may record, which may ask the user and wait for the answer; an OS that fails to
/// answer says why.
pub(crate) type AskAccess = Box<dyn Fn() -> Result<Access, String> + Send + Sync>;

type Listeners = BTreeMap<Listener, Arc<Connection>>;

/// A capture that failed to start, or that the OS refused, is tried again once no terminal listens.
#[derive(Default)]
enum Capturing {
    #[default]
    Idle,
    /// The OS is asking the user, off the thread that keeps the UI current.
    Asking,
    Running(Capture),
    Failed,
    Denied,
}

#[derive(Default)]
struct Sending {
    /// The listeners of the newest `send_to`, which an answer from the OS applies.
    wanted: Listeners,
    tasks: BTreeMap<Listener, tauri::async_runtime::JoinHandle<()>>,
}

/// Hears whether access is denied, each time that changes.
pub(crate) type DeniedChanged = Box<dyn Fn(bool) + Send + Sync>;

pub struct Microphone {
    ask: AskAccess,
    open: OpenCapture,
    denied_changed: DeniedChanged,
    capture: Mutex<Capturing>,
    sending: Mutex<Sending>,
}

impl Microphone {
    pub fn new(ask: AskAccess, open: OpenCapture, denied_changed: DeniedChanged) -> Arc<Self> {
        Arc::new(Self { ask, open, denied_changed, capture: Mutex::default(), sending: Mutex::default() })
    }

    /// Moves to `next`, telling `denied_changed` whenever access starts or stops being denied.
    fn set(&self, capture: &mut Capturing, next: Capturing) {
        let (was, is) = (matches!(capture, Capturing::Denied), matches!(next, Capturing::Denied));
        *capture = next;
        if was != is {
            (self.denied_changed)(is);
        }
    }

    /// Starts the capture the OS answered for, unless no terminal listens any more, then sends to whoever does now.
    fn answered(self: &Arc<Self>, answer: Result<Access, String>) {
        {
            let mut capture = lock(&self.capture);
            if !matches!(*capture, Capturing::Asking) {
                return;
            }
            let next = match answer {
                Ok(Access::Denied) => Capturing::Denied,
                Ok(Access::Granted) => (self.open)().map_or_else(
                    |error| {
                        eprintln!("octopus app: {error}");
                        Capturing::Failed
                    },
                    Capturing::Running,
                ),
                Err(error) => {
                    eprintln!("octopus app: ask for the microphone: {error}");
                    Capturing::Failed
                }
            };
            self.set(&mut capture, next);
        }
        let mut sending = lock(&self.sending);
        let wanted = sending.wanted.clone();
        self.apply(&mut sending, wanted);
    }

    /// Sends the microphone to exactly `listeners`, each over its server's `connection`; the capture runs while any
    /// listener does.
    pub fn send_to(self: &Arc<Self>, listeners: Listeners) {
        let mut sending = lock(&self.sending);
        sending.wanted.clone_from(&listeners);
        self.apply(&mut sending, listeners);
    }

    fn apply(self: &Arc<Self>, sending: &mut Sending, listeners: Listeners) {
        let sending = &mut sending.tasks;
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
            self.set(&mut capture, Capturing::Idle);
            return;
        }
        if !fresh.is_empty() && matches!(*capture, Capturing::Idle) {
            self.set(&mut capture, Capturing::Asking);
            let microphone = Arc::clone(self);
            tauri::async_runtime::spawn_blocking(move || microphone.answered((microphone.ask)()));
            return;
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
