use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{CaptureStatus, Connection, Dark};
use obsid_ambilight::config::{Config, Mode};
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::icons::{IconKind, kind_for, render};

fn px(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * size + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

#[test]
fn lit_icon_is_a_filled_circle_of_the_colour() {
    let c = Rgb::new(200, 100, 50);
    let img = render(IconKind::Lit(c), 32);
    assert_eq!(img.len(), 32 * 32 * 4);
    assert_eq!(px(&img, 32, 16, 16), [200, 100, 50, 255]);
    assert_eq!(px(&img, 32, 0, 0)[3], 0, "corners are transparent");
}

#[test]
fn off_icon_is_a_ring() {
    let img = render(IconKind::Off, 32);
    assert_eq!(px(&img, 32, 16, 16)[3], 0, "hollow centre");
    let ring = (0..32).map(|x| px(&img, 32, x, 16)[3]).max().unwrap();
    assert_eq!(ring, 255);
}

#[test]
fn status_dots_differ_in_colour() {
    let a = render(IconKind::Connecting, 16);
    let b = render(IconKind::Offline, 16);
    assert_ne!(px(&a, 16, 8, 8), px(&b, 16, 8, 8));
    assert_eq!(px(&a, 16, 8, 8)[3], 255);
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
fn ambilight_icon_is_a_ring_of_four_colours() {
    let img = render(IconKind::Ambilight { warn: false }, 32);
    assert_eq!(px(&img, 32, 16, 16)[3], 0, "hollow centre");
    let corners = [px(&img, 32, 6, 6), px(&img, 32, 25, 6), px(&img, 32, 25, 25), px(&img, 32, 6, 25)];
    for c in corners {
        assert_eq!(c[3], 255, "{corners:?}");
    }
    for i in 0..4 {
        for j in i + 1..4 {
            assert_ne!(corners[i][..3], corners[j][..3], "quadrants {i} and {j}");
        }
    }
}

#[test]
fn ambilight_warning_adds_an_amber_dot() {
    let plain = render(IconKind::Ambilight { warn: false }, 32);
    let warned = render(IconKind::Ambilight { warn: true }, 32);
    assert_eq!(px(&warned, 32, 25, 25), [0xF2, 0xB3, 0x3D, 255]);
    assert_ne!(px(&plain, 32, 25, 25), px(&warned, 32, 25, 25));
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
