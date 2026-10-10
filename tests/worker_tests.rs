mod common;

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::Image;
use obsid_ambilight::ambilight::layout::HighlightTarget;
use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::controller::{LightChange, Target};
use obsid_ambilight::app::state::{Connection, Dark, DeviceInfo, Snapshot};
use obsid_ambilight::app::worker::{AutostartFn, DeviceLink, Notice, Worker, WorkerOptions, resolve};
use obsid_ambilight::capture::{Grabber, GrabberFactory, GrabberInfo};
use obsid_ambilight::config::{self, Config, ExitAction, Mode};
use obsid_ambilight::protocol::Rgb;

const IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 85);

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Discover,
    Handshake(Ipv4Addr),
    Apply(Ipv4Addr, LightChange),
    ExitOff(Ipv4Addr),
    StartStream(Ipv4Addr, usize),
    Frame(Vec<Rgb>),
    StopStream(Ipv4Addr, LightChange, bool),
}

#[derive(Default)]
struct Script {
    calls: Vec<Call>,
    devices: Vec<DeviceInfo>,
    fail_handshake: bool,
    panic_on_first_discover: bool,
    /// The next `stream_tick` reports a lost session.
    fail_next_tick: bool,
    /// Session counts as open until this moment (set by `apply`).
    session_until: Option<Instant>,
    streaming: bool,
    /// Delay inside `apply` (a busy worker).
    apply_delay: Duration,
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
    fn frames(&self) -> Vec<Vec<Rgb>> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Frame(f) => Some(f),
                _ => None,
            })
            .collect()
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
        let delay = self.0.lock().unwrap().apply_delay;
        std::thread::sleep(delay);
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::Apply(ip, change));
        s.session_until = Some(Instant::now() + Duration::from_millis(300));
        Ok(())
    }
    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String> {
        self.0.lock().unwrap().calls.push(Call::ExitOff(ip));
        Ok(())
    }
    fn start_stream(&mut self, ip: Ipv4Addr, led_count: usize) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::StartStream(ip, led_count));
        s.streaming = true;
        Ok(())
    }
    fn send_frame(&mut self, colors: &[Rgb]) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        assert!(s.streaming, "frame sent outside a stream");
        s.calls.push(Call::Frame(colors.to_vec()));
        Ok(())
    }
    fn stream_tick(&mut self) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        if std::mem::take(&mut s.fail_next_tick) {
            s.streaming = false;
            return Err("keep-alive lost".into());
        }
        Ok(())
    }
    fn stop_stream(&mut self, ip: Ipv4Addr, then: LightChange, quick: bool) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::StopStream(ip, then, quick));
        s.streaming = false;
        Ok(())
    }
    fn session_open(&self) -> bool {
        let s = self.0.lock().unwrap();
        s.streaming || s.session_until.is_some_and(|t| Instant::now() < t)
    }
}

/// Grabber that always shows one solid colour.
struct SolidGrabber {
    info: GrabberInfo,
    color: [f32; 3],
    alive: Arc<AtomicUsize>,
}

impl Grabber for SolidGrabber {
    fn info(&self) -> &GrabberInfo {
        &self.info
    }
    fn grab(&mut self) -> Result<Option<Image>, String> {
        Ok(Some(Image::filled(32, 18, self.color)))
    }
}

impl Drop for SolidGrabber {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Factory of red-screen grabbers; the counter tracks live grabbers.
fn red_screen() -> (GrabberFactory, Arc<AtomicUsize>) {
    let alive = Arc::new(AtomicUsize::new(0));
    let a = Arc::clone(&alive);
    let f: GrabberFactory = Arc::new(move |_: &str| {
        a.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(SolidGrabber { info: GrabberInfo::default(), color: [1.0, 0.0, 0.0], alive: Arc::clone(&a) })
            as Box<dyn Grabber>)
    });
    (f, alive)
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
    spawn_with(cfg, link, path, red_screen().0)
}

fn spawn_with(
    cfg: Config,
    link: FakeLink,
    path: Option<std::path::PathBuf>,
    grabbers: GrabberFactory,
) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    spawn_full(cfg, link, path, grabbers, Arc::new(|_| Ok(())))
}

fn spawn_full(
    cfg: Config,
    link: FakeLink,
    path: Option<std::path::PathBuf>,
    grabbers: GrabberFactory,
    autostart: AutostartFn,
) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    let notices = Arc::new(Mutex::new(Vec::new()));
    let n = Arc::clone(&notices);
    let w = Worker::spawn(
        cfg,
        Vec::new(),
        path,
        Box::new(link),
        grabbers,
        autostart,
        Arc::new(move |x: Notice| n.lock().unwrap().push(x)),
        fast(),
    );
    (w, notices)
}

fn ambilight_cfg() -> Config {
    let mut c = saved_cfg();
    c.app.mode = Mode::Ambilight;
    c.ambilight.smoothing = 0;
    c
}

fn wait_calls(link: &FakeLink, what: &str, pred: impl Fn(&[Call]) -> bool) -> Vec<Call> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let calls = link.calls();
        if pred(&calls) {
            return calls;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {calls:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
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

// ---------- streaming ----------

#[test]
fn ambilight_streams_captured_frames() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "stream start", |c| c.contains(&Call::StartStream(IP, 60)));
    wait_calls(&link, "red frame", |c| c.contains(&Call::Frame(vec![Rgb::new(255, 0, 0); 60])));
    let s = wait_for(&w, "capture status", |s| s.stream.capture.as_ref().is_some_and(|c| c.active));
    assert!(s.stream.streaming);
}

#[test]
fn an_unchanged_screen_is_resent_once_per_second() {
    let link = FakeLink::with_strip();
    let (_w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    let before = link.frames().len();
    std::thread::sleep(Duration::from_millis(2200));
    let sent = link.frames().len() - before;
    assert!((1..=3).contains(&sent), "{sent} frames for a still screen in 2.2 s");
}

#[test]
fn switching_to_static_stops_capture_and_the_stream() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(ambilight_cfg(), link.clone(), None, grabbers);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert_eq!(alive.load(Ordering::SeqCst), 1);
    w.send(Command::SetMode { mode: Mode::Static });
    let full = LightChange::full(&Config::default().light);
    wait_calls(&link, "stop", |c| c.contains(&Call::StopStream(IP, full, false)));
    wait_for(&w, "capture stopped", |s| s.stream.capture.is_none() && !s.stream.streaming);
    assert_eq!(alive.load(Ordering::SeqCst), 0, "the grabber is released");
    let frames = link.frames().len();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(link.frames().len(), frames, "no frames after the stop");
}

#[test]
fn stream_failure_goes_offline_and_resumes() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    link.0.lock().unwrap().fail_next_tick = true;
    wait_for(&w, "offline", |s| s.connection == Connection::Offline && s.stream.capture.is_none());
    wait_calls(&link, "second start", |c| c.iter().filter(|x| matches!(x, Call::StartStream(..))).count() == 2);
    wait_for(&w, "streaming again", |s| s.connection == Connection::Ready && s.stream.streaming);
}

#[test]
fn highlight_from_static_sends_markers_then_returns_to_static() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(saved_cfg(), link.clone(), None, grabbers);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    w.send(Command::Highlight { target: HighlightTarget::Led(5) });
    let calls = wait_calls(&link, "marker frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert!(calls.contains(&Call::StartStream(IP, 60)));
    let marker = link.frames()[0].clone();
    assert_eq!(marker[5], Rgb::new(255, 255, 255));
    assert_eq!(alive.load(Ordering::SeqCst), 0, "no capture for a highlight in static mode");
    // Nobody renews the highlight (the window was closed): back to static after 3 s.
    let full = LightChange::full(&Config::default().light);
    let deadline = Instant::now() + Duration::from_secs(6);
    while !link.calls().contains(&Call::StopStream(IP, full, false)) {
        assert!(Instant::now() < deadline, "{:?}", link.calls());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn led_count_change_while_streaming_resizes_frames() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(f) if f.len() == 60)));
    w.send(Command::SetLedCount { count: 40 });
    wait_calls(&link, "40-LED frame", |c| c.iter().any(|x| matches!(x, Call::Frame(f) if f.len() == 40)));
}

#[test]
fn exit_during_ambilight_leaves_direct_mode_with_the_static_colour() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(ambilight_cfg(), link.clone(), None, grabbers);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert!(w.send(Command::Exit));
    assert!(w.join(Duration::from_secs(3)), "worker did not stop");
    let full = LightChange::full(&Config::default().light);
    assert!(link.calls().contains(&Call::StopStream(IP, full, true)), "{:?}", link.calls());
    assert_eq!(alive.load(Ordering::SeqCst), 0);
}

// ---------- Windows lifecycle ----------

#[test]
fn startup_syncs_autostart_with_the_setting() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);
    let mut cfg = saved_cfg();
    cfg.startup.autostart = false;
    let (w, _) = spawn_full(
        cfg,
        FakeLink::with_strip(),
        None,
        red_screen().0,
        Arc::new(move |on| {
            s.lock().unwrap().push(on);
            Ok(())
        }),
    );
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    assert_eq!(*seen.lock().unwrap(), vec![false]);
    w.send(Command::SetAutostart { enabled: true });
    let deadline = Instant::now() + Duration::from_secs(3);
    while seen.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*seen.lock().unwrap(), vec![false, true]);
}

#[test]
fn a_failing_autostart_is_reported_in_the_snapshot() {
    let (w, _) = spawn_full(
        saved_cfg(),
        FakeLink::with_strip(),
        None,
        red_screen().0,
        Arc::new(|_| Err("access denied".into())),
    );
    wait_for(&w, "autostart error", |s| s.errors.iter().any(|e| e.contains("autostart")));
    wait_for(&w, "still connects", |s| s.connection == Connection::Ready);
}

#[test]
fn send_and_wait_returns_once_the_command_is_handled() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    assert!(w.handle().send_and_wait(Command::Darken { reason: Dark::Sleep }, Duration::from_secs(2)));
    assert!(link.calls().contains(&Call::Apply(IP, LightChange::off())), "{:?}", link.calls());
}

#[test]
fn send_and_wait_gives_up_on_a_busy_worker() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    std::thread::sleep(Duration::from_millis(250));
    link.0.lock().unwrap().apply_delay = Duration::from_millis(600);
    let h = w.handle();
    assert!(!h.send_and_wait(Command::SetPower { on: false }, Duration::from_millis(100)));
    assert!(
        h.send_and_wait(Command::SetPower { on: true }, Duration::from_secs(3)),
        "later commands are still awaited"
    );
}

#[test]
fn locking_in_ambilight_stops_capture_and_unlocking_restarts_it() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(ambilight_cfg(), link.clone(), None, grabbers);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert!(w.handle().send_and_wait(Command::Darken { reason: Dark::Locked }, Duration::from_secs(2)));
    assert!(link.calls().contains(&Call::StopStream(IP, LightChange::off(), false)));
    assert_eq!(alive.load(Ordering::SeqCst), 0);
    w.send(Command::Undarken { reason: Dark::Locked });
    wait_calls(&link, "second start", |c| c.iter().filter(|x| matches!(x, Call::StartStream(..))).count() == 2);
}
