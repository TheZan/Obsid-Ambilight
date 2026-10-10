use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{CaptureStatus, Connection, Dark};
use obsid_ambilight::config::{Config, Mode};
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::icons::{IconKind, app_icon, kind_for, render};

fn px(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * size + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

fn near(a: [u8; 4], rgb: [u8; 3], tol: u8) -> bool {
    a[3] > 200 && a.iter().zip(rgb).all(|(&x, y)| x.abs_diff(y) <= tol)
}

/// At 32 px: the left side of the monitor frame, the dark screen, and the
/// status dot at the top right corner.
const FRAME: (u32, u32) = (5, 13);
const SCREEN: (u32, u32) = (16, 12);
const DOT: (u32, u32) = (26, 6);
const SCREEN_RGB: [u8; 3] = [16, 22, 38];

fn is_amber(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] > 140 && p[1] < 210 && p[2] < 110
}

fn is_red(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 130 && p[2] < 130
}

#[test]
fn icons_come_in_any_requested_size() {
    for size in [16, 20, 24, 32, 40, 64, 96] {
        for kind in [IconKind::Lit(Rgb::new(1, 2, 3)), IconKind::Off, IconKind::Ambilight { warn: true }] {
            assert_eq!(render(kind, size).len(), (size * size * 4) as usize, "{kind:?} at {size}");
        }
    }
}

#[test]
fn static_icon_frame_takes_the_strip_colour() {
    for c in [Rgb::new(40, 120, 230), Rgb::new(230, 40, 60)] {
        let img = render(IconKind::Lit(c), 32);
        assert!(
            near(px(&img, 32, FRAME.0, FRAME.1), [c.r, c.g, c.b], 12),
            "{c:?}: {:?}",
            px(&img, 32, FRAME.0, FRAME.1)
        );
        assert!(near(px(&img, 32, SCREEN.0, SCREEN.1), SCREEN_RGB, 12), "screen stays dark");
        assert_eq!(px(&img, 32, 0, 0)[3], 0, "corners are transparent");
    }
}

#[test]
fn off_icon_is_a_grey_monitor_without_a_dot() {
    let img = render(IconKind::Off, 32);
    let f = px(&img, 32, FRAME.0, FRAME.1);
    assert!(f[3] > 200 && f[0].abs_diff(f[2]) < 40 && f[0] > 100, "grey frame: {f:?}");
    assert!(near(px(&img, 32, SCREEN.0, SCREEN.1), SCREEN_RGB, 12));
    let d = px(&img, 32, DOT.0, DOT.1);
    assert!(!is_amber(d) && !is_red(d), "{d:?}");
}

#[test]
fn status_dots_are_amber_while_connecting_and_red_offline() {
    let connecting = render(IconKind::Connecting, 32);
    let offline = render(IconKind::Offline, 32);
    assert!(is_amber(px(&connecting, 32, DOT.0, DOT.1)), "{:?}", px(&connecting, 32, DOT.0, DOT.1));
    assert!(is_red(px(&offline, 32, DOT.0, DOT.1)), "{:?}", px(&offline, 32, DOT.0, DOT.1));
    for size in [16, 20, 24] {
        assert_ne!(render(IconKind::Connecting, size), render(IconKind::Offline, size), "{size}");
    }
}

#[test]
fn ambilight_warning_adds_an_amber_dot() {
    let plain = render(IconKind::Ambilight { warn: false }, 32);
    let warned = render(IconKind::Ambilight { warn: true }, 32);
    assert!(!is_amber(px(&plain, 32, DOT.0 + 1, DOT.1)) || px(&plain, 32, DOT.0 + 1, DOT.1)[3] < 200);
    assert!(is_amber(px(&warned, 32, DOT.0, DOT.1)), "{:?}", px(&warned, 32, DOT.0, DOT.1));
    assert_eq!(px(&plain, 32, FRAME.0, FRAME.1), px(&warned, 32, FRAME.0, FRAME.1), "frame untouched");
    assert_ne!(plain, warned);
}

#[test]
fn app_icon_is_a_square_opaque_image() {
    let (rgba, size) = app_icon();
    assert_eq!(size, 256);
    assert_eq!(rgba.len(), 256 * 256 * 4);
    assert_eq!(px(&rgba, 256, 128, 128)[3], 255);
}

#[test]
fn icon_kind_follows_connection_and_power() {
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false);
    s.connection = Connection::Ready;
    assert_eq!(kind_for(&s), IconKind::Lit(s.config.light.color));
    s.config.light.power = false;
    assert_eq!(kind_for(&s), IconKind::Off);
    s.connection = Connection::Connecting;
    assert_eq!(kind_for(&s), IconKind::Connecting);
    s.connection = Connection::NoDevice;
    s.searching = true;
    assert_eq!(kind_for(&s), IconKind::Connecting);
    s.searching = false;
    assert_eq!(kind_for(&s), IconKind::Offline);
    s.connection = Connection::Offline;
    assert_eq!(kind_for(&s), IconKind::Offline);
}

#[test]
fn icon_kind_shows_ambilight_and_unavailable_capture() {
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false);
    s.connection = Connection::Ready;
    s.config.app.mode = Mode::Ambilight;
    assert_eq!(kind_for(&s), IconKind::Ambilight { warn: false });
    s.stream.capture = Some(CaptureStatus { active: false, error: Some("lost".into()), ..CaptureStatus::default() });
    assert_eq!(kind_for(&s), IconKind::Ambilight { warn: true });
    s.config.light.power = false;
    assert_eq!(kind_for(&s), IconKind::Off);
}

#[test]
fn a_dark_strip_shows_the_off_icon() {
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false);
    s.connection = Connection::Ready;
    s.dark = vec![Dark::Locked];
    assert_eq!(kind_for(&s), IconKind::Off);
    s.config.app.mode = Mode::Ambilight;
    assert_eq!(kind_for(&s), IconKind::Off);
}
