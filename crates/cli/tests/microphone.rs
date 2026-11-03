#![cfg(target_os = "linux")]

mod common;

use std::collections::BTreeMap;
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Running, SERVER, connect, create, eventually, read_until, start_with, task, workspace};
use octopus_client::{CallError, Connection, DataStream};
use octopus_proto::Error;
use octopus_proto::ops::{HarnessSet, TaskFocus, TermSend};
use octopus_proto::types::{CheckoutKind, Hooks, MIC_FORMAT, PtyId, Stream, TaskId};
use octopus_shell::microphone::{Access, Capture, Chunk, Microphone};
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

const TONE_MS: usize = 20;
const TONE_SAMPLES: usize = MIC_FORMAT.rate as usize * TONE_MS / 1000;
/// A recording of the tone peaks near half scale; one of silence peaks at 0.
const HEARD: u16 = u16::MAX / 8;

/// `TONE_MS` of a 440 Hz tone at half scale, as `MIC_FORMAT`, starting at sample `from`.
fn tone(from: usize) -> Vec<u8> {
    let rate = MIC_FORMAT.rate as f64;
    (from..from + TONE_SAMPLES)
        .flat_map(|at| {
            let phase = 2.0 * std::f64::consts::PI * 440.0 * at as f64 / rate;
            ((phase.sin() * f64::from(i16::MAX) / 2.0) as i16).to_le_bytes()
        })
        .collect()
}

/// A capture that plays the tone in place of a microphone, until it is dropped.
fn tone_capture() -> Result<Capture, String> {
    let (audio, _) = broadcast::channel(64);
    let (playing, stopped) = std::sync::mpsc::channel::<()>();
    let sender = audio.clone();
    std::thread::spawn(move || {
        let mut from = 0;
        while stopped.recv_timeout(Duration::from_millis(TONE_MS as u64)) == Err(RecvTimeoutError::Timeout) {
            let _nobody_listening = sender.send(Chunk::from(tone(from)));
            from += TONE_SAMPLES;
        }
    });
    Ok(Capture::new(audio, playing))
}

/// A task whose agent records 2 s from its virtual microphone, its terminal held by `holder` and listening.
struct Listening {
    dir: TempDir,
    running: Running,
    holder: Arc<Connection>,
    task: TaskId,
    pty: PtyId,
    live: DataStream,
}

impl Listening {
    async fn start(holder: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let running = start_with(dir.path(), "server.sock", "host.sock", true).await;
        let holder = Arc::new(connect(&running, holder).await);
        let workspace = workspace(&holder, dir.path()).await;
        let script = format!(
            "read go; arecord -q -f S16_LE -r {} -c 1 -t raw -d 2 {}; echo recorded; exec cat",
            MIC_FORMAT.rate,
            dir.path().join("heard.raw").display()
        );
        let argv = vec!["sh".into(), "-c".into(), script];
        let harness = HarnessSet {
            id: "rec".into(),
            argv,
            install: "true".into(),
            start: vec![],
            resume: vec![],
            hooks: Hooks::None,
        };
        holder.call(&harness).await.expect("harness.set");
        let id = create(&holder, &workspace, CheckoutKind::Root, "rec").await;
        holder.call(&TaskFocus { task: id.clone(), args: vec![] }).await.expect("focus");
        let pty = task(&holder, &id).await.panels[0].id.clone();
        let live = holder.open_data_stream(&Stream::Pty(pty.clone())).await.expect("attach");
        let go = TermSend { task: id.clone(), panel: "agent".into(), text: "go".into(), enter: true, take_lock: true };
        holder.call(&go).await.expect("start recording");
        eventually("the agent listening", async || task(&holder, &id).await.panels[0].listening).await;
        Self { dir, running, holder, task: id, pty, live }
    }

    /// The recording's loudest sample, once the agent is done listening.
    async fn peak(mut self) -> u16 {
        read_until(&mut self.live, "recorded").await;
        let (holder, id) = (&self.holder, &self.task);
        eventually("the agent done listening", async || !task(holder, id).await.panels[0].listening).await;
        let heard = std::fs::read(self.dir.path().join("heard.raw")).expect("read the recording");
        heard.as_chunks::<2>().0.iter().map(|sample| i16::from_le_bytes(*sample).unsigned_abs()).max().unwrap_or(0)
    }

    /// The client's microphone, as a phone or the desktop app runs it, sending to this terminal alone.
    fn send(&self, microphone: &Arc<Microphone>) {
        microphone.send_to(BTreeMap::from([((SERVER.to_string(), self.pty.clone()), Arc::clone(&self.holder))]));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a PulseAudio or PipeWire sound server and arecord"]
async fn a_recording_program_hears_the_lock_holder_and_a_viewer_sends_nothing() {
    let listening = Listening::start("laptop").await;
    let desk = connect(&listening.running, "desk").await;
    match desk.open_data_stream(&Stream::Mic(listening.pty.clone())).await {
        Err(CallError::Server(error)) => assert_eq!(error, Error::PtyLocked { holder: "laptop".into() }),
        Err(error) => panic!("a viewer's microphone failed otherwise: {error}"),
        Ok(_) => panic!("a viewer's microphone was taken"),
    }
    let mut mic = listening.holder.open_data_stream(&Stream::Mic(listening.pty.clone())).await.expect("send");
    let mut interval = tokio::time::interval(Duration::from_millis(TONE_MS as u64));
    for chunk in 0..150 {
        interval.tick().await;
        mic.write_all(&tone(chunk * TONE_SAMPLES)).await.expect("send audio");
    }
    let peak = listening.peak().await;
    assert!(peak > HEARD, "the recording peaks at {peak}");
}

/// The capture path a client app runs, from asking the OS through the stream to the server, carries its audio into
/// the terminal, and a denial sends nothing and says so.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a PulseAudio or PipeWire sound server and arecord"]
async fn the_client_microphone_carries_its_capture_into_the_terminal_once_granted() {
    let denials = Arc::new(Mutex::new(Vec::new()));
    let heard_denials = Arc::clone(&denials);
    let granted = Microphone::new(
        Box::new(|| Ok(Access::Granted)),
        Box::new(tone_capture),
        Box::new(move |denied| heard_denials.lock().expect("denials").push(denied)),
    );
    let listening = Listening::start("phone").await;
    listening.send(&granted);
    let peak = listening.peak().await;
    assert!(peak > HEARD, "a granted capture's recording peaks at {peak}");

    let recorded_denials = Arc::new(Mutex::new(Vec::new()));
    let heard = Arc::clone(&recorded_denials);
    let denied = Microphone::new(
        Box::new(|| Ok(Access::Denied)),
        Box::new(tone_capture),
        Box::new(move |is| heard.lock().expect("denials").push(is)),
    );
    let listening = Listening::start("phone").await;
    listening.send(&denied);
    let peak = listening.peak().await;
    assert_eq!(peak, 0, "a denied capture sends nothing, so the terminal hears silence");
    assert_eq!(*recorded_denials.lock().expect("denials"), [true]);
    assert!(denials.lock().expect("denials").is_empty(), "a granted capture is never denied");
}
