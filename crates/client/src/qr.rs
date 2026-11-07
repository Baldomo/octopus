//! An invite or a link as a QR code, for a phone to scan.

use std::fmt::Display;

use qrcode::QrCode;
use qrcode::render::{svg, unicode};

fn code(url: &impl Display) -> anyhow::Result<QrCode> {
    QrCode::new(url.to_string()).map_err(|error| anyhow::anyhow!("{url} does not fit a QR code: {error}"))
}

/// For a dark terminal: two modules per character, a block for each light one.
pub fn text(url: &impl Display) -> anyhow::Result<String> {
    Ok(code(url)?
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build())
}

/// For a page: an SVG that scales to its box.
pub fn svg(url: &impl Display) -> anyhow::Result<String> {
    Ok(code(url)?.render::<svg::Color<'_>>().quiet_zone(true).build())
}
