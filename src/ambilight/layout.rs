//! Strip layout: segments of LEDs laid along lines on the screen (front view).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::config::Segment;
use crate::protocol::Rgb;

pub const MAX_SEGMENTS: usize = 16;
/// Distance of the perimeter template from the screen edge.
pub const EDGE_INSET: f64 = 0.02;
/// LEDs marked on each side of the one being looked for.
const NEIGHBOURS: usize = 10;
const BEFORE: Rgb = Rgb::new(48, 0, 0);
const AFTER: Rgb = Rgb::new(0, 48, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentError {
    /// An index is not below the LED count.
    OutOfRange { led_count: usize },
    /// A coordinate is outside 0..=1 or not finite.
    BadPoint,
    /// Shares LEDs with the segment at this list position.
    Overlap(usize),
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { led_count } => write!(f, "LED index outside 0..{led_count}"),
            Self::BadPoint => f.write_str("point outside the screen"),
            Self::Overlap(i) => write!(f, "shares LEDs with segment {}", i + 1),
        }
    }
}

/// What the layout editor lights on the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "index", rename_all = "snake_case")]
pub enum HighlightTarget {
    Led(usize),
    Segment(usize),
}

impl Segment {
    pub fn lo(&self) -> usize {
        self.from.min(self.to)
    }

    pub fn hi(&self) -> usize {
        self.from.max(self.to)
    }

    /// Number of LEDs (never zero).
    pub fn count(&self) -> usize {
        self.hi() - self.lo() + 1
    }

    /// LED indices from `from` to `to`, in segment order.
    pub fn leds(&self) -> Vec<usize> {
        if self.from <= self.to { (self.from..=self.to).collect() } else { (self.to..=self.from).rev().collect() }
    }

    pub fn overlaps(&self, other: &Segment) -> bool {
        self.lo() <= other.hi() && other.lo() <= self.hi()
    }

    /// Screen point of the `k`-th LED of the segment (0 = `from`).
    pub fn point(&self, k: usize) -> [f64; 2] {
        let t = (k as f64 + 0.5) / self.count() as f64;
        [self.start[0] + (self.end[0] - self.start[0]) * t, self.start[1] + (self.end[1] - self.start[1]) * t]
    }

    /// Same line, LEDs in the opposite order.
    pub fn reversed(&self) -> Segment {
        Segment { from: self.to, to: self.from, start: self.start, end: self.end }
    }
}

pub fn check(seg: &Segment, led_count: usize) -> Result<(), SegmentError> {
    if seg.hi() >= led_count {
        return Err(SegmentError::OutOfRange { led_count });
    }
    let inside = |p: [f64; 2]| p.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c));
    if !inside(seg.start) || !inside(seg.end) {
        return Err(SegmentError::BadPoint);
    }
    Ok(())
}

/// The problem of each segment, in list order; an overlap names the first
/// earlier segment it collides with.
pub fn errors(segments: &[Segment], led_count: usize) -> Vec<Option<SegmentError>> {
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            check(s, led_count)
                .err()
                .or_else(|| segments[..i].iter().position(|e| e.overlaps(s)).map(SegmentError::Overlap))
        })
        .collect()
}

/// Keeps valid segments that do not overlap an already kept one, at most
/// [`MAX_SEGMENTS`]; warns about everything dropped.
pub fn sanitize(segments: Vec<Segment>, led_count: usize) -> (Vec<Segment>, Vec<String>) {
    let mut kept: Vec<Segment> = Vec::new();
    let mut warnings = Vec::new();
    for (i, s) in segments.into_iter().enumerate() {
        let problem =
            check(&s, led_count).err().or_else(|| kept.iter().position(|k| k.overlaps(&s)).map(SegmentError::Overlap));
        match problem {
            Some(e) => warnings.push(format!("ambilight.segments[{i}]: {e}, skipped")),
            None if kept.len() == MAX_SEGMENTS => {
                warnings.push(format!("ambilight.segments: only the first {MAX_SEGMENTS} segments are kept"));
                break;
            }
            None => kept.push(s),
        }
    }
    (kept, warnings)
}

/// Screen point of every LED; `None` for LEDs outside all segments.
pub fn led_points(segments: &[Segment], led_count: usize) -> Vec<Option<[f64; 2]>> {
    let mut out = vec![None; led_count];
    for s in segments {
        for (k, led) in s.leds().into_iter().enumerate() {
            if let Some(slot) = out.get_mut(led) {
                *slot = Some(s.point(k));
            }
        }
    }
    out
}

/// Screen point each LED takes its colour from: its place on the segment,
/// moved straight to the screen edge the segment faces. Horizontal runs go to
/// the top or bottom edge, vertical ones to the left or right edge, whichever
/// is nearer, so a loop glued inside the back of the monitor still shows the
/// edge colours while the editor draws it where it really lies.
pub fn sample_points(segments: &[Segment], led_count: usize) -> Vec<Option<[f64; 2]>> {
    let mut out = vec![None; led_count];
    for s in segments {
        for (k, led) in s.leds().into_iter().enumerate() {
            if let Some(slot) = out.get_mut(led) {
                *slot = Some(s.to_edge(s.point(k)));
            }
        }
    }
    out
}

impl Segment {
    /// Moves `p` to the edge this segment faces (see [`sample_points`]).
    fn to_edge(&self, p: [f64; 2]) -> [f64; 2] {
        let (dx, dy) = ((self.end[0] - self.start[0]).abs(), (self.end[1] - self.start[1]).abs());
        let mid = [(self.start[0] + self.end[0]) / 2.0, (self.start[1] + self.end[1]) / 2.0];
        // A point or an exact diagonal faces whichever edge is nearer.
        let horizontal = if dx == dy { (mid[1] - 0.5).abs() >= (mid[0] - 0.5).abs() } else { dx > dy };
        let side = |v: f64| if v < 0.5 { 0.0 } else { 1.0 };
        if horizontal { [p[0], side(mid[1])] } else { [side(mid[0]), p[1]] }
    }
}

/// Four sides from LED 0, clockwise seen from the front: left (bottom → top),
/// top (left → right), right (top → bottom), bottom (right → left).
pub fn perimeter(counts: [usize; 4]) -> Vec<Segment> {
    let (a, b) = (EDGE_INSET, 1.0 - EDGE_INSET);
    let lines = [([a, b], [a, a]), ([a, a], [b, a]), ([b, a], [b, b]), ([b, b], [a, b])];
    let mut next = 0;
    let mut out = Vec::new();
    for (n, (start, end)) in counts.into_iter().zip(lines) {
        if n == 0 {
            continue;
        }
        out.push(Segment { from: next, to: next + n - 1, start, end });
        next += n;
    }
    out
}

/// `[left, top, right, bottom]` for `led_count` LEDs around a 16:9 screen.
pub fn default_counts(led_count: usize) -> [usize; 4] {
    let top = led_count * 16 / 50;
    let rest = led_count - 2 * top;
    let left = rest / 2;
    [left, top, rest - left, top]
}

pub fn default_layout(led_count: usize) -> Vec<Segment> {
    perimeter(default_counts(led_count))
}

/// Fits segments to a smaller LED count: drops segments that start beyond it
/// and shortens the rest so the remaining LEDs keep their screen points.
pub fn trim(segments: &[Segment], led_count: usize) -> Vec<Segment> {
    segments
        .iter()
        .filter(|s| s.lo() < led_count)
        .map(|s| {
            if s.hi() < led_count {
                return s.clone();
            }
            let keep = (led_count - s.lo()) as f64 / s.count() as f64;
            let toward = |a: [f64; 2], b: [f64; 2]| [a[0] + (b[0] - a[0]) * keep, a[1] + (b[1] - a[1]) * keep];
            if s.from <= s.to {
                Segment { from: s.from, to: led_count - 1, start: s.start, end: toward(s.start, s.end) }
            } else {
                Segment { from: led_count - 1, to: s.to, start: toward(s.end, s.start), end: s.end }
            }
        })
        .collect()
}

/// Frame shown by the layout editor: one LED white with dim red before and
/// dim green after it, or one segment as a green (`from`) → blue (`to`) gradient.
pub fn highlight_frame(target: HighlightTarget, segments: &[Segment], led_count: usize) -> Vec<Rgb> {
    let mut frame = vec![Rgb::BLACK; led_count];
    match target {
        HighlightTarget::Led(i) => {
            for (j, c) in frame.iter_mut().enumerate() {
                *c = if j == i {
                    Rgb::new(255, 255, 255)
                } else if j < i && i - j <= NEIGHBOURS {
                    BEFORE
                } else if j > i && j - i <= NEIGHBOURS {
                    AFTER
                } else {
                    Rgb::BLACK
                };
            }
        }
        HighlightTarget::Segment(k) => {
            if let Some(s) = segments.get(k) {
                let leds = s.leds();
                let n = leds.len();
                for (pos, led) in leds.into_iter().enumerate() {
                    let t = if n > 1 { pos as f64 / (n - 1) as f64 } else { 0.0 };
                    if let Some(c) = frame.get_mut(led) {
                        *c = Rgb::new(0, (255.0 * (1.0 - t)).round() as u8, (255.0 * t).round() as u8);
                    }
                }
            }
        }
    }
    frame
}
