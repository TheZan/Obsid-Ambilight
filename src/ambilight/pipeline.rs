//! One small frame in, LED colours out: letterbox → sampling → adjustment →
//! smoothing.

use std::time::Instant;

use super::color::{self, Adjust, Color};
use super::image::Image;
use super::layout;
use super::letterbox::{Bars, Letterbox};
use super::smooth::{self, Smoother};
use crate::config::{Config, Segment};
use crate::protocol::Rgb;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub led_count: usize,
    pub segments: Vec<Segment>,
    pub depth: f64,
    pub adjust: Adjust,
    pub smoothing: u8,
    pub letterbox: bool,
}

impl Settings {
    pub fn from_config(cfg: &Config) -> Self {
        let a = &cfg.ambilight;
        Self {
            led_count: cfg.device.led_count,
            segments: a.segments.clone(),
            depth: a.depth,
            adjust: Adjust::from_config(a),
            smoothing: a.smoothing,
            letterbox: a.letterbox,
        }
    }
}

#[derive(Debug, Default)]
pub struct Pipeline {
    letterbox: Letterbox,
    smoother: Smoother,
    size: (usize, usize),
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bars currently left out of sampling.
    pub fn bars(&self) -> Bars {
        self.letterbox.current()
    }

    pub fn process(&mut self, img: &Image, s: &Settings, now: Instant) -> Vec<Rgb> {
        let size = (img.width(), img.height());
        if self.size != size {
            self.size = size;
            self.letterbox.reset();
        }
        let area = if s.letterbox {
            self.letterbox.update(img, now).content(img)
        } else {
            self.letterbox.reset();
            img.full()
        };
        let targets: Vec<Color> = layout::sample_points(&s.segments, s.led_count)
            .into_iter()
            .map(|p| match p {
                Some(p) => color::adjust(img.mean(Image::sample_rect(area, p, s.depth)), &s.adjust),
                None => [0.0; 3],
            })
            .collect();
        self.smoother.step(&targets, smooth::tau(s.smoothing), now).iter().map(|c| color::to_rgb(*c)).collect()
    }
}
