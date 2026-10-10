//! Letterbox/pillarbox bars in the small frame, applied with hysteresis so a
//! dark scene does not make the sampled area jump around.

use std::time::{Duration, Instant};

use super::color::Color;
use super::image::{Image, Rect};

/// A row/column is black when every pixel's largest channel is below this (sRGB).
pub const BLACK_LEVEL: f32 = 0.06;
/// Bars thicker than this share of the side are not bars.
pub const MAX_BAR: f32 = 0.30;
/// How long new bars must persist before they are used.
pub const HOLD: Duration = Duration::from_secs(1);

/// Bar thickness in small-frame pixels, the same on opposite sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bars {
    /// Top and bottom.
    pub vertical: usize,
    /// Left and right.
    pub horizontal: usize,
}

impl Bars {
    pub fn content(&self, img: &Image) -> Rect {
        Rect {
            x0: self.horizontal,
            y0: self.vertical,
            x1: img.width() - self.horizontal,
            y1: img.height() - self.vertical,
        }
    }
}

fn dark(c: Color) -> bool {
    c[0].max(c[1]).max(c[2]) < BLACK_LEVEL
}

/// Bars of this frame alone; `None` when the whole frame is black.
pub fn detect(img: &Image) -> Option<Bars> {
    let (w, h) = (img.width(), img.height());
    let row_dark = |y: usize| (0..w).all(|x| dark(img.pixel(x, y)));
    let col_dark = |x: usize| (0..h).all(|y| dark(img.pixel(x, y)));
    let top = (0..h).take_while(|&y| row_dark(y)).count();
    if top == h {
        return None;
    }
    let bottom = (0..h).rev().take_while(|&y| row_dark(y)).count();
    let left = (0..w).take_while(|&x| col_dark(x)).count();
    let right = (0..w).rev().take_while(|&x| col_dark(x)).count();
    let limit = |n: usize, side: usize| if n as f32 > side as f32 * MAX_BAR { 0 } else { n };
    Some(Bars { vertical: limit(top.min(bottom), h), horizontal: limit(left.min(right), w) })
}

#[derive(Debug, Default)]
pub struct Letterbox {
    current: Bars,
    candidate: Option<(Bars, Instant)>,
}

impl Letterbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Bars {
        self.current
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, img: &Image, now: Instant) -> Bars {
        let Some(seen) = detect(img) else { return self.current };
        if seen == self.current {
            self.candidate = None;
            return self.current;
        }
        match self.candidate {
            Some((bars, since)) if bars == seen => {
                if now.duration_since(since) >= HOLD {
                    self.current = seen;
                    self.candidate = None;
                }
            }
            _ => self.candidate = Some((seen, now)),
        }
        self.current
    }
}
