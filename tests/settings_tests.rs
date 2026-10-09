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
