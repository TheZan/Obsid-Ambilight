//! Exponential smoothing of LED colours, skipped on scene cuts.

use std::time::{Duration, Instant};

use super::color::{Color, luma};

/// A change of the mean LED luma larger than this is a scene cut.
pub const SCENE_CUT: f32 = 0.25;
/// Time constant at smoothing 100 %.
pub const MAX_TAU: Duration = Duration::from_millis(500);

/// Time constant for the "Smoothing" setting (0..=100).
pub fn tau(smoothing: u8) -> Duration {
    MAX_TAU.mul_f64(f64::from(smoothing.min(100)) / 100.0)
}

#[derive(Debug, Default)]
pub struct Smoother {
    state: Vec<Color>,
    last: Option<Instant>,
}

impl Smoother {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Moves the state toward `target` and returns it.
    pub fn step(&mut self, target: &[Color], tau: Duration, now: Instant) -> &[Color] {
        let jump = match self.last {
            Some(last) if self.state.len() == target.len() && !tau.is_zero() => {
                if (mean_luma(target) - mean_luma(&self.state)).abs() > SCENE_CUT {
                    None
                } else {
                    let dt = now.saturating_duration_since(last).as_secs_f32();
                    Some(1.0 - (-dt / tau.as_secs_f32()).exp())
                }
            }
            _ => None,
        };
        match jump {
            Some(a) => {
                for (s, t) in self.state.iter_mut().zip(target) {
                    for (sv, tv) in s.iter_mut().zip(t) {
                        *sv += (tv - *sv) * a;
                    }
                }
            }
            None => self.state = target.to_vec(),
        }
        self.last = Some(now);
        &self.state
    }
}

fn mean_luma(colors: &[Color]) -> f32 {
    if colors.is_empty() { 0.0 } else { colors.iter().map(|c| luma(*c)).sum::<f32>() / colors.len() as f32 }
}
