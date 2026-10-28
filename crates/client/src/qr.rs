//! An invite as a QR code, for a phone to scan.

use octopus_proto::pairing::Invite;
use qrcode::QrCode;
use qrcode::render::{svg, unicode};

fn code(invite: &Invite) -> QrCode {
    QrCode::new(invite.to_string()).expect("an invite fits a QR code")
}

/// For a dark terminal: two modules per character, a block for each light one.
pub fn text(invite: &Invite) -> String {
    code(invite)
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build()
}

/// For a page: an SVG that scales to its box.
pub fn svg(invite: &Invite) -> String {
    code(invite).render::<svg::Color<'_>>().quiet_zone(true).build()
}
