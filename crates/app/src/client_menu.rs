//! The client menu's prefs: the appearance the window shows and the zoom it scales its UI by.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zoom_outside_the_steps_is_refused() {
        assert!(serde_json::from_str::<Zoom>("0").is_err());
        assert_eq!(serde_json::from_str::<Zoom>("125").ok(), Some(Zoom(125)));
    }

    #[test]
    fn zoom_steps_through_the_zooms_and_stops_at_either_end() {
        let stepped = |from, step, times| (0..times).fold(Zoom(from), |zoom, _| zoom.step(step));
        assert_eq!(stepped(100, ZoomStep::In, 9), Zoom(150));
        assert_eq!(stepped(100, ZoomStep::Out, 9), Zoom(80));
        assert_eq!(stepped(110, ZoomStep::Out, 1), Zoom(100));
        assert_eq!(stepped(150, ZoomStep::Reset, 1), Zoom(100));
    }
}
