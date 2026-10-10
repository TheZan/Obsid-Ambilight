//! Capture thread: grabs frames at the Ambilight frame rate, runs the colour
//! pipeline and leaves the newest LED colours in a slot for the device worker.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use super::{Grabber, GrabberFactory};
use crate::ambilight::image::Image;
use crate::ambilight::pipeline::{Pipeline, Settings};
use crate::ambilight::rate::RateMeter;
use crate::app::state::CaptureStatus;
use crate::config::Config;
use crate::protocol::Rgb;

/// Pause before reopening a grabber that failed or could not be opened.
pub const REOPEN_DELAY: Duration = Duration::from_secs(1);
/// Panics in a row after which the thread gives up.
pub const MAX_PANICS: u32 = 3;
/// Longest single sleep, so a stop request is noticed quickly.
const SLICE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq)]
pub struct CaptureSettings {
    pub monitor: String,
    pub fps: u32,
    pub pipeline: Settings,
}

impl CaptureSettings {
    pub fn from_config(cfg: &Config) -> Self {
        Self { monitor: cfg.ambilight.monitor.clone(), fps: cfg.ambilight.fps, pipeline: Settings::from_config(cfg) }
    }
}

struct Shared {
    settings: Mutex<CaptureSettings>,
    frame: Mutex<Option<Vec<Rgb>>>,
    status: Mutex<CaptureStatus>,
    stop: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A running capture thread; dropping it stops the thread.
pub struct Capture {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    pub fn start(factory: GrabberFactory, settings: CaptureSettings) -> Capture {
        let shared = Arc::new(Shared {
            settings: Mutex::new(settings),
            frame: Mutex::new(None),
            status: Mutex::new(CaptureStatus::default()),
            stop: AtomicBool::new(false),
        });
        let s = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("capture".into())
            .spawn(move || supervise(&s, &factory))
            .expect("spawn capture thread");
        Capture { shared, thread: Some(thread) }
    }

    pub fn update(&self, settings: CaptureSettings) {
        *lock(&self.shared.settings) = settings;
    }

    /// The newest LED colours since the last call.
    pub fn take_frame(&self) -> Option<Vec<Rgb>> {
        lock(&self.shared.frame).take()
    }

    pub fn status(&self) -> CaptureStatus {
        lock(&self.shared.status).clone()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Sleeps up to `d`, returning early when asked to stop.
fn pause(shared: &Shared, d: Duration) {
    let until = Instant::now() + d;
    while !shared.stop.load(Ordering::SeqCst) {
        let Some(rest) = until.checked_duration_since(Instant::now()) else { return };
        thread::sleep(rest.min(SLICE));
    }
}

fn set_unavailable(shared: &Shared, e: String) {
    let mut st = lock(&shared.status);
    st.active = false;
    st.error = Some(e);
}

fn supervise(shared: &Shared, factory: &GrabberFactory) {
    let mut panics = 0;
    while !shared.stop.load(Ordering::SeqCst) {
        match catch_unwind(AssertUnwindSafe(|| run(shared, factory))) {
            Ok(()) => return,
            Err(_) => {
                panics += 1;
                error!(panics, "capture thread panicked");
                if panics > MAX_PANICS {
                    set_unavailable(shared, "screen capture stopped after repeated internal errors".into());
                    return;
                }
                set_unavailable(
                    shared,
                    format!("screen capture restarted after an internal error ({panics}/{MAX_PANICS})"),
                );
                pause(shared, REOPEN_DELAY);
            }
        }
    }
}

fn run(shared: &Shared, factory: &GrabberFactory) {
    // The grabber and the monitor setting it was opened for.
    let mut grabber: Option<(Box<dyn Grabber>, String)> = None;
    let mut pipeline = Pipeline::new();
    let mut last: Option<Image> = None;
    let mut rate = RateMeter::new();
    let mut next = Instant::now();
    while !shared.stop.load(Ordering::SeqCst) {
        let settings = lock(&shared.settings).clone();
        if grabber.as_ref().is_some_and(|(_, m)| *m != settings.monitor) {
            grabber = None;
            last = None;
        }
        if grabber.is_none() {
            match factory(&settings.monitor) {
                Ok(g) => {
                    let i = g.info().clone();
                    info!(monitor = %i.monitor, adapter = %i.adapter, hdr = i.hdr, "capture started");
                    *lock(&shared.status) = CaptureStatus {
                        monitor: i.monitor,
                        adapter: i.adapter,
                        width: i.width,
                        height: i.height,
                        hdr: i.hdr,
                        sdr_white_nits: i.sdr_white_nits,
                        active: true,
                        error: i
                            .fallback
                            .then(|| format!("monitor {} not found, capturing the primary one", settings.monitor)),
                        ..CaptureStatus::default()
                    };
                    grabber = Some((g, settings.monitor.clone()));
                }
                Err(e) => {
                    warn!(%e, "screen capture unavailable");
                    set_unavailable(shared, e);
                    pause(shared, REOPEN_DELAY);
                    continue;
                }
            }
        }
        let (g, _) = grabber.as_mut().expect("grabber just opened");
        match g.grab() {
            Ok(Some(img)) => last = Some(img),
            Ok(None) => {}
            Err(e) => {
                warn!(%e, "screen capture lost, reopening");
                set_unavailable(shared, e);
                grabber = None;
                pause(shared, REOPEN_DELAY);
                continue;
            }
        }
        let now = Instant::now();
        if let Some(img) = &last {
            let colors = pipeline.process(img, &settings.pipeline, now);
            *lock(&shared.frame) = Some(colors);
            rate.tick(now);
            let bars = pipeline.bars();
            let mut st = lock(&shared.status);
            st.fps = rate.rate(now);
            st.bars = [bars.vertical as f32 / img.height() as f32, bars.horizontal as f32 / img.width() as f32];
        }
        // Sleep to the next frame time; the next grab takes whatever the
        // desktop accumulated meanwhile, so a 240 Hz screen costs `fps` copies.
        next += Duration::from_secs(1) / settings.fps.max(1);
        let now = Instant::now();
        if next < now {
            next = now;
        }
        pause(shared, next - now);
    }
}
