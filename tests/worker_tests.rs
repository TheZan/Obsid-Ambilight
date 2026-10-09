mod common;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::controller::{LightChange, Target};
use obsid_ambilight::app::state::{Connection, DeviceInfo, Snapshot};
use obsid_ambilight::app::worker::{DeviceLink, Notice, Worker, WorkerOptions, resolve};
use obsid_ambilight::config::{self, Config, ExitAction};
use obsid_ambilight::protocol::Rgb;

const IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 85);

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Discover,
    Handshake(Ipv4Addr),
    Apply(Ipv4Addr, LightChange),
    ExitOff(Ipv4Addr),
}

#[derive(Default)]
struct Script {
    calls: Vec<Call>,
    devices: Vec<DeviceInfo>,
    fail_handshake: bool,
    panic_on_first_discover: bool,
    /// Session counts as open until this moment (set by `apply`).
    session_until: Option<Instant>,
}

#[derive(Clone, Default)]
struct FakeLink(Arc<Mutex<Script>>);

impl FakeLink {
    fn with_strip() -> Self {
        let l = Self::default();
        l.0.lock().unwrap().devices = vec![dev("dcda0cd778bc", IP)];
        l
    }
    fn calls(&self) -> Vec<Call> {
        self.0.lock().unwrap().calls.clone()
    }
}

impl DeviceLink for FakeLink {
    fn discover(&mut self) -> Result<Vec<DeviceInfo>, String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::Discover);
        if std::mem::take(&mut s.panic_on_first_discover) {
            drop(s);
            panic!("simulated worker crash");
        }
        Ok(s.devices.clone())
    }
    fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::Handshake(ip));
        if s.fail_handshake { Err("timeout".into()) } else { Ok(Duration::from_millis(12)) }
    }
    fn apply(&mut self, ip: Ipv4Addr, change: LightChange) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::Apply(ip, change));
        s.session_until = Some(Instant::now() + Duration::from_millis(300));
        Ok(())
    }
    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String> {
        self.0.lock().unwrap().calls.push(Call::ExitOff(ip));
        Ok(())
    }
    fn session_open(&self) -> bool {
        self.0.lock().unwrap().session_until.is_some_and(|t| Instant::now() < t)
    }
}

fn dev(id: &str, ip: Ipv4Addr) -> DeviceInfo {
    DeviceInfo { id: id.into(), name: String::new(), ip, fw_ver: None }
}

fn saved_cfg() -> Config {
    let mut c = Config::default();
    c.device.id = "dcda0cd778bc".into();
    c.device.last_ip = Some(IP);
    c
}

fn fast() -> WorkerOptions {
    WorkerOptions { save_delay: Duration::from_millis(50), min_apply_interval: Duration::from_millis(200) }
}

fn spawn(cfg: Config, link: FakeLink, path: Option<std::path::PathBuf>) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    let notices = Arc::new(Mutex::new(Vec::new()));
    let n = Arc::clone(&notices);
    let w = Worker::spawn(
        cfg,
        Vec::new(),
        path,
        Box::new(link),
        Arc::new(move |x: Notice| n.lock().unwrap().push(x)),
        fast(),
    );
    (w, notices)
}

fn wait_for(w: &Worker, what: &str, pred: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let s = w.snapshot();
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {s:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn resolve_saved_strip_by_discovery() {
    let mut link = FakeLink::with_strip();
    let r = resolve(&mut link, &Target::Saved { id: "dcda0cd778bc".into(), last_ip: None }).unwrap();
    assert!(r.verified);
    assert_eq!(r.device.ip, IP);
}

#[test]
fn resolve_falls_back_to_last_ip_unverified() {
    let mut link = FakeLink::default();
    let r = resolve(&mut link, &Target::Saved { id: "dcda0cd778bc".into(), last_ip: Some(IP) }).unwrap();
    assert!(!r.verified);
    assert_eq!(r.device.id, "dcda0cd778bc");
    assert_eq!(link.calls(), vec![Call::Discover, Call::Handshake(IP)]);
}

#[test]
fn resolve_fails_when_nothing_answers() {
    let mut link = FakeLink::default();
    link.0.lock().unwrap().fail_handshake = true;
    assert!(resolve(&mut link, &Target::Saved { id: "x".into(), last_ip: Some(IP) }).is_err());
    assert!(resolve(&mut link, &Target::Saved { id: "x".into(), last_ip: None }).is_err());
    assert!(resolve(&mut link, &Target::Manual(IP)).is_err());
}

#[test]
fn startup_connects_and_applies_static() {
    let link = FakeLink::with_strip();
    let (w, notices) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !link.calls().iter().any(|c| matches!(c, Call::Apply(..))) {
        assert!(Instant::now() < deadline, "{:?}", link.calls());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(notices.lock().unwrap().contains(&Notice::Status));
}

#[test]
fn throttles_light_changes_and_sends_the_last_value() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    std::thread::sleep(Duration::from_millis(250));
    let before = link.calls().len();
    let start = Instant::now();
    for b in 1..=100u8 {
        w.send(Command::SetStatic { color: None, brightness: Some(b) });
        std::thread::sleep(Duration::from_millis(5));
    }
    wait_for(&w, "last brightness", |s| s.config.light.brightness == 100);
    std::thread::sleep(Duration::from_millis(400));
    let applies: Vec<_> = link.calls()[before..].iter().filter(|c| matches!(c, Call::Apply(..))).cloned().collect();
    let elapsed = start.elapsed().as_secs_f64() + 0.4;
    assert!(applies.len() as f64 <= elapsed * 5.0 + 2.0, "{} applies in {elapsed:.1} s", applies.len());
    assert_eq!(
        applies.last().unwrap(),
        &Call::Apply(
            IP,
            LightChange { power: Some(true), color: Some(Config::default().light.color), brightness: Some(100) }
        )
    );
}

#[test]
fn config_is_saved_after_a_change() {
    let dir = common::temp_dir("worker-save");
    let path = dir.join("config.toml");
    let (w, _) = spawn(saved_cfg(), FakeLink::with_strip(), Some(path.clone()));
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    w.send(Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: None });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if config::load(&path).config.light.color == Rgb::new(1, 2, 3) {
            break;
        }
        assert!(Instant::now() < deadline, "config not saved");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn exit_with_power_off_turns_strip_off_and_stops() {
    let mut cfg = saved_cfg();
    cfg.app.exit_action = ExitAction::Off;
    let link = FakeLink::with_strip();
    let (w, notices) = spawn(cfg, link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    assert!(w.send(Command::Exit));
    assert!(w.join(Duration::from_secs(3)), "worker did not stop");
    assert!(link.calls().contains(&Call::ExitOff(IP)));
    assert!(notices.lock().unwrap().contains(&Notice::Stopped));
}

#[test]
fn worker_restarts_after_a_panic() {
    let link = FakeLink::with_strip();
    link.0.lock().unwrap().panic_on_first_discover = true;
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    let s = wait_for(&w, "ready after restart", |s| s.connection == Connection::Ready);
    assert!(s.errors.iter().any(|e| e.contains("restarted")), "{:?}", s.errors);
    assert_eq!(link.calls().iter().filter(|c| **c == Call::Discover).count(), 2);
}

#[test]
fn notice_round_trips_through_usize() {
    for n in [Notice::Status, Notice::OpenSettings, Notice::Stopped] {
        assert_eq!(Notice::from_usize(n as usize), Some(n));
    }
    assert_eq!(Notice::from_usize(0), None);
}

#[test]
fn a_failed_save_is_reported_in_the_snapshot() {
    let dir = common::temp_dir("worker-save-fail");
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "a file where a directory is expected").unwrap();
    let (w, _) = spawn(saved_cfg(), FakeLink::with_strip(), Some(blocker.join("config.toml")));
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    w.send(Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: None });
    wait_for(&w, "save error", |s| s.errors.iter().any(|e| e.contains("could not save settings")));
}

#[test]
fn session_status_clears_on_its_own_after_the_session_expires() {
    let (w, _) = spawn(saved_cfg(), FakeLink::with_strip(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    w.send(Command::SetPower { on: true });
    wait_for(&w, "session open", |s| s.session_open);
    // No further commands: the worker must notice the expiry by itself.
    wait_for(&w, "session closed", |s| !s.session_open);
}
