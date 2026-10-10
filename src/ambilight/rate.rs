//! Events per second over a sliding window (capture and send FPS).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct RateMeter {
    ticks: VecDeque<Instant>,
}

impl RateMeter {
    pub const WINDOW: Duration = Duration::from_secs(5);

    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self, now: Instant) {
        self.ticks.push_back(now);
        while self.ticks.front().is_some_and(|t| now.duration_since(*t) > Self::WINDOW) {
            self.ticks.pop_front();
        }
    }

    /// Rate of the ticks within the window before `now`, rounded to 0.1.
    pub fn rate(&self, now: Instant) -> f32 {
        let recent: Vec<Instant> =
            self.ticks.iter().copied().filter(|t| now.saturating_duration_since(*t) <= Self::WINDOW).collect();
        match (recent.first(), recent.last()) {
            (Some(first), Some(last)) if recent.len() >= 2 && last > first => {
                let r = (recent.len() - 1) as f32 / last.duration_since(*first).as_secs_f32();
                (r * 10.0).round() / 10.0
            }
            _ => 0.0,
        }
    }
}
