//! Colour maths: HDR (scRGB) to SDR, half floats, LED adjustments.

use crate::config::AmbilightConfig;
use crate::protocol::Rgb;

/// RGB, normally sRGB-encoded 0..1.
pub type Color = [f32; 3];

/// Brightness of SDR white in scRGB units (1.0 = 80 nits).
const SCRGB_NITS: f32 = 80.0;

pub fn srgb_encode(linear: f32) -> f32 {
    if linear <= 0.003_130_8 { 12.92 * linear } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 }
}

/// Linear scRGB → sRGB: SDR white maps to 1.0, out-of-gamut negatives and
/// non-finite values to 0, and anything brighter than SDR white is scaled
/// down by its largest channel so the hue survives.
pub fn hdr_to_srgb(c: Color, sdr_white_nits: f32) -> Color {
    let white = if sdr_white_nits > 0.0 { sdr_white_nits } else { SCRGB_NITS };
    let scale = SCRGB_NITS / white;
    let mut v = c.map(|x| if x.is_finite() { (x * scale).max(0.0) } else { 0.0 });
    let m = v[0].max(v[1]).max(v[2]);
    if m > 1.0 {
        v = v.map(|x| x / m);
    }
    v.map(srgb_encode)
}

/// IEEE 754 half precision → `f32` (DXGI FP16 frames).
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1F);
    let frac = f32::from(h & 0x3FF);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 if frac == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// Rec. 709 luma.
pub fn luma(c: Color) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn to_rgb(c: Color) -> Rgb {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb::new(q(c[0]), q(c[1]), q(c[2]))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjust {
    pub saturation: f32,
    /// 0..1
    pub brightness: f32,
    /// 0..1; colours whose largest channel is below it become black.
    pub black_threshold: f32,
}

impl Adjust {
    pub fn from_config(a: &AmbilightConfig) -> Self {
        Self {
            saturation: a.saturation as f32,
            brightness: f32::from(a.brightness) / 100.0,
            black_threshold: f32::from(a.black_threshold) / 100.0,
        }
    }
}

/// Threshold, then saturation around the luma, then brightness.
pub fn adjust(c: Color, a: &Adjust) -> Color {
    if c[0].max(c[1]).max(c[2]) < a.black_threshold {
        return [0.0; 3];
    }
    let l = luma(c);
    c.map(|x| (l + (x - l) * a.saturation).clamp(0.0, 1.0) * a.brightness)
}
