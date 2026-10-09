use std::net::Ipv4Addr;
use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{Connection, DeviceInfo};
use obsid_ambilight::config::Config;
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::settings::diagnostics::summary;

#[test]
fn summary_lists_key_facts_without_secrets() {
    let mut s = Controller::new(
        Config::default(),
        vec!["w".into()],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(true);
    s.connection = Connection::Ready;
    s.device = Some(DeviceInfo {
        id: "dcda0cd778bc".into(),
        name: String::new(),
        ip: Ipv4Addr::new(192, 168, 1, 85),
        fw_ver: Some("36".into()),
    });
    s.errors = vec!["timeout".into()];
    let text = summary(&s);
    for needle in
        ["Obsid Ambilight", env!("CARGO_PKG_VERSION"), "192.168.1.85", "dcda0cd778bc", "fw 36", "Ready", "timeout", "w"]
    {
        assert!(text.contains(needle), "missing {needle}: {text}");
    }
    assert!(!text.to_lowercase().contains("token"));
}

#[test]
fn hex_commit_applies_only_real_changes() {
    use obsid_ambilight::protocol::Rgb;
    use obsid_ambilight::ui::settings::lighting::{HexCommit, hex_commit};
    let current = Rgb::new(0xF5, 0xD4, 0xA1);
    assert_eq!(hex_commit("#123456", current), HexCommit::Apply(Rgb::new(0x12, 0x34, 0x56)));
    assert_eq!(hex_commit(" 123456 ", current), HexCommit::Apply(Rgb::new(0x12, 0x34, 0x56)));
    assert_eq!(
        hex_commit("#f5d4a1", current),
        HexCommit::Unchanged,
        "focus loss on the same colour must not power the strip on"
    );
    assert_eq!(hex_commit("#000000", current), HexCommit::Black);
    assert_eq!(hex_commit("zz", current), HexCommit::Invalid);
}
