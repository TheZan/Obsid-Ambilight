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
use crate::protocol::session::{Session, SessionConfig};
use crate::protocol::transport::UdpTransport;
use crate::protocol::{self, CONTROL_PORT, ProtocolError, Rgb};

/// A session unused for this long is assumed expired on the device
/// (it announces a 10 s keep-alive interval) and is replaced.
pub const SESSION_IDLE: Duration = Duration::from_secs(5);

pub struct UdpLink {
    current: Option<(Ipv4Addr, Chameleon2<UdpTransport>, Instant)>,
    port: u16,
}

impl Default for UdpLink {
    fn default() -> Self {
        Self::with_port(CONTROL_PORT)
    }
}

impl UdpLink {
    pub fn new() -> Self {
        Self::default()
    }

    /// A link to strips listening on another UDP port (tests use a loopback fake).
    pub fn with_port(port: u16) -> Self {
        Self { current: None, port }
    }

    /// New session on the strip at `ip`.
    fn open(&self, ip: Ipv4Addr, led_count: usize, cfg: SessionConfig) -> protocol::Result<Chameleon2<UdpTransport>> {
        let transport = UdpTransport::connect(ip, self.port)?;
        let mut dev = Chameleon2::new(Session::new(transport, cfg), led_count)?;
        dev.session_mut().open()?;
        Ok(dev)
    }

    fn connect(&self, ip: Ipv4Addr, cfg: SessionConfig) -> protocol::Result<Chameleon2<UdpTransport>> {
        // LED count is irrelevant for static commands.
        self.open(ip, MAX_LEDS, cfg)
    }

    fn device(&mut self, ip: Ipv4Addr) -> protocol::Result<&mut Chameleon2<UdpTransport>> {
        let reusable = matches!(&self.current, Some((cur, dev, used)) if *cur == ip && dev.session().is_open() && used.elapsed() < SESSION_IDLE);
        if !reusable {
            self.current = None;
            let dev = self.connect(ip, SessionConfig::default())?;
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

fn quick_config() -> SessionConfig {
    SessionConfig { reply_timeout: Duration::from_millis(400), handshake_attempts: 1, ..SessionConfig::default() }
}

impl DeviceLink for UdpLink {
    fn discover(&mut self) -> Result<Vec<DeviceInfo>, String> {
        let found = discovery::discover(&DiscoveryOptions::default()).map_err(|e| e.to_string())?;
        Ok(found.iter().filter(|d| d.is_chameleon2() && d.supports_rgbic_udp()).map(DeviceInfo::from).collect())
    }

    fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String> {
        // A connection test must not end a running stream: time a keep-alive on it instead.
        if let Some((cur, dev, used)) = self.current.as_mut()
            && *cur == ip
            && dev.is_direct()
            && dev.session().is_open()
        {
            let start = Instant::now();
            dev.session_mut().keep_alive().map_err(text)?;
            *used = Instant::now();
            return Ok(start.elapsed());
        }
        self.current = None;
        let start = Instant::now();
        let dev = self.connect(ip, SessionConfig::default()).map_err(text)?;
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
        let quick = quick_config();
        let reusable = matches!(&self.current, Some((cur, dev, used)) if *cur == ip && dev.session().is_open() && used.elapsed() < SESSION_IDLE);
        if !reusable {
            let dev = self.connect(ip, quick).map_err(text)?;
            self.current = Some((ip, dev, Instant::now()));
        }
        let (_, dev, _) = self.current.as_mut().expect("session just ensured");
        dev.set_power(false).map_err(text)
    }

    fn start_stream(&mut self, ip: Ipv4Addr, led_count: usize) -> Result<(), String> {
        self.current = None;
        let mut dev = self.open(ip, led_count, SessionConfig::default()).map_err(text)?;
        // Hardware brightness stays at 100; Ambilight brightness is applied to the frames.
        dev.set_power(true).and_then(|()| dev.set_brightness(100)).and_then(|()| dev.enter_direct()).map_err(text)?;
        self.current = Some((ip, dev, Instant::now()));
        Ok(())
    }

    fn send_frame(&mut self, colors: &[Rgb]) -> Result<(), String> {
        let Some((_, dev, used)) = self.current.as_mut() else { return Err("no stream is running".into()) };
        if dev.led_count() != colors.len() {
            dev.set_led_count(colors.len()).map_err(text)?;
        }
        dev.send_frame(colors).map_err(text)?;
        *used = Instant::now();
        Ok(())
    }

    fn stream_tick(&mut self) -> Result<(), String> {
        let Some((_, dev, _)) = self.current.as_mut() else { return Err("no stream is running".into()) };
        // A single unanswered keep-alive is tolerated; the session decides when it is lost.
        if let Err(e) = dev.tick(Instant::now())
            && !dev.session().is_open()
        {
            return Err(text(e));
        }
        if dev.is_direct() { Ok(()) } else { Err("direct mode was lost".into()) }
    }

    fn stop_stream(&mut self, ip: Ipv4Addr, then: LightChange, quick: bool) -> Result<(), String> {
        let reusable = matches!(&self.current, Some((cur, dev, _)) if *cur == ip && dev.session().is_open());
        if !reusable {
            let cfg = if quick { quick_config() } else { SessionConfig::default() };
            let dev = self.connect(ip, cfg).map_err(text)?;
            self.current = Some((ip, dev, Instant::now()));
        }
        let (_, dev, used) = self.current.as_mut().expect("session just ensured");
        *used = Instant::now();
        dev.leave_direct().and_then(|()| send(dev, then)).map_err(text)
    }

    fn session_open(&self) -> bool {
        matches!(&self.current, Some((_, dev, used)) if dev.session().is_open() && used.elapsed() < SESSION_IDLE)
    }
}
