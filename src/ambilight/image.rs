//! Small frame of sRGB-encoded colours (0..1) and averages over areas of it.

use super::color::Color;

/// Pixel rectangle with exclusive right/bottom edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Rect {
    pub fn width(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }

    pub fn height(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    width: usize,
    height: usize,
    pixels: Vec<Color>,
}

impl Image {
    /// Panics if a side is zero or `pixels` does not hold `width * height` colours.
    pub fn new(width: usize, height: usize, pixels: Vec<Color>) -> Self {
        assert!(width > 0 && height > 0, "empty image");
        assert_eq!(pixels.len(), width * height, "pixel count");
        Self { width, height, pixels }
    }

    pub fn filled(width: usize, height: usize, c: Color) -> Self {
        Self::new(width, height, vec![c; width * height])
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn full(&self) -> Rect {
        Rect { x0: 0, y0: 0, x1: self.width, y1: self.height }
    }

    pub fn pixel(&self, x: usize, y: usize) -> Color {
        self.pixels[y * self.width + x]
    }

    pub fn set(&mut self, x: usize, y: usize, c: Color) {
        self.pixels[y * self.width + x] = c;
    }

    /// Mean colour of `r` clipped to the image; black for an empty area.
    pub fn mean(&self, r: Rect) -> Color {
        let (x1, y1) = (r.x1.min(self.width), r.y1.min(self.height));
        let mut sum = [0.0f32; 3];
        let mut n = 0u32;
        for y in r.y0..y1 {
            for x in r.x0..x1 {
                for (acc, v) in sum.iter_mut().zip(self.pixel(x, y)) {
                    *acc += v;
                }
                n += 1;
            }
        }
        if n == 0 { [0.0; 3] } else { sum.map(|s| s / n as f32) }
    }

    /// Square of side `depth × shorter side of area` (at least 1 px) centred
    /// on `p` (fractions of `area`) and moved inside `area`.
    pub fn sample_rect(area: Rect, p: [f64; 2], depth: f64) -> Rect {
        let (w, h) = (area.width(), area.height());
        if w == 0 || h == 0 {
            return area;
        }
        let short = w.min(h);
        let side = ((depth * short as f64).round() as usize).clamp(1, short);
        let place = |origin: usize, len: usize, frac: f64| {
            let centre = origin as f64 + frac.clamp(0.0, 1.0) * len as f64;
            let start = (centre - side as f64 / 2.0).round().max(origin as f64) as usize;
            start.min(origin + len - side)
        };
        let (x0, y0) = (place(area.x0, w, p[0]), place(area.y0, h, p[1]));
        Rect { x0, y0, x1: x0 + side, y1: y0 + side }
    }
}
