//! Typed operations for the Yeelight Obsid RGBIC strip (model `Chameleon2`).

use std::net::Ipv4Addr;
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, info};

use super::packets::{Call, Effect, MAX_LEDS};
use super::session::{Session, SessionConfig};
use super::transport::{Transport, UdpTransport};
use super::{CONTROL_PORT, ProtocolError, Result, Rgb};

pub const MODEL: &str = "Chameleon2";

/// Ways to leave direct (streaming) mode. On fw 36 `Deactivate` is accepted and
/// the static state survives session expiry at the protocol level; the visual
/// result and Apple Home hand-over are still to be confirmed, so the other
/// strategies stay available for comparison. See `docs/protocol.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStrategy {
    /// Stop frames and send `set_rgb`/`set_bright` in the same session.
    SetRgb,
    /// `set_power off`, `set_power on`, then the static colour.
    PowerCycle,
    /// Stop all traffic (including keep-alives) so the device session expires,
    /// then open a new session and apply the static colour.
    ExpireSession { wait: Duration },
    /// `deactivate_fx_mode` (advertised by Chameleon2 in `support`), then the
    /// static colour in the same session.
    Deactivate,
}

pub struct Chameleon2<T: Transport> {
    session: Session<T>,
    led_count: usize,
    direct: bool,
}

impl Chameleon2<UdpTransport> {
    /// Connects to a strip and performs the session handshake.
    pub fn connect(ip: Ipv4Addr, led_count: usize, cfg: SessionConfig) -> Result<Self> {
        let transport = UdpTransport::connect(ip, CONTROL_PORT)?;
        let mut dev = Self::new(Session::new(transport, cfg), led_count)?;
        dev.session.open()?;
        Ok(dev)
    }
}

impl<T: Transport> Chameleon2<T> {
    pub fn new(session: Session<T>, led_count: usize) -> Result<Self> {
        if !(1..=MAX_LEDS).contains(&led_count) {
            return Err(ProtocolError::InvalidInput(format!("LED count {led_count} not in 1..={MAX_LEDS}")));
        }
        Ok(Self { session, led_count, direct: false })
    }

    pub fn led_count(&self) -> usize {
        self.led_count
    }

    /// Changes the frame length expected by `send_frame` (the LED count setting changed).
    pub fn set_led_count(&mut self, led_count: usize) -> Result<()> {
        if !(1..=MAX_LEDS).contains(&led_count) {
            return Err(ProtocolError::InvalidInput(format!("LED count {led_count} not in 1..={MAX_LEDS}")));
        }
        self.led_count = led_count;
        Ok(())
    }

    pub fn is_direct(&self) -> bool {
        self.direct
    }

    pub fn session(&self) -> &Session<T> {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session<T> {
        &mut self.session
    }

    /// Re-runs the handshake if the session was lost.
    pub fn ensure_session(&mut self) -> Result<()> {
        if !self.session.is_open() {
            self.direct = false;
            self.session.open()?;
        }
        Ok(())
    }

    /// Call periodically; sends a keep-alive when one is due.
    pub fn tick(&mut self, now: Instant) -> Result<()> {
        if self.session.keep_alive_due(now)
            && let Err(e) = self.session.keep_alive()
        {
            if !self.session.is_open() {
                self.direct = false;
            }
            return Err(e);
        }
        Ok(())
    }

    pub fn set_power(&mut self, on: bool) -> Result<()> {
        self.ensure_session()?;
        self.session.request(&Call::set_power(on, Effect::Sudden))?;
        if !on {
            self.direct = false;
        }
        Ok(())
    }

    pub fn set_brightness(&mut self, percent: u8) -> Result<()> {
        self.ensure_session()?;
        self.session.request(&Call::set_bright(percent, Effect::Sudden)?)?;
        Ok(())
    }

    pub fn set_color(&mut self, color: Rgb) -> Result<()> {
        self.ensure_session()?;
        self.session.request(&Call::set_rgb(color, Effect::Sudden)?)?;
        Ok(())
    }

    /// Power on, then colour and brightness.
    pub fn apply_static(&mut self, color: Rgb, brightness: u8) -> Result<()> {
        self.set_power(true)?;
        self.set_color(color)?;
        self.set_brightness(brightness)
    }

    pub fn enter_direct(&mut self) -> Result<()> {
        self.ensure_session()?;
        self.session.request(&Call::activate_fx_mode("direct"))?;
        self.direct = true;
        info!("direct mode active");
        Ok(())
    }

    /// `deactivate_fx_mode direct`; the static state is applied separately.
    pub fn leave_direct(&mut self) -> Result<()> {
        self.ensure_session()?;
        self.session.request(&Call::deactivate_fx_mode("direct"))?;
        self.direct = false;
        info!("direct mode left");
        Ok(())
    }

    /// Sends one frame (fire-and-forget). Requires direct mode and exactly
    /// `led_count` colours.
    pub fn send_frame(&mut self, colors: &[Rgb]) -> Result<()> {
        if !self.direct {
            return Err(ProtocolError::InvalidInput("not in direct mode".into()));
        }
        if colors.len() != self.led_count {
            return Err(ProtocolError::InvalidInput(format!(
                "frame has {} colours, strip is configured for {}",
                colors.len(),
                self.led_count
            )));
        }
        if let Err(e) = self.session.send(&Call::update_leds(colors)?) {
            if !self.session.is_open() {
                self.direct = false;
            }
            return Err(e);
        }
        Ok(())
    }

    /// Leaves direct mode with the given strategy and applies a static colour.
    pub fn exit_direct(&mut self, strategy: ExitStrategy, color: Rgb, brightness: u8) -> Result<()> {
        debug!(?strategy, "leaving direct mode");
        self.direct = false;
        match strategy {
            ExitStrategy::SetRgb => {
                self.set_color(color)?;
                self.set_brightness(brightness)
            }
            ExitStrategy::PowerCycle => {
                self.set_power(false)?;
                thread::sleep(Duration::from_millis(300));
                self.apply_static(color, brightness)
            }
            ExitStrategy::ExpireSession { wait } => {
                self.session.close();
                thread::sleep(wait);
                self.session.open()?;
                self.apply_static(color, brightness)
            }
            ExitStrategy::Deactivate => {
                self.ensure_session()?;
                self.session.request(&Call::deactivate_fx_mode("direct"))?;
                self.set_color(color)?;
                self.set_brightness(brightness)
            }
        }
    }
}
