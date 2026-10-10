//! Status icons from the bundled PNG set (`assets/icons/tray`): the static
//! icon is recoloured to the strip colour, and the amber status dot is made
//! from the red one of the offline icon.

use std::io::Cursor;

use crate::app::state::{Connection, Snapshot};
use crate::config::Mode;
use crate::protocol::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    /// Static mode: the monitor frame in the current colour.
    Lit(Rgb),
    /// Lighting switched off: a grey monitor.
    Off,
    /// Looking for / connecting to the strip: a grey monitor with an amber dot.
    Connecting,
    /// Strip unavailable or not chosen: a grey monitor with a red dot.
    Offline,
    /// Ambilight: a gradient monitor; `warn` adds an amber dot (capture unavailable).
    Ambilight { warn: bool },
}

/// Sizes drawn by hand; other sizes are scaled from the next larger one.
const SIZES: [u32; 6] = [16, 20, 24, 32, 48, 64];

macro_rules! tray_set {
    ($name:literal) => {
        [
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/tray/", $name, "-16.png")).as_slice(),
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/tray/", $name, "-20.png")),
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/tray/", $name, "-24.png")),
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/tray/", $name, "-32.png")),
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/tray/", $name, "-48.png")),
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/tray/", $name, "-64.png")),
        ]
    };
}

const STATIC: [&[u8]; 6] = tray_set!("static");
const AMBILIGHT: [&[u8]; 6] = tray_set!("ambilight");
const OFF: [&[u8]; 6] = tray_set!("off");
const OFFLINE: [&[u8]; 6] = tray_set!("offline");
const APP: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/app-256.png"));

/// Colours of the artwork: the dark screen, the static frame, the dot and its halo.
const SCREEN: Rgb = Rgb::new(0x10, 0x16, 0x26);
const STATIC_FRAME: Rgb = Rgb::new(0xFF, 0xC5, 0x79);
const HALO: Rgb = Rgb::new(0x11, 0x16, 0x23);
const RED: Rgb = Rgb::new(0xFF, 0x65, 0x6B);
const AMBER: Rgb = Rgb::new(0xFF, 0xB1, 0x3B);
/// Status dot halo in the 128-unit artwork: centre and radius.
const DOT: (f32, f32, f32) = (107.0, 25.0, 12.0);

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
    let i = SIZES.iter().position(|&s| s >= size).unwrap_or(SIZES.len() - 1);
    let from = SIZES[i];
    let img = match kind {
        IconKind::Lit(c) => {
            let mut img = decode(STATIC[i]).0;
            for p in img.as_chunks_mut::<4>().0 {
                retint(p, SCREEN, STATIC_FRAME, c);
            }
            img
        }
        IconKind::Off => decode(OFF[i]).0,
        IconKind::Offline => decode(OFFLINE[i]).0,
        IconKind::Connecting => amber_dot(i),
        IconKind::Ambilight { warn: false } => decode(AMBILIGHT[i]).0,
        IconKind::Ambilight { warn: true } => {
            let mut img = decode(AMBILIGHT[i]).0;
            let dot = amber_dot(i);
            let k = from as f32 / 128.0;
            let (cx, cy, r) = (DOT.0 * k, DOT.1 * k, DOT.2 * k);
            for (n, (p, d)) in img.as_chunks_mut::<4>().0.iter_mut().zip(dot.as_chunks::<4>().0).enumerate() {
                let (x, y) = ((n as u32 % from) as f32 + 0.5, (n as u32 / from) as f32 + 0.5);
                if (x - cx).hypot(y - cy) < r {
                    p.copy_from_slice(d);
                }
            }
            img
        }
    };
    if from == size { img } else { resample(&img, from, size) }
}

/// The application icon (256×256 RGBA) for windows.
pub fn app_icon() -> (Vec<u8>, u32) {
    decode(APP)
}

/// The offline icon with its red dot turned amber.
fn amber_dot(i: usize) -> Vec<u8> {
    let mut img = decode(OFFLINE[i]).0;
    for p in img.as_chunks_mut::<4>().0 {
        if p[0] > p[1].saturating_add(60) && p[0] > p[2].saturating_add(60) {
            retint(p, HALO, RED, AMBER);
        }
    }
    img
}

/// Moves a pixel lying between `dark` and `light` to the same place between
/// `dark` and `to`; alpha is kept.
fn retint(p: &mut [u8], dark: Rgb, light: Rgb, to: Rgb) {
    let d = [dark.r, dark.g, dark.b].map(f32::from);
    let l = [light.r, light.g, light.b].map(f32::from);
    let t_ = [to.r, to.g, to.b].map(f32::from);
    let axis = [l[0] - d[0], l[1] - d[1], l[2] - d[2]];
    let len2: f32 = axis.iter().map(|a| a * a).sum();
    let t = ((0..3).map(|c| (f32::from(p[c]) - d[c]) * axis[c]).sum::<f32>() / len2).clamp(0.0, 1.0);
    for c in 0..3 {
        p[c] = (d[c] + t * (t_[c] - d[c])).round().clamp(0.0, 255.0) as u8;
    }
}

/// Decodes a bundled PNG to square RGBA.
fn decode(bytes: &[u8]) -> (Vec<u8>, u32) {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().expect("bundled PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("bundled PNG size")];
    let info = reader.next_frame(&mut buf).expect("bundled PNG");
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => {
            buf.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
        }
        png::ColorType::Grayscale => buf.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => unreachable!("expanded by normalize_to_color8"),
    };
    (rgba, info.width)
}

/// Area-average scaling of a square RGBA image, in premultiplied space.
fn resample(src: &[u8], from: u32, to: u32) -> Vec<u8> {
    let scale = from as f32 / to as f32;
    let mut out = Vec::with_capacity((to * to * 4) as usize);
    for y in 0..to {
        for x in 0..to {
            let (x0, x1) = (x as f32 * scale, (x + 1) as f32 * scale);
            let (y0, y1) = (y as f32 * scale, (y + 1) as f32 * scale);
            let mut acc = [0.0f32; 4];
            let mut area = 0.0;
            for sy in y0.floor() as u32..(y1.ceil() as u32).min(from) {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                for sx in x0.floor() as u32..(x1.ceil() as u32).min(from) {
                    let w = wy * (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let p = &src[((sy * from + sx) * 4) as usize..][..4];
                    let a = f32::from(p[3]) * w;
                    for c in 0..3 {
                        acc[c] += f32::from(p[c]) * a;
                    }
                    acc[3] += a;
                    area += w;
                }
            }
            let alpha = acc[3] / area.max(f32::EPSILON);
            let rgb = acc.map(|v| if acc[3] > 0.0 { v / acc[3] } else { 0.0 });
            out.extend_from_slice(&[rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, alpha.round() as u8]);
        }
    }
    out
}
