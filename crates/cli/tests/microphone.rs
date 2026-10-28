#![cfg(target_os = "linux")]

mod common;

use std::time::Duration;

use common::{connect, create, eventually, read_until, start_with, task, workspace};
use octopus_client::CallError;
use octopus_proto::Error;
use octopus_proto::ops::{HarnessSet, TaskFocus, TermSend};
use octopus_proto::types::{Checkout, MIC_FORMAT, Stream};
use tokio::io::AsyncWriteExt;

/// 20 ms of a 440 Hz tone at half scale, as `MIC_FORMAT`, starting at sample `from`.
fn tone(from: usize) -> Vec<u8> {
    let rate = MIC_FORMAT.rate as usize;
    (from..from + rate / 50)
        .flat_map(|at| {
            let phase = 2.0 * std::f64::consts::PI * 440.0 * at as f64 / rate as f64;
            ((phase.sin() * f64::from(i16::MAX) / 2.0) as i16).to_le_bytes()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a PulseAudio or PipeWire sound server and arecord"]
async fn a_recording_program_hears_the_lock_holder_and_a_viewer_sends_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start_with(dir.path(), "server.sock", "host.sock", true).await;
    let (laptop, desk) = (connect(&running, "laptop").await, connect(&running, "desk").await);
    let workspace = workspace(&laptop, dir.path()).await;
    let recording = dir.path().join("heard.raw");
    let script = format!(
        "read go; arecord -q -f S16_LE -r {} -c 1 -t raw -d 2 {}; echo recorded; exec cat",
        MIC_FORMAT.rate,
        recording.display()
    );
    let argv = vec!["sh".into(), "-c".into(), script];
    let harness =
        HarnessSet { id: "rec".into(), argv, install: "true".into(), start: vec![], resume: vec![], hooks: false };
    laptop.call(&harness).await.expect("harness.set");
    let id = create(&laptop, &workspace, Checkout::Root, "rec").await;
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    let pty = task(&laptop, &id).await.panels[0].id.clone();
    let mut live = laptop.open_data_stream(&Stream::Pty(pty.clone())).await.expect("attach");
    let go = TermSend { task: id.clone(), panel: "agent".into(), text: "go".into(), enter: true, take_lock: true };
    laptop.call(&go).await.expect("start recording");
    eventually("the agent listening", async || task(&laptop, &id).await.panels[0].listening).await;

    match desk.open_data_stream(&Stream::Mic(pty.clone())).await {
        Err(CallError::Server(error)) => assert_eq!(error, Error::PtyLocked { holder: "laptop".into() }),
        Err(error) => panic!("a viewer's microphone failed otherwise: {error}"),
        Ok(_) => panic!("a viewer's microphone was taken"),
    }
    let mut mic = laptop.open_data_stream(&Stream::Mic(pty)).await.expect("send the microphone");
    let mut interval = tokio::time::interval(Duration::from_millis(20));
    for chunk in 0..150 {
        interval.tick().await;
        mic.write_all(&tone(chunk * MIC_FORMAT.rate as usize / 50)).await.expect("send audio");
    }
    read_until(&mut live, "recorded").await;
    eventually("the agent done listening", async || !task(&laptop, &id).await.panels[0].listening).await;

    let heard = std::fs::read(&recording).expect("read the recording");
    let peak = heard.as_chunks::<2>().0.iter().map(|sample| i16::from_le_bytes(*sample).unsigned_abs()).max();
    assert!(peak.is_some_and(|peak| peak > u16::MAX / 8), "the recording peaks at {peak:?} of {} bytes", heard.len());
}
