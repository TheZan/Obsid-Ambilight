use std::net::Ipv4Addr;
use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{CaptureStatus, Connection, Dark, DeviceInfo, Snapshot};
use obsid_ambilight::config::{Config, Mode};
use obsid_ambilight::i18n::{EN, RU, dark_reasons};
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::tray::tooltip;

fn ready() -> Snapshot {
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false);
    s.connection = Connection::Ready;
    s.device =
        Some(DeviceInfo { id: "x".into(), name: String::new(), ip: Ipv4Addr::new(192, 168, 1, 85), fw_ver: None });
    s
}

#[test]
fn static_tooltip_is_unchanged() {
    assert_eq!(tooltip(&ready(), &EN), "Obsid Ambilight — Static #F5D4A1 · 40% · 192.168.1.85");
}

#[test]
fn ambilight_tooltip_shows_rate_hdr_and_address() {
    let mut s = ready();
    s.config.app.mode = Mode::Ambilight;
    s.stream.capture = Some(CaptureStatus { hdr: true, active: true, ..CaptureStatus::default() });
    assert_eq!(tooltip(&s, &EN), "Obsid Ambilight — Ambilight · 25 FPS · HDR · 192.168.1.85");
}

#[test]
fn unavailable_capture_is_mentioned() {
    let mut s = ready();
    s.config.app.mode = Mode::Ambilight;
    s.stream.capture = Some(CaptureStatus { active: false, error: Some("lost".into()), ..CaptureStatus::default() });
    assert!(tooltip(&s, &EN).contains("capture unavailable"), "{}", tooltip(&s, &EN));
}

#[test]
fn a_dark_tooltip_says_why() {
    let mut s = ready();
    s.dark = vec![Dark::Locked, Dark::DisplayOff];
    assert_eq!(tooltip(&s, &EN), "Obsid Ambilight — Off · screen locked, display off · 192.168.1.85");
}

#[test]
fn dark_reasons_are_translated() {
    assert_eq!(dark_reasons(&RU, &[Dark::Sleep]), "сон");
    assert_eq!(dark_reasons(&EN, &[]), "");
}
