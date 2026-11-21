//! The show's sound: the song from its start instant, and the system's alert sound each time the show asks for it, on
//! the default output device, until the show ends.

use std::io::Cursor;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context;
use rodio::source::Buffered;
use rodio::{Decoder, DeviceSinkBuilder, Player, Source};

static SONG: &[u8] = include_bytes!("../../show/song.mp3");

/// The playing sound, which plays the alert sound on asking and stops once dropped.
pub(super) struct Sound(mpsc::Sender<()>);

impl Sound {
    pub(super) fn alert(&self) {
        let _ = self.0.send(());
    }
}

/// How long the song plays.
pub(super) fn length() -> Duration {
    song().ok().and_then(|song| song.total_duration()).expect("the song names its length")
}

/// Plays the song from `start`, and `alert`, the system's alert sound, at each asking, on a thread that holds the
/// output device until the sound is dropped.
pub(super) fn play(start: Instant, alert: Option<&'static Path>) -> Sound {
    let (sent, alerts) = mpsc::channel();
    thread::spawn(move || {
        if let Err(error) = run(start, alert, &alerts) {
            eprintln!("octopus app: play the show's sound: {error:#}");
        }
    });
    Sound(sent)
}

fn run(start: Instant, alert: Option<&Path>, alerts: &mpsc::Receiver<()>) -> anyhow::Result<()> {
    let mut sink = DeviceSinkBuilder::open_default_sink().context("open the default output device")?;
    sink.log_on_drop(false);
    let player = Player::connect_new(sink.mixer());
    player.pause();
    player.append(song().context("decode the song")?);
    let alert = alert.and_then(|path| {
        decoded(path)
            .inspect_err(|error| eprintln!("octopus app: the show plays without its alert sound: {error:#}"))
            .ok()
    });
    thread::sleep(start.saturating_duration_since(Instant::now()));
    player.play();
    for () in alerts {
        if let Some(alert) = &alert {
            sink.mixer().add(alert.clone());
        }
    }
    Ok(())
}

/// The song, decoded so it starts on its first sample: the MP3's own header names the encoder's delay, which gapless
/// decoding drops.
fn song() -> Result<Decoder<Cursor<&'static [u8]>>, rodio::decoder::DecoderError> {
    Decoder::builder().with_data(Cursor::new(SONG)).with_byte_len(SONG.len() as u64).with_gapless(true).build()
}

/// The sound file at `path`, decoded once so every alert replays it from memory.
fn decoded(path: &Path) -> anyhow::Result<Buffered<Decoder<Cursor<Vec<u8>>>>> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(Decoder::try_from(Cursor::new(bytes)).with_context(|| format!("decode {}", path.display()))?.buffered())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_song_names_its_length() {
        let length = super::length().as_secs_f64();
        assert!((length - 36.059).abs() < 0.01, "{length}");
    }
}
