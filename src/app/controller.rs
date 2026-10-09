//! Pure state machine of the device worker. It never touches the network or
//! the clock: the worker feeds it events with the current time and executes
//! the actions it returns, feeding results back as events.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::commands::Command;
use super::state::{Connection, DeviceInfo, Snapshot, TestState};
use crate::config::{self, Config, ExitAction, LightConfig};
use crate::protocol::Rgb;
use crate::protocol::packets::MAX_LEDS;
use crate::reconnect::Backoff;

pub const MAX_ERRORS: usize = 20;
pub const RETRY_BASE: Duration = Duration::from_secs(2);
pub const RETRY_MAX: Duration = Duration::from_secs(300);

/// Commands to send to the strip, in order: power, colour, brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightChange {
    pub power: Option<bool>,
    pub color: Option<Rgb>,
    pub brightness: Option<u8>,
}

impl LightChange {
    /// The full user state: on + colour + brightness, or just off.
    pub fn full(light: &LightConfig) -> Self {
        if light.power {
            Self { power: Some(true), color: Some(light.color), brightness: Some(light.brightness) }
        } else {
            Self::off()
        }
    }

    pub fn off() -> Self {
        Self { power: Some(false), color: None, brightness: None }
    }
}

/// How to find the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Multicast discovery by id, then the last known IP.
    Saved { id: String, last_ip: Option<Ipv4Addr> },
    /// User-entered address, used as is.
    Manual(Ipv4Addr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub device: DeviceInfo,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Startup,
    Command(Command),
    Resolved(Result<Resolved, String>),
    Discovered(Result<Vec<DeviceInfo>, String>),
    Applied(Result<(), String>),
    Tested(Result<Duration, String>),
    /// Time has passed; lets due retries fire.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Resolve(Target),
    Discover,
    Apply { ip: Ipv4Addr, change: LightChange },
    Test { ip: Ipv4Addr },
    SaveConfig,
    OpenSettings,
    Shutdown { power_off_ip: Option<Ipv4Addr> },
}

pub struct Controller {
    cfg: Config,
    connection: Connection,
    device: Option<DeviceInfo>,
    verified: bool,
    discovered: Vec<DeviceInfo>,
    searching: bool,
    /// First run: select the strip automatically if discovery finds exactly one.
    auto_select: bool,
    /// Change to send once the strip is reachable.
    pending: Option<LightChange>,
    retry_at: Option<Instant>,
    retry_in_secs: Option<u64>,
    backoff: Backoff,
    test: TestState,
    errors: VecDeque<String>,
    config_warnings: Vec<String>,
}

impl Controller {
    pub fn new(cfg: Config, config_warnings: Vec<String>, backoff: Backoff) -> Self {
        Self {
            cfg,
            connection: Connection::NoDevice,
            device: None,
            verified: false,
            discovered: Vec::new(),
            searching: false,
            auto_select: false,
            pending: None,
            retry_at: None,
            retry_in_secs: None,
            backoff,
            test: TestState::Idle,
            errors: VecDeque::new(),
            config_warnings,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// When the controller next needs a `Tick`.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.retry_at
    }

    pub fn record_error(&mut self, message: String) {
        warn!(%message, "error");
        if self.errors.len() == MAX_ERRORS {
            self.errors.pop_front();
        }
        self.errors.push_back(message);
    }

    pub fn snapshot(&self, session_open: bool) -> Snapshot {
        Snapshot {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            connection: self.connection,
            retry_in_secs: self.retry_in_secs,
            device: self.device.clone(),
            verified: self.verified,
            session_open,
            searching: self.searching,
            discovered: self.discovered.clone(),
            test: self.test.clone(),
            errors: self.errors.iter().cloned().collect(),
            config_warnings: self.config_warnings.clone(),
            config: self.cfg.clone(),
        }
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match event {
            Event::Startup => self.startup(),
            Event::Command(cmd) => self.command(cmd),
            Event::Tick => match self.retry_at {
                Some(at) if self.connection == Connection::Offline && now >= at => self.begin_resolve(),
                _ => Vec::new(),
            },
            Event::Resolved(Ok(r)) => self.on_resolved(r),
            Event::Resolved(Err(e)) => {
                self.record_error(e);
                self.go_offline(now);
                Vec::new()
            }
            Event::Discovered(result) => self.on_discovered(result),
            Event::Applied(Ok(())) => {
                self.backoff.reset();
                Vec::new()
            }
            Event::Applied(Err(e)) => {
                self.record_error(e);
                // Whatever was being sent, re-send the user's full state later.
                self.pending = Some(LightChange::full(&self.cfg.light));
                self.go_offline(now);
                Vec::new()
            }
            Event::Tested(result) => {
                self.test = match result {
                    Ok(d) => TestState::Ok { millis: d.as_millis() as u64 },
                    Err(message) => TestState::Failed { message },
                };
                Vec::new()
            }
        }
    }

    fn startup(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.cfg.startup.power_on {
            if !self.cfg.light.power {
                self.cfg.light.power = true;
                actions.push(Action::SaveConfig);
            }
            self.pending = Some(LightChange::full(&self.cfg.light));
        }
        if self.target().is_some() {
            actions.extend(self.begin_resolve());
        } else {
            self.auto_select = true;
            self.searching = true;
            actions.push(Action::Discover);
        }
        actions
    }

    fn target(&self) -> Option<Target> {
        if let Some(ip) = self.cfg.device.manual_ip {
            Some(Target::Manual(ip))
        } else if !self.cfg.device.id.is_empty() {
            Some(Target::Saved { id: self.cfg.device.id.clone(), last_ip: self.cfg.device.last_ip })
        } else {
            None
        }
    }

    fn begin_resolve(&mut self) -> Vec<Action> {
        self.retry_at = None;
        self.retry_in_secs = None;
        match self.target() {
            Some(t) => {
                self.connection = Connection::Connecting;
                vec![Action::Resolve(t)]
            }
            None => {
                self.connection = Connection::NoDevice;
                self.device = None;
                Vec::new()
            }
        }
    }

    fn go_offline(&mut self, now: Instant) {
        let delay = self.backoff.next_delay().unwrap_or(RETRY_MAX);
        self.connection = Connection::Offline;
        self.retry_at = Some(now + delay);
        self.retry_in_secs = Some(delay.as_secs().max(1));
        info!(?delay, "strip offline, will retry");
    }

    fn on_resolved(&mut self, r: Resolved) -> Vec<Action> {
        let mut actions = Vec::new();
        let d = &mut self.cfg.device;
        let before = d.clone();
        if !r.device.id.is_empty() {
            d.id = r.device.id.clone();
        }
        if !r.device.name.is_empty() {
            d.name = r.device.name.clone();
        }
        if d.manual_ip.is_none() {
            d.last_ip = Some(r.device.ip);
        }
        if *d != before {
            actions.push(Action::SaveConfig);
        }
        self.become_ready(r.device, r.verified);
        match self.pending.take() {
            // Being found is not enough when commands keep failing: the
            // backoff resets only after the pending change is applied.
            Some(change) => actions.push(self.apply_action(change)),
            None => self.backoff.reset(),
        }
        actions
    }

    fn become_ready(&mut self, device: DeviceInfo, verified: bool) {
        self.connection = Connection::Ready;
        self.device = Some(device);
        self.verified = verified;
        self.retry_at = None;
        self.retry_in_secs = None;
    }

    fn on_discovered(&mut self, result: Result<Vec<DeviceInfo>, String>) -> Vec<Action> {
        self.searching = false;
        match result {
            Ok(list) => self.discovered = list,
            Err(e) => self.record_error(e),
        }
        if !std::mem::take(&mut self.auto_select) {
            return Vec::new();
        }
        if self.discovered.len() == 1 {
            let dev = self.discovered[0].clone();
            self.select(dev)
        } else {
            vec![Action::OpenSettings]
        }
    }

    fn select(&mut self, dev: DeviceInfo) -> Vec<Action> {
        let d = &mut self.cfg.device;
        d.id = dev.id.clone();
        d.name = dev.name.clone();
        d.last_ip = Some(dev.ip);
        d.manual_ip = None;
        self.become_ready(dev, true);
        // A strip the user just picked starts with a fresh retry schedule.
        self.backoff.reset();
        let mut actions = vec![Action::SaveConfig];
        let change = self.pending.take().or(self.cfg.light.power.then(|| LightChange::full(&self.cfg.light)));
        if let Some(change) = change {
            actions.push(self.apply_action(change));
        }
        actions
    }

    fn apply_action(&self, change: LightChange) -> Action {
        let ip = self.device.as_ref().map(|d| d.ip).expect("apply_action needs a device");
        Action::Apply { ip, change }
    }

    /// Sends now when ready, otherwise keeps it for the next connection.
    fn light(&mut self, change: LightChange) -> Vec<Action> {
        if self.connection == Connection::Ready && self.device.is_some() {
            vec![self.apply_action(change)]
        } else {
            self.pending = Some(change);
            Vec::new()
        }
    }

    fn command(&mut self, cmd: Command) -> Vec<Action> {
        match cmd {
            Command::SetStatic { color, brightness } => {
                if let Some(b) = brightness.filter(|b| !(1..=100).contains(b)) {
                    self.record_error(format!("brightness {b} is outside 1..=100"));
                    return Vec::new();
                }
                if color == Some(Rgb::BLACK) {
                    self.record_error("black cannot be a static colour".to_owned());
                    return Vec::new();
                }
                let l = &mut self.cfg.light;
                l.power = true;
                if let Some(c) = color {
                    l.color = c;
                }
                if let Some(b) = brightness {
                    l.brightness = b;
                }
                let mut actions = vec![Action::SaveConfig];
                actions.extend(self.light(LightChange::full(&self.cfg.light)));
                actions
            }
            Command::SetPower { on } => {
                self.cfg.light.power = on;
                let mut actions = vec![Action::SaveConfig];
                actions.extend(self.light(LightChange::full(&self.cfg.light)));
                actions
            }
            Command::SelectDevice { id } => match self.discovered.iter().find(|d| d.id == id).cloned() {
                Some(dev) => self.select(dev),
                None => {
                    self.record_error(format!("strip {id} is not in the discovery list"));
                    Vec::new()
                }
            },
            Command::SetManualIp { ip } => {
                self.cfg.device.manual_ip = ip;
                self.device = None;
                if self.cfg.light.power {
                    self.pending = Some(LightChange::full(&self.cfg.light));
                }
                let mut actions = vec![Action::SaveConfig];
                actions.extend(self.begin_resolve());
                actions
            }
            Command::Discover => {
                if self.searching {
                    return Vec::new();
                }
                self.searching = true;
                vec![Action::Discover]
            }
            Command::TestConnection => {
                match self.device.as_ref().map(|d| d.ip).or(self.cfg.device.manual_ip).or(self.cfg.device.last_ip) {
                    Some(ip) => {
                        self.test = TestState::Running;
                        vec![Action::Test { ip }]
                    }
                    None => {
                        self.test = TestState::Failed { message: "no strip selected".to_owned() };
                        Vec::new()
                    }
                }
            }
            Command::SetLanguage { language } => {
                self.cfg.app.language = language;
                vec![Action::SaveConfig]
            }
            Command::SetPowerOnStart { enabled } => {
                self.cfg.startup.power_on = enabled;
                vec![Action::SaveConfig]
            }
            Command::SetExitAction { action } => {
                self.cfg.app.exit_action = action;
                vec![Action::SaveConfig]
            }
            Command::SetLedCount { count } => {
                if !(1..=MAX_LEDS).contains(&count) {
                    self.record_error(format!("LED count {count} is outside 1..={MAX_LEDS}"));
                    return Vec::new();
                }
                self.cfg.device.led_count = count;
                vec![Action::SaveConfig]
            }
            Command::SetPresets { presets } => {
                let (clean, warnings) = config::sanitize_presets(presets);
                for w in warnings {
                    self.record_error(w);
                }
                self.cfg.light.presets = clean;
                vec![Action::SaveConfig]
            }
            Command::Exit => {
                let power_off_ip = match (self.cfg.app.exit_action, self.connection, &self.device) {
                    (ExitAction::Off, Connection::Ready, Some(d)) => Some(d.ip),
                    _ => None,
                };
                vec![Action::Shutdown { power_off_ip }]
            }
        }
    }
}
