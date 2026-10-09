//! Locating a saved device by its stable id and pacing reconnect attempts.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::discovery::DiscoveredDevice;

/// How a saved device was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The saved IP answered a unicast search with the expected id.
    SavedIp(DiscoveredDevice),
    /// Found by LAN discovery; the IP may have changed.
    Discovered(DiscoveredDevice),
}

impl Resolution {
    pub fn device(&self) -> &DiscoveredDevice {
        match self {
            Self::SavedIp(d) | Self::Discovered(d) => d,
        }
    }

    pub fn ip_changed(&self, last_ip: Option<Ipv4Addr>) -> bool {
        last_ip != Some(self.device().ip)
    }
}

/// Resolves the device `id`: first the saved IP (identity-checked via `probe`),
/// then a LAN-wide `discover`. Never trusts an IP alone, since DHCP may have
/// given it to another device.
pub fn resolve(
    id: &str,
    last_ip: Option<Ipv4Addr>,
    mut probe: impl FnMut(Ipv4Addr) -> Option<DiscoveredDevice>,
    mut discover: impl FnMut() -> Vec<DiscoveredDevice>,
) -> Option<Resolution> {
    if let Some(ip) = last_ip
        && let Some(dev) = probe(ip).filter(|d| d.id == id)
    {
        return Some(Resolution::SavedIp(dev));
    }
    discover().into_iter().find(|d| d.id == id).map(Resolution::Discovered)
}

/// Exponential backoff with "equal jitter": each delay is uniform in
/// `[base*2^n / 2, base*2^n]`, capped at `max`, for at most `max_attempts`.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    max_attempts: Option<u32>,
    attempt: u32,
    rng: u64,
}

impl Backoff {
    pub fn new(base: Duration, max: Duration, max_attempts: Option<u32>) -> Self {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        Self::with_seed(base, max, max_attempts, seed)
    }

    pub fn with_seed(base: Duration, max: Duration, max_attempts: Option<u32>, seed: u64) -> Self {
        Self { base, max, max_attempts, attempt: 0, rng: seed | 1 }
    }

    pub fn attempt(&self) -> u32 {
        self.attempt
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Delay before the next attempt, or `None` once the attempts are exhausted.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.max_attempts.is_some_and(|m| self.attempt >= m) {
            return None;
        }
        let exp = self.base.saturating_mul(1u32 << self.attempt.min(16)).min(self.max);
        self.attempt += 1;
        let half = exp / 2;
        let jitter_ns = (half.as_nanos() as u64).checked_add(1).map_or(0, |span| self.next_rand() % span);
        Some(half + Duration::from_nanos(jitter_ns))
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift64: good enough for spreading retries, no dependency needed.
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
}
