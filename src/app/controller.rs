//! Pure state machine of the device worker. It never touches the network or
//! the clock: the worker feeds it events with the current time and executes
//! the actions it returns, feeding results back as events.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::commands::Command;
use super::state::{Connection, DeviceInfo, Snapshot, StreamStatus, TestState};
use crate::ambilight::layout::{self, HighlightTarget};
use crate::config::{self, AmbilightConfig, Config, ExitAction, LightConfig, Mode};
use crate::protocol::Rgb;
use crate::protocol::packets::MAX_LEDS;
use crate::reconnect::Backoff;

pub const MAX_ERRORS: usize = 20;
pub const RETRY_BASE: Duration = Duration::from_secs(2);
pub const RETRY_MAX: Duration = Duration::from_secs(300);
/// A layout highlight ends this long after the last `Highlight` command.
pub const HIGHLIGHT_TTL: Duration = Duration::from_secs(3);

/// Commands to send to the strip, in order: power, colour, brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightChange {
    pub power: Option<bool>,
    pub color: Option<Rgb>,
    pub brightness: Option<u8>,
}

impl LightChange {
    /// The full static state: on + colour + brightness, or just off.
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

/// What the strip should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desired {
    /// Static colour or off, sent as plain commands.
    Light(LightChange),
    /// Screen colours streamed in direct mode.
    Ambilight,
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
    /// Result of `Action::StartStream`.
    StreamStarted(Result<(), String>),
    /// Result of `Action::StopStream` on the strip at this address.
    StreamStopped(Ipv4Addr, Result<(), String>),
    /// The running stream lost its session.
    StreamFailed(String),
    /// Time has passed; lets due retries and highlight expiry fire.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Resolve(Target),
    Discover,
    Apply {
        ip: Ipv4Addr,
        change: LightChange,
    },
    /// Enter direct mode; the worker sends frames until `StopStream`.
    StartStream {
        ip: Ipv4Addr,
    },
    /// Leave direct mode, then send `then`.
    StopStream {
        ip: Ipv4Addr,
        then: LightChange,
    },
    Test {
        ip: Ipv4Addr,
    },
    SaveConfig,
    OpenSettings,
    /// `stop_stream` leaves direct mode with short timeouts and applies its
    /// change; `power_off_ip` switches off a strip that is not streaming.
    Shutdown {
        power_off_ip: Option<Ipv4Addr>,
        stop_stream: Option<(Ipv4Addr, LightChange)>,
    },
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
    /// State to reach once the strip is reachable.
    pending: Option<Desired>,
    retry_at: Option<Instant>,
    retry_in_secs: Option<u64>,
    backoff: Backoff,
    test: TestState,
    errors: VecDeque<String>,
    config_warnings: Vec<String>,
    /// Direct mode is on (or `StartStream` was just issued).
    streaming: bool,
    /// Layout editor overlay and when it expires.
    highlight: Option<(HighlightTarget, Instant)>,
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
            streaming: false,
            highlight: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// When the controller next needs a `Tick`.
    pub fn next_deadline(&self) -> Option<Instant> {
        [self.retry_at, self.highlight.map(|(_, until)| until)].into_iter().flatten().min()
    }

    pub fn streaming(&self) -> bool {
        self.streaming
    }

    /// The layout overlay to send instead of screen colours.
    pub fn highlight(&self) -> Option<HighlightTarget> {
        self.highlight.map(|(t, _)| t)
    }

    /// Screen capture is needed: streaming in Ambilight mode.
    pub fn capture_wanted(&self) -> bool {
        self.streaming && self.connection == Connection::Ready && self.desired() == Desired::Ambilight
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
            stream: StreamStatus {
                streaming: self.streaming,
                highlight: self.highlight.is_some(),
                ..StreamStatus::default()
            },
        }
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match event {
            Event::Startup => self.startup(),
            Event::Command(cmd) => self.command(cmd, now),
            Event::Tick => self.tick(now),
            Event::Resolved(Ok(r)) => self.on_resolved(r),
            Event::Resolved(Err(e)) => {
                self.record_error(e);
                self.go_offline(now);
                Vec::new()
            }
            Event::Discovered(result) => self.on_discovered(result),
            Event::Applied(Ok(())) | Event::StreamStarted(Ok(())) | Event::StreamStopped(_, Ok(())) => {
                self.backoff.reset();
                Vec::new()
            }
            Event::Applied(Err(e)) => self.fail(e, now),
            Event::StreamStarted(Err(e)) | Event::StreamFailed(e) => {
                self.streaming = false;
                self.fail(e, now)
            }
            Event::StreamStopped(ip, Err(e)) => {
                if self.ready_ip() == Some(ip) {
                    self.fail(e, now)
                } else {
                    // The user has already switched to another strip.
                    self.record_error(e);
                    Vec::new()
                }
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

    /// A command to the strip failed: keep the user's state and go offline.
    fn fail(&mut self, e: String, now: Instant) -> Vec<Action> {
        self.record_error(e);
        self.pending = Some(self.desired());
        self.go_offline(now);
        Vec::new()
    }

    fn tick(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.highlight.is_some_and(|(_, until)| now >= until) {
            actions.extend(self.end_highlight());
        }
        if self.connection == Connection::Offline && self.retry_at.is_some_and(|at| now >= at) {
            actions.extend(self.begin_resolve());
        }
        actions
    }

    fn startup(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.cfg.startup.power_on {
            if !self.cfg.light.power {
                self.cfg.light.power = true;
                actions.push(Action::SaveConfig);
            }
            self.pending = Some(self.desired());
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
        self.streaming = false;
        self.highlight = None;
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
            // backoff resets only after the pending state is reached.
            Some(desired) => actions.extend(self.drive(desired)),
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
        let mut actions = self.leave_stream();
        let d = &mut self.cfg.device;
        d.id = dev.id.clone();
        d.name = dev.name.clone();
        d.last_ip = Some(dev.ip);
        d.manual_ip = None;
        self.become_ready(dev, true);
        // A strip the user just picked starts with a fresh retry schedule.
        self.backoff.reset();
        actions.push(Action::SaveConfig);
        let desired = match self.pending.take() {
            Some(d) => Some(d),
            None => self.cfg.light.power.then(|| self.desired()),
        };
        if let Some(d) = desired {
            actions.extend(self.drive(d));
        }
        actions
    }

    /// The user's chosen state of the strip.
    fn desired(&self) -> Desired {
        if self.cfg.light.power && self.cfg.app.mode == Mode::Ambilight {
            Desired::Ambilight
        } else {
            Desired::Light(LightChange::full(&self.cfg.light))
        }
    }

    fn ready_ip(&self) -> Option<Ipv4Addr> {
        if self.connection == Connection::Ready { self.device.as_ref().map(|d| d.ip) } else { None }
    }

    /// Moves the strip to `desired` now when ready, otherwise keeps it for the
    /// next connection.
    fn drive(&mut self, desired: Desired) -> Vec<Action> {
        let Some(ip) = self.ready_ip() else {
            self.pending = Some(desired);
            return Vec::new();
        };
        // The editor overlay keeps the stream; the user's state follows when it ends.
        if self.highlight.is_some() {
            return Vec::new();
        }
        match (desired, self.streaming) {
            (Desired::Ambilight, true) => Vec::new(),
            (Desired::Ambilight, false) => {
                self.streaming = true;
                vec![Action::StartStream { ip }]
            }
            (Desired::Light(change), true) => {
                self.streaming = false;
                vec![Action::StopStream { ip, then: change }]
            }
            (Desired::Light(change), false) => vec![Action::Apply { ip, change }],
        }
    }

    /// Leaves direct mode on the current strip before switching to another
    /// one; the old strip keeps the static colour.
    fn leave_stream(&mut self) -> Vec<Action> {
        self.highlight = None;
        match (self.streaming, self.ready_ip()) {
            (true, Some(ip)) => {
                self.streaming = false;
                vec![Action::StopStream { ip, then: LightChange::full(&self.cfg.light) }]
            }
            _ => Vec::new(),
        }
    }

    fn save_and_drive(&mut self) -> Vec<Action> {
        let mut actions = vec![Action::SaveConfig];
        let desired = self.desired();
        actions.extend(self.drive(desired));
        actions
    }

    fn start_highlight(&mut self, target: HighlightTarget, now: Instant) -> Vec<Action> {
        let Some(ip) = self.ready_ip() else {
            self.record_error("cannot highlight LEDs: the strip is not connected".to_owned());
            return Vec::new();
        };
        let valid = match target {
            HighlightTarget::Led(i) => i < self.cfg.device.led_count,
            HighlightTarget::Segment(k) => k < self.cfg.ambilight.segments.len(),
        };
        if !valid {
            self.record_error(format!("nothing to highlight at {target:?}"));
            return Vec::new();
        }
        self.highlight = Some((target, now + HIGHLIGHT_TTL));
        if self.streaming {
            Vec::new()
        } else {
            self.streaming = true;
            vec![Action::StartStream { ip }]
        }
    }

    fn end_highlight(&mut self) -> Vec<Action> {
        if self.highlight.take().is_none() {
            return Vec::new();
        }
        let desired = self.desired();
        self.drive(desired)
    }

    fn command(&mut self, cmd: Command, now: Instant) -> Vec<Action> {
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
                self.cfg.app.mode = Mode::Static;
                self.save_and_drive()
            }
            Command::SetPower { on } => {
                self.cfg.light.power = on;
                self.save_and_drive()
            }
            Command::SetMode { mode } => {
                self.cfg.app.mode = mode;
                self.cfg.light.power = true;
                self.save_and_drive()
            }
            Command::SetAmbilight { settings } => {
                if let Err(e) = settings.check() {
                    self.record_error(e);
                    return Vec::new();
                }
                let segments = std::mem::take(&mut self.cfg.ambilight.segments);
                self.cfg.ambilight = AmbilightConfig { segments, ..settings };
                vec![Action::SaveConfig]
            }
            Command::SetLayout { segments } => {
                let (clean, warnings) = layout::sanitize(segments, self.cfg.device.led_count);
                for w in warnings {
                    self.record_error(w);
                }
                self.cfg.ambilight.segments = clean;
                vec![Action::SaveConfig]
            }
            Command::Highlight { target } => self.start_highlight(target, now),
            Command::StopHighlight => self.end_highlight(),
            Command::SelectDevice { id } => match self.discovered.iter().find(|d| d.id == id).cloned() {
                Some(dev) => self.select(dev),
                None => {
                    self.record_error(format!("strip {id} is not in the discovery list"));
                    Vec::new()
                }
            },
            Command::SetManualIp { ip } => {
                let mut actions = self.leave_stream();
                self.cfg.device.manual_ip = ip;
                self.device = None;
                if self.cfg.light.power {
                    self.pending = Some(self.desired());
                }
                actions.push(Action::SaveConfig);
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
                self.cfg.ambilight.segments = layout::trim(&self.cfg.ambilight.segments, count);
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
                let ip = self.ready_ip();
                let stop_stream = match (self.streaming, ip) {
                    (true, Some(ip)) => Some((
                        ip,
                        match self.cfg.app.exit_action {
                            // A live stream cannot outlive the app: keep the static colour.
                            ExitAction::Keep => LightChange::full(&self.cfg.light),
                            ExitAction::Off => LightChange::off(),
                        },
                    )),
                    _ => None,
                };
                let power_off_ip = match (stop_stream.is_none(), self.cfg.app.exit_action, ip) {
                    (true, ExitAction::Off, Some(ip)) => Some(ip),
                    _ => None,
                };
                vec![Action::Shutdown { power_off_ip, stop_stream }]
            }
        }
    }
}
