//! [`DeviceLink`] over the real UDP protocol. Sessions are opened on demand
//! and reused only for [`SESSION_IDLE`]; there is no background keep-alive.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use super::controller::LightChange;
use super::state::DeviceInfo;
use super::worker::DeviceLink;
use crate::discovery::{self, DiscoveryOptions};
use crate::protocol::device::Chameleon2;
use crate::protocol::packets::MAX_LEDS;
use crate::protocol::session::SessionConfig;
use crate::protocol::transport::UdpTransport;
use crate::protocol::{self, ProtocolError};

/// A session unused for this long is assumed expired on the device
/// (it announces a 10 s keep-alive interval) and is replaced.
pub const SESSION_IDLE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct UdpLink {
    current: Option<(Ipv4Addr, Chameleon2<UdpTransport>, Instant)>,
}

impl UdpLink {
    pub fn new() -> Self {
        Self::default()
    }

    fn connect(ip: Ipv4Addr, cfg: SessionConfig) -> protocol::Result<Chameleon2<UdpTransport>> {
        // LED count is irrelevant for static commands.
        Chameleon2::connect(ip, MAX_LEDS, cfg)
    }

    fn device(&mut self, ip: Ipv4Addr) -> protocol::Result<&mut Chameleon2<UdpTransport>> {
        let reusable = matches!(&self.current, Some((cur, dev, used)) if *cur == ip && dev.session().is_open() && used.elapsed() < SESSION_IDLE);
        if !reusable {
            self.current = None;
            let dev = Self::connect(ip, SessionConfig::default())?;
            self.current = Some((ip, dev, Instant::now()));
        }
        let (_, dev, used) = self.current.as_mut().expect("session just ensured");
        *used = Instant::now();
        Ok(dev)
    }
}

fn send(dev: &mut Chameleon2<UdpTransport>, change: LightChange) -> protocol::Result<()> {
    match change.power {
        Some(false) => return dev.set_power(false),
        Some(true) => dev.set_power(true)?,
        None => {}
    }
    if let Some(c) = change.color {
        dev.set_color(c)?;
    }
    if let Some(b) = change.brightness {
        dev.set_brightness(b)?;
    }
    Ok(())
}

fn text(e: ProtocolError) -> String {
    e.to_string()
}

impl DeviceLink for UdpLink {
    fn discover(&mut self) -> Result<Vec<DeviceInfo>, String> {
        let found = discovery::discover(&DiscoveryOptions::default()).map_err(|e| e.to_string())?;
        Ok(found.iter().filter(|d| d.is_chameleon2() && d.supports_rgbic_udp()).map(DeviceInfo::from).collect())
    }

    fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String> {
        self.current = None;
        let start = Instant::now();
        let dev = Self::connect(ip, SessionConfig::default()).map_err(text)?;
        let took = start.elapsed();
        self.current = Some((ip, dev, Instant::now()));
        Ok(took)
    }

    fn apply(&mut self, ip: Ipv4Addr, change: LightChange) -> Result<(), String> {
        let first = self.device(ip).and_then(|dev| send(dev, change));
        if first.is_ok() {
            return Ok(());
        }
        // One retry on a fresh session before reporting the strip unreachable.
        self.current = None;
        self.device(ip).and_then(|dev| send(dev, change)).map_err(text)
    }

    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String> {
        let quick = SessionConfig {
            reply_timeout: Duration::from_millis(400),
            handshake_attempts: 1,
            ..SessionConfig::default()
        };
        let reusable = matches!(&self.current, Some((cur, dev, used)) if *cur == ip && dev.session().is_open() && used.elapsed() < SESSION_IDLE);
        if !reusable {
            self.current = Some((ip, Self::connect(ip, quick).map_err(text)?, Instant::now()));
        }
        let (_, dev, _) = self.current.as_mut().expect("session just ensured");
        dev.set_power(false).map_err(text)
    }

    fn session_open(&self) -> bool {
        matches!(&self.current, Some((_, dev, used)) if dev.session().is_open() && used.elapsed() < SESSION_IDLE)
    }
}
