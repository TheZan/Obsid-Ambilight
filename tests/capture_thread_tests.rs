use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::Image;
use obsid_ambilight::ambilight::rate::RateMeter;
use obsid_ambilight::app::state::CaptureStatus;
use obsid_ambilight::capture::thread::{Capture, CaptureSettings};
use obsid_ambilight::capture::{Grabber, GrabberFactory, GrabberInfo};
use obsid_ambilight::config::Config;
use obsid_ambilight::protocol::Rgb;

#[derive(Default)]
struct Probe {
    opened: Mutex<Vec<String>>,
    alive: AtomicUsize,
    fail_next_grab: AtomicBool,
    fail_opens: AtomicUsize,
    color: Mutex<[f32; 3]>,
}

struct FakeGrabber {
    info: GrabberInfo,
    probe: Arc<Probe>,
}

impl Grabber for FakeGrabber {
    fn info(&self) -> &GrabberInfo {
        &self.info
    }
    fn grab(&mut self) -> Result<Option<Image>, String> {
        if self.probe.fail_next_grab.swap(false, Ordering::SeqCst) {
            return Err("capture lost: test".into());
        }
        Ok(Some(Image::filled(32, 18, *self.probe.color.lock().unwrap())))
    }
}

impl Drop for FakeGrabber {
    fn drop(&mut self) {
        self.probe.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

fn factory(probe: &Arc<Probe>) -> GrabberFactory {
    let probe = Arc::clone(probe);
    Arc::new(move |monitor: &str| {
        probe.opened.lock().unwrap().push(monitor.to_owned());
        if probe.fail_opens.load(Ordering::SeqCst) > 0 {
            probe.fail_opens.fetch_sub(1, Ordering::SeqCst);
            return Err("no monitor".into());
        }
        probe.alive.fetch_add(1, Ordering::SeqCst);
        let info = GrabberInfo {
            monitor: if monitor.is_empty() { r"\\.\DISPLAY1".into() } else { monitor.to_owned() },
            adapter: "Fake GPU".into(),
            width: 2560,
            height: 1440,
            hdr: true,
            sdr_white_nits: 200.0,
            fallback: monitor == "missing",
        };
        Ok(Box::new(FakeGrabber { info, probe: Arc::clone(&probe) }) as Box<dyn Grabber>)
    })
}

fn settings() -> CaptureSettings {
    let mut s = CaptureSettings::from_config(&Config::default());
    s.fps = 30;
    s.pipeline.smoothing = 0;
    s
}

fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(x) = f() {
            return x;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn red_probe() -> Arc<Probe> {
    let p = Arc::new(Probe::default());
    *p.color.lock().unwrap() = [1.0, 0.0, 0.0];
    p
}

#[test]
fn produces_led_colours_from_grabbed_frames() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    let frame = wait("frame", || c.take_frame());
    assert_eq!(frame, vec![Rgb::new(255, 0, 0); 60]);
    let s: CaptureStatus = wait("status", || Some(c.status()).filter(|s| s.active && s.fps > 0.0));
    assert_eq!((s.monitor.as_str(), s.adapter.as_str(), s.width, s.height), (r"\\.\DISPLAY1", "Fake GPU", 2560, 1440));
    assert!(s.hdr);
    assert_eq!(s.sdr_white_nits, 200.0);
    assert_eq!(s.error, None);
}

#[test]
fn follows_settings_updates() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("first frame", || c.take_frame());
    let mut s = settings();
    s.pipeline.led_count = 10;
    s.pipeline.segments.retain(|seg| seg.hi() < 10);
    c.update(s);
    wait("10-LED frame", || c.take_frame().filter(|f| f.len() == 10));
}

#[test]
fn reopens_after_a_lost_grabber() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("first frame", || c.take_frame());
    probe.fail_next_grab.store(true, Ordering::SeqCst);
    wait("reopen", || (probe.opened.lock().unwrap().len() == 2).then_some(()));
    wait("frames again", || c.take_frame());
    assert_eq!(probe.alive.load(Ordering::SeqCst), 1, "the lost grabber was dropped");
}

#[test]
fn factory_failure_is_reported_and_retried() {
    let probe = red_probe();
    probe.fail_opens.store(1, Ordering::SeqCst);
    let c = Capture::start(factory(&probe), settings());
    let s = wait("error", || Some(c.status()).filter(|s| s.error.is_some()));
    assert!(!s.active);
    wait("recovery", || Some(c.status()).filter(|s| s.active && s.error.is_none()));
    wait("frame", || c.take_frame());
}

#[test]
fn changing_the_monitor_reopens_the_grabber() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("first frame", || c.take_frame());
    c.update(CaptureSettings { monitor: r"\\.\DISPLAY2".into(), ..settings() });
    wait("second open", || {
        (probe.opened.lock().unwrap().last().map(String::as_str) == Some(r"\\.\DISPLAY2")).then_some(())
    });
    wait("status", || Some(c.status()).filter(|s| s.monitor == r"\\.\DISPLAY2"));
}

#[test]
fn a_fallback_is_reported_as_a_warning() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), CaptureSettings { monitor: "missing".into(), ..settings() });
    let s = wait("status", || Some(c.status()).filter(|s| s.active));
    assert!(s.error.as_deref().is_some_and(|e| e.contains("missing")), "{s:?}");
}

#[test]
fn dropping_stops_the_thread_and_releases_the_grabber() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("frame", || c.take_frame());
    drop(c);
    assert_eq!(probe.alive.load(Ordering::SeqCst), 0);
}

#[test]
fn rate_meter_measures_recent_ticks() {
    let t0 = Instant::now();
    let mut m = RateMeter::new();
    assert_eq!(m.rate(t0), 0.0);
    for i in 0..26 {
        m.tick(t0 + Duration::from_millis(40 * i));
    }
    let r = m.rate(t0 + Duration::from_secs(1));
    assert!((r - 25.0).abs() < 0.5, "{r}");
    assert_eq!(m.rate(t0 + RateMeter::WINDOW * 3), 0.0, "old ticks do not count");
}
