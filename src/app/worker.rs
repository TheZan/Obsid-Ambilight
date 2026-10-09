//! The device worker thread: owns the link to the strip, runs the
//! [`Controller`], throttles light changes, saves the config and survives
//! panics (up to [`MAX_RESTARTS`] in a row).

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{error, info};

use super::commands::{Command, coalesce};
use super::controller::{Action, Controller, Event, LightChange, RETRY_BASE, RETRY_MAX, Resolved, Target};
use super::state::{DeviceInfo, Snapshot};
use crate::config::{self, Config};
use crate::reconnect::Backoff;

pub const MAX_RESTARTS: u32 = 3;

/// Access to the strip; [`UdpLink`](super::link::UdpLink) in the app, a fake in tests.
pub trait DeviceLink: Send {
    /// Chameleon2 strips that answered a LAN search.
    fn discover(&mut self) -> Result<Vec<DeviceInfo>, String>;
    /// Opens a fresh UDP session; returns how long it took.
    fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String>;
    fn apply(&mut self, ip: Ipv4Addr, change: LightChange) -> Result<(), String>;
    /// Best-effort power off with short timeouts (app exit).
    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String>;
    fn session_open(&self) -> bool;
}

/// What the worker tells the UI thread. Values are stable: they travel in WPARAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Status = 1,
    OpenSettings = 2,
    Stopped = 3,
}

impl Notice {
    pub fn from_usize(v: usize) -> Option<Self> {
        match v {
            1 => Some(Self::Status),
            2 => Some(Self::OpenSettings),
            3 => Some(Self::Stopped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Delay before writing the config after a change (coalesces bursts).
    pub save_delay: Duration,
    /// Minimum time between two light applies (≈ 5 per second).
    pub min_apply_interval: Duration,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self { save_delay: Duration::from_millis(500), min_apply_interval: Duration::from_millis(200) }
    }
}

/// Finds the strip for a target. Saved strips are matched by id via
/// discovery first; the last IP is only a fallback and is reported unverified.
pub fn resolve(link: &mut dyn DeviceLink, target: &Target) -> Result<Resolved, String> {
    match target {
        Target::Manual(ip) => {
            link.handshake(*ip).map_err(|e| format!("no reply from {ip}: {e}"))?;
            Ok(Resolved {
                device: DeviceInfo { id: String::new(), name: String::new(), ip: *ip, fw_ver: None },
                verified: false,
            })
        }
        Target::Saved { id, last_ip } => {
            let discovery_error = match link.discover() {
                Ok(list) => match list.into_iter().find(|d| d.id == *id) {
                    Some(device) => return Ok(Resolved { device, verified: true }),
                    None => None,
                },
                Err(e) => Some(e),
            };
            match last_ip {
                Some(ip) => {
                    link.handshake(*ip).map_err(|e| format!("strip {id} not found; last IP {ip}: {e}"))?;
                    Ok(Resolved {
                        device: DeviceInfo { id: id.clone(), name: String::new(), ip: *ip, fw_ver: None },
                        verified: false,
                    })
                }
                None => Err(match discovery_error {
                    Some(e) => format!("strip {id} not found: {e}"),
                    None => format!("strip {id} not found on the network"),
                }),
            }
        }
    }
}

type Notify = Arc<dyn Fn(Notice) + Send + Sync>;

/// Cloneable access for other threads (IPC server, tray).
#[derive(Clone)]
pub struct WorkerHandle {
    tx: Sender<Command>,
    shared: Arc<Mutex<Snapshot>>,
}

impl WorkerHandle {
    /// Queues a command; false if the worker has stopped.
    pub fn send(&self, cmd: Command) -> bool {
        self.tx.send(cmd).is_ok()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

pub struct Worker {
    handle: WorkerHandle,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(
        config: Config,
        warnings: Vec<String>,
        config_path: Option<PathBuf>,
        mut link: Box<dyn DeviceLink>,
        notify: Notify,
        options: WorkerOptions,
    ) -> Worker {
        let (tx, rx) = mpsc::channel();
        let first = new_controller(config, warnings);
        let shared = Arc::new(Mutex::new(first.snapshot(false)));
        let handle = WorkerHandle { tx, shared: Arc::clone(&shared) };
        let thread = thread::Builder::new()
            .name("device-worker".into())
            .spawn(move || {
                let ctx = Ctx { shared, notify, config_path, options };
                let mut ctrl = first;
                let mut restarts = 0;
                loop {
                    let result = catch_unwind(AssertUnwindSafe(|| run(&mut ctrl, link.as_mut(), &rx, &ctx)));
                    match result {
                        Ok(()) => return,
                        Err(_) if restarts < MAX_RESTARTS => {
                            restarts += 1;
                            error!(restarts, "device worker panicked, restarting");
                            let snap = ctx.snapshot();
                            ctrl = new_controller(snap.config, snap.config_warnings);
                            for e in snap.errors {
                                ctrl.record_error(e);
                            }
                            ctrl.record_error(format!(
                                "device worker restarted after an internal error ({restarts}/{MAX_RESTARTS})"
                            ));
                        }
                        Err(_) => {
                            error!("device worker failed repeatedly, giving up");
                            let mut snap = ctx.snapshot();
                            snap.errors.push("device worker stopped after repeated internal errors".into());
                            ctx.publish_snapshot(snap);
                            return;
                        }
                    }
                }
            })
            .expect("spawn device worker");
        Worker { handle, thread: Some(thread) }
    }

    pub fn send(&self, cmd: Command) -> bool {
        self.handle.send(cmd)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.handle.snapshot()
    }

    pub fn handle(&self) -> WorkerHandle {
        self.handle.clone()
    }

    /// Waits up to `timeout` for the thread to end; true if it ended.
    pub fn join(mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while let Some(t) = &self.thread {
            if t.is_finished() {
                let _ = self.thread.take().map(JoinHandle::join);
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        true
    }
}

fn new_controller(config: Config, warnings: Vec<String>) -> Controller {
    Controller::new(config, warnings, Backoff::new(RETRY_BASE, RETRY_MAX, None))
}

struct Ctx {
    shared: Arc<Mutex<Snapshot>>,
    notify: Notify,
    config_path: Option<PathBuf>,
    options: WorkerOptions,
}

impl Ctx {
    fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn publish_snapshot(&self, snap: Snapshot) {
        let changed = {
            let mut cur = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let changed = *cur != snap;
            *cur = snap;
            changed
        };
        if changed {
            (self.notify)(Notice::Status);
        }
    }
}

/// One worker life. Returns normally only on exit or when all senders are gone.
fn run(ctrl: &mut Controller, link: &mut dyn DeviceLink, rx: &Receiver<Command>, ctx: &Ctx) {
    let mut state = RunState { save_at: None, last_apply: None, exit: false };
    process(ctrl, link, ctx, &mut state, Event::Startup);

    while !state.exit {
        let now = Instant::now();
        let deadline = [ctrl.next_deadline(), state.save_at].into_iter().flatten().min();
        let wait = deadline.map_or(Duration::from_secs(3600), |d| d.saturating_duration_since(now));
        match rx.recv_timeout(wait) {
            Ok(first) => {
                let mut batch = vec![first];
                batch.extend(rx.try_iter());
                // Throttle: wait out the minimum interval, collecting newer commands.
                if batch.iter().any(Command::is_light_change) {
                    if let Some(last) = state.last_apply {
                        let ready_at = last + ctx.options.min_apply_interval;
                        while let Some(rest) = ready_at.checked_duration_since(Instant::now()) {
                            match rx.recv_timeout(rest) {
                                Ok(cmd) => batch.push(cmd),
                                Err(RecvTimeoutError::Timeout) => break,
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        batch.extend(rx.try_iter());
                    }
                }
                for cmd in coalesce(batch) {
                    process(ctrl, link, ctx, &mut state, Event::Command(cmd));
                    if state.exit {
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => process(ctrl, link, ctx, &mut state, Event::Tick),
            Err(RecvTimeoutError::Disconnected) => {
                save_now(ctrl, ctx, &mut state);
                return;
            }
        }
        if state.save_at.is_some_and(|t| Instant::now() >= t) {
            save_now(ctrl, ctx, &mut state);
        }
    }
}

struct RunState {
    save_at: Option<Instant>,
    last_apply: Option<Instant>,
    exit: bool,
}

/// Feeds one event and executes the resulting actions, following up with the
/// events they produce until nothing is left.
fn process(ctrl: &mut Controller, link: &mut dyn DeviceLink, ctx: &Ctx, st: &mut RunState, event: Event) {
    let mut queue = VecDeque::from([event]);
    while let Some(ev) = queue.pop_front() {
        let actions = ctrl.handle(ev, Instant::now());
        ctx.publish_snapshot(ctrl.snapshot(link.session_open()));
        for action in actions {
            match action {
                Action::Resolve(target) => queue.push_back(Event::Resolved(resolve(link, &target))),
                Action::Discover => queue.push_back(Event::Discovered(link.discover())),
                Action::Apply { ip, change } => {
                    st.last_apply = Some(Instant::now());
                    queue.push_back(Event::Applied(link.apply(ip, change)));
                }
                Action::Test { ip } => queue.push_back(Event::Tested(link.handshake(ip))),
                Action::SaveConfig => {
                    st.save_at.get_or_insert(Instant::now() + ctx.options.save_delay);
                }
                Action::OpenSettings => (ctx.notify)(Notice::OpenSettings),
                Action::Shutdown { power_off_ip } => {
                    if let Some(ip) = power_off_ip {
                        if let Err(e) = link.power_off_for_exit(ip) {
                            info!(%e, "power off on exit failed");
                        }
                    }
                    save_now(ctrl, ctx, st);
                    st.exit = true;
                    (ctx.notify)(Notice::Stopped);
                    return;
                }
            }
        }
        ctx.publish_snapshot(ctrl.snapshot(link.session_open()));
    }
}

fn save_now(ctrl: &mut Controller, ctx: &Ctx, st: &mut RunState) {
    if st.save_at.take().is_none() {
        return;
    }
    if let Some(path) = &ctx.config_path {
        if let Err(e) = config::save(path, ctrl.config()) {
            ctrl.record_error(format!("could not save settings: {e}"));
        }
    }
}
