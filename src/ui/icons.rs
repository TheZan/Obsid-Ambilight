//! Status icons drawn in code (no image assets): anti-aliased circles.

use crate::app::state::{Connection, Snapshot};
use crate::protocol::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    /// Static mode: a disc in the current colour.
    Lit(Rgb),
    /// Lighting switched off: a grey ring.
    Off,
    /// Looking for / connecting to the strip: an amber dot.
    Connecting,
    /// Strip unavailable or not chosen: a red dot.
    Offline,
}

const GREY: Rgb = Rgb::new(0x9A, 0xA0, 0xA6);
const AMBER: Rgb = Rgb::new(0xF2, 0xB3, 0x3D);
const RED: Rgb = Rgb::new(0xE5, 0x48, 0x4D);

pub fn kind_for(s: &Snapshot) -> IconKind {
    match s.connection {
        Connection::Ready if s.config.light.power => IconKind::Lit(s.config.light.color),
        Connection::Ready => IconKind::Off,
        Connection::Connecting => IconKind::Connecting,
        Connection::NoDevice if s.searching => IconKind::Connecting,
        Connection::NoDevice | Connection::Offline => IconKind::Offline,
    }
}

/// RGBA pixels (unpremultiplied), `size`×`size`, rows top to bottom.
pub fn render(kind: IconKind, size: u32) -> Vec<u8> {
    let s = size as f32;
    let centre = s / 2.0;
    let (color, outer, inner) = match kind {
        IconKind::Lit(c) => (c, s * 0.44, 0.0),
        IconKind::Off => (GREY, s * 0.42, s * 0.42 - (s * 0.12).max(1.5)),
        IconKind::Connecting => (AMBER, s * 0.30, 0.0),
        IconKind::Offline => (RED, s * 0.30, 0.0),
    };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - centre).powi(2) + (y as f32 + 0.5 - centre).powi(2)).sqrt();
            let cover_outer = (outer - d + 0.5).clamp(0.0, 1.0);
            let cover_inner = if inner > 0.0 { (inner - d + 0.5).clamp(0.0, 1.0) } else { 0.0 };
            let alpha = ((cover_outer - cover_inner).max(0.0) * 255.0).round() as u8;
            out.extend_from_slice(&[color.r, color.g, color.b, alpha]);
        }
    }
    out
}
