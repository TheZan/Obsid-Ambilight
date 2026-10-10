use std::time::{Duration, Instant};

use obsid_ambilight::capture::Grabber;
use obsid_ambilight::capture::dxgi::{DxgiGrabber, MAX_SMALL_WIDTH, mip_level};
use obsid_ambilight::capture::outputs;

/// `DuplicateOutput1` works only in per-monitor DPI aware processes, like the tray app.
fn dpi_aware() {
    use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    // SAFETY: plain call; failing because it is already set is harmless.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

#[test]
fn mip_level_reads_back_at_most_256_pixels_wide() {
    assert_eq!(mip_level(256), 0);
    assert_eq!(mip_level(1920), 3);
    assert_eq!(mip_level(2560), 4);
    assert_eq!(mip_level(3840), 4);
    assert_eq!(mip_level(7680), 5);
}

#[test]
#[ignore = "needs a real desktop and GPU"]
fn lists_the_monitors() {
    let monitors = outputs::list().unwrap();
    assert!(!monitors.is_empty());
    assert_eq!(monitors.iter().filter(|m| m.primary).count(), 1, "{monitors:?}");
}

#[test]
#[ignore = "needs a real desktop and GPU"]
fn captures_the_primary_monitor() {
    dpi_aware();
    let mut g = DxgiGrabber::open("").unwrap();
    assert!(g.info().width >= 640, "{:?}", g.info());
    assert!(!g.info().fallback);
    let deadline = Instant::now() + Duration::from_secs(3);
    let img = loop {
        if let Some(img) = g.grab().unwrap() {
            break img;
        }
        assert!(Instant::now() < deadline, "no frame within 3 s");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(img.width() as u32 <= MAX_SMALL_WIDTH && img.height() > 0);
}

#[test]
#[ignore = "needs a real desktop and GPU"]
fn an_unknown_monitor_falls_back_to_the_primary() {
    dpi_aware();
    let g = DxgiGrabber::open(r"\\.\NO_SUCH_DISPLAY").unwrap();
    assert!(g.info().fallback);
}
