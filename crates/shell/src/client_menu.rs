//! The client menu's prefs: the appearance the window shows, the zoom it scales its UI by, and the flags its agents
//! start with.

use serde::{Deserialize, Serialize};
use tauri::Theme;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// The theme the window takes; none follows the OS.
    pub fn theme(self) -> Option<Theme> {
        match self {
            Self::System => None,
            Self::Light => Some(Theme::Light),
            Self::Dark => Some(Theme::Dark),
        }
    }
}

/// The zoom in percent, one of `ZOOMS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(try_from = "u16")]
pub struct Zoom(u16);

pub const ZOOMS: [u16; 6] = [80, 90, 100, 110, 125, 150];

impl TryFrom<u16> for Zoom {
    type Error = String;

    fn try_from(percent: u16) -> Result<Self, String> {
        ZOOMS.contains(&percent).then_some(Self(percent)).ok_or_else(|| format!("zoom {percent} is none of {ZOOMS:?}"))
    }
}
const ACTUAL_SIZE: Zoom = Zoom(100);

impl Default for Zoom {
    fn default() -> Self {
        ACTUAL_SIZE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ZoomStep {
    In,
    Out,
    Reset,
}

impl Zoom {
    /// The next zoom of `ZOOMS` in the step's direction, staying put at either end.
    pub fn step(self, step: ZoomStep) -> Self {
        let Self(now) = self;
        match step {
            ZoomStep::In => Self(ZOOMS.into_iter().find(|&zoom| zoom > now).unwrap_or(now)),
            ZoomStep::Out => Self(ZOOMS.into_iter().rev().find(|&zoom| zoom < now).unwrap_or(now)),
            ZoomStep::Reset => ACTUAL_SIZE,
        }
    }

    /// How many window points one CSS pixel of the UI takes.
    pub fn scale(self) -> f64 {
        f64::from(self.0) / 100.0
    }
}

/// A harness's agent flags as the user typed them, shell words that split into the args each focus sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(try_from = "String")]
pub struct AgentFlags(String);

impl TryFrom<String> for AgentFlags {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        match shell_words::split(&text) {
            Ok(_) => Ok(Self(text.trim().to_string())),
            Err(_) => Err(format!("the quoting of {text} does not close")),
        }
    }
}

impl AgentFlags {
    pub fn args(&self) -> Vec<String> {
        shell_words::split(&self.0).expect("agent flags hold only text that splits")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_flags_split_as_shell_words_and_refuse_quoting_that_does_not_close() {
        let flags = |text: &str| serde_json::from_value::<AgentFlags>(serde_json::json!(text));
        let typed = flags(" --allow-dangerously-skip-permissions --append-system-prompt 'be brief' ").expect("splits");
        assert_eq!(typed.args(), ["--allow-dangerously-skip-permissions", "--append-system-prompt", "be brief"]);
        let saved = serde_json::to_value(&typed).expect("serializes");
        assert_eq!(saved, "--allow-dangerously-skip-permissions --append-system-prompt 'be brief'");
        assert!(flags("--model 'opus").is_err());
        assert!(flags("  ").expect("splits").is_empty());
    }

    #[test]
    fn zoom_steps_through_the_zooms_and_stops_at_either_end() {
        assert!(serde_json::from_str::<Zoom>("0").is_err());
        assert_eq!(serde_json::from_str::<Zoom>("125").ok(), Some(Zoom(125)));
        let stepped = |from, step, times| (0..times).fold(Zoom(from), |zoom, _| zoom.step(step));
        assert_eq!(stepped(100, ZoomStep::In, 9), Zoom(150));
        assert_eq!(stepped(100, ZoomStep::Out, 9), Zoom(80));
        assert_eq!(stepped(110, ZoomStep::Out, 1), Zoom(100));
        assert_eq!(stepped(150, ZoomStep::Reset, 1), Zoom(100));
    }
}
