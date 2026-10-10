//! Status icons drawn in code (no image assets): anti-aliased circles.

use std::f32::consts::{FRAC_PI_2, PI};

use crate::app::state::{Connection, Snapshot};
use crate::config::Mode;
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
    /// Ambilight: a ring of four colours; `warn` adds an amber dot (capture unavailable).
    Ambilight { warn: bool },
}

const GREY: Rgb = Rgb::new(0x9A, 0xA0, 0xA6);
const AMBER: Rgb = Rgb::new(0xF2, 0xB3, 0x3D);
const RED: Rgb = Rgb::new(0xE5, 0x48, 0x4D);
const ARCS: [Rgb; 4] = [RED, Rgb::new(0x3D, 0xB2, 0x5A), Rgb::new(0x3D, 0x7E, 0xE5), AMBER];

pub fn kind_for(s: &Snapshot) -> IconKind {
    match s.connection {
        Connection::Ready if !s.dark.is_empty() => IconKind::Off,
        Connection::Ready if s.config.light.power && s.config.app.mode == Mode::Ambilight => {
            IconKind::Ambilight { warn: s.stream.capture.as_ref().is_some_and(|c| !c.active && c.error.is_some()) }
        }
        Connection::Ready if s.config.light.power => IconKind::Lit(s.config.light.color),
        Connection::Ready => IconKind::Off,
        Connection::Connecting => IconKind::Connecting,
        Connection::NoDevice if s.searching => IconKind::Connecting,
        Connection::NoDevice | Connection::Offline => IconKind::Offline,
    }
}

/// RGBA pixels (unpremultiplied), `size`×`size`, rows top to bottom.
pub fn render(kind: IconKind, size: u32) -> Vec<u8> {
    if let IconKind::Ambilight { warn } = kind {
        return ambilight(size, warn);
    }
    let s = size as f32;
    let centre = s / 2.0;
    let (color, outer, inner) = match kind {
        IconKind::Lit(c) => (c, s * 0.44, 0.0),
        IconKind::Off => (GREY, s * 0.42, s * 0.42 - (s * 0.12).max(1.5)),
        IconKind::Connecting => (AMBER, s * 0.30, 0.0),
        IconKind::Offline => (RED, s * 0.30, 0.0),
        IconKind::Ambilight { .. } => unreachable!("drawn by ambilight()"),
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

/// Ring split into four coloured quadrants, with an optional amber dot at the bottom right.
fn ambilight(size: u32, warn: bool) -> Vec<u8> {
    let s = size as f32;
    let c = s / 2.0;
    let (outer, inner) = (s * 0.44, s * 0.27);
    let (dot_c, dot_r) = (s * 0.78, s * 0.2);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (dx, dy) = (px - c, py - c);
            let d = (dx * dx + dy * dy).sqrt();
            let ring = ((outer - d + 0.5).clamp(0.0, 1.0) - (inner - d + 0.5).clamp(0.0, 1.0)).max(0.0);
            let quadrant = (((dy.atan2(dx) + PI) / FRAC_PI_2) as usize).min(3);
            let dot = if warn {
                (dot_r - ((px - dot_c).powi(2) + (py - dot_c).powi(2)).sqrt() + 0.5).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (color, alpha) = if dot > 0.0 { (AMBER, dot.max(ring)) } else { (ARCS[quadrant], ring) };
            out.extend_from_slice(&[color.r, color.g, color.b, (alpha * 255.0).round() as u8]);
        }
    }
    out
}
