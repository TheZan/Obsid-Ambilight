use std::net::Ipv4Addr;
use std::time::Duration;

use obsid_ambilight::discovery::{DiscoveredDevice, SEARCH_REQUEST, parse_response};
use obsid_ambilight::reconnect::{Backoff, Resolution, resolve};

const SRC: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 50);

/// Search response in the format of Yeelight's specification, adapted to a
/// Chameleon2 (header values carry a leading space, as the SignalRGB driver shows).
fn chameleon_response(id: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\n\
         Cache-Control: max-age=3600\r\n\
         Date: \r\n\
         Ext: \r\n\
         Location: yeelight://192.168.1.50:55443\r\n\
         Server: POSIX UPnP/1.0 YGLC/1\r\n\
         id: {id}\r\n\
         model: Chameleon2\r\n\
         fw_ver: 18\r\n\
         support: get_prop set_power set_bright set_rgb udp_sess_new udp_sess_keep_alive activate_fx_mode update_leds\r\n\
         power: on\r\n\
         bright: 40\r\n\
         name: Desk\r\n"
    )
}

#[test]
fn search_request_targets_yeelight() {
    assert!(SEARCH_REQUEST.starts_with("M-SEARCH * HTTP/1.1\r\n"));
    assert!(SEARCH_REQUEST.contains("HOST: 239.255.255.250:1982\r\n"));
    assert!(SEARCH_REQUEST.contains("MAN: \"ssdp:discover\"\r\n"));
    assert!(SEARCH_REQUEST.contains("ST: wifi_bulb\r\n"));
}

#[test]
fn parses_chameleon2_response() {
    let d = parse_response(&chameleon_response("0x0000000012345678"), SRC).unwrap();
    assert_eq!(d.id, "0x0000000012345678");
    assert_eq!(d.model, "Chameleon2");
    assert_eq!(d.ip, SRC);
    assert_eq!(d.location, "yeelight://192.168.1.50:55443");
    assert_eq!(d.name, "Desk");
    assert_eq!(d.display_name(), "Desk");
    assert_eq!(d.fw_ver.as_deref(), Some("18"));
    assert_eq!(d.power.as_deref(), Some("on"));
    assert!(d.is_chameleon2());
    assert!(d.supports("update_leds"));
    assert!(d.supports_rgbic_udp());
    assert!(!d.supports("update"), "support matching must be by whole method name");
}

#[test]
fn parses_spec_notify_for_other_model() {
    let text = "NOTIFY * HTTP/1.1\r\nHost: 239.255.255.250:1982\r\nLocation: yeelight://192.168.31.253:55443\r\n\
                id: 0x000000000649f51e\r\nmodel: lamp19\r\nsupport: get_prop set_power udp_sess_new\r\nname: \r\n";
    let d = parse_response(text, Ipv4Addr::new(192, 168, 31, 253)).unwrap();
    assert_eq!(d.model, "lamp19");
    assert!(!d.is_chameleon2());
    assert!(!d.supports_rgbic_udp());
    assert_eq!(d.display_name(), "lamp19", "empty name falls back to model");
}

#[test]
fn header_names_are_case_insensitive_and_lf_only_is_accepted() {
    let text =
        chameleon_response("0x1").replace("\r\n", "\n").replace("Location", "LOCATION").replace("model", "Model");
    let d = parse_response(&text, SRC).unwrap();
    assert!(d.is_chameleon2());
}

#[test]
fn rejects_non_yeelight_messages() {
    assert!(parse_response("", SRC).is_none());
    assert!(parse_response(SEARCH_REQUEST, SRC).is_none(), "our own request is not a device");
    let upnp = chameleon_response("0x1").replace("yeelight://", "http://");
    assert!(parse_response(&upnp, SRC).is_none());
    let no_id = chameleon_response("");
    assert!(parse_response(&no_id, SRC).is_none());
    let no_model = chameleon_response("0x1").replace("model: Chameleon2\r\n", "");
    assert!(parse_response(&no_model, SRC).is_none());
}

// ---------- resolving a saved device ----------

fn dev(id: &str, ip: [u8; 4]) -> DiscoveredDevice {
    let mut d = parse_response(&chameleon_response(id), SRC).unwrap();
    d.ip = Ipv4Addr::from(ip);
    d
}

#[test]
fn saved_ip_with_matching_id_wins_without_discovery() {
    let mut discovered = false;
    let r = resolve(
        "0xA",
        Some(Ipv4Addr::new(10, 0, 0, 5)),
        |ip| Some(dev("0xA", ip.octets())),
        || {
            discovered = true;
            vec![]
        },
    )
    .unwrap();
    assert!(matches!(r, Resolution::SavedIp(_)));
    assert!(!r.ip_changed(Some(Ipv4Addr::new(10, 0, 0, 5))));
    assert!(!discovered);
}

#[test]
fn saved_ip_taken_by_another_device_falls_back_to_discovery() {
    let r = resolve(
        "0xA",
        Some(Ipv4Addr::new(10, 0, 0, 5)),
        |ip| Some(dev("0xB", ip.octets())),
        || vec![dev("0xB", [10, 0, 0, 5]), dev("0xA", [10, 0, 0, 9])],
    )
    .unwrap();
    assert!(matches!(r, Resolution::Discovered(_)));
    assert_eq!(r.device().ip, Ipv4Addr::new(10, 0, 0, 9));
    assert!(r.ip_changed(Some(Ipv4Addr::new(10, 0, 0, 5))));
}

#[test]
fn unknown_device_is_not_resolved() {
    assert!(resolve("0xA", None, |_| None, || vec![dev("0xB", [10, 0, 0, 2])]).is_none());
}

// ---------- backoff ----------

#[test]
fn backoff_grows_is_capped_and_jittered() {
    let base = Duration::from_millis(500);
    let max = Duration::from_secs(30);
    let mut b = Backoff::with_seed(base, max, None, 42);
    for attempt in 0..12u32 {
        let exp = base.saturating_mul(1 << attempt).min(max);
        let d = b.next_delay().unwrap();
        assert!(d >= exp / 2 && d <= exp, "attempt {attempt}: {d:?} not in [{:?}, {exp:?}]", exp / 2);
    }
    b.reset();
    assert!(b.next_delay().unwrap() <= base);
}

#[test]
fn backoff_respects_attempt_limit() {
    let mut b = Backoff::with_seed(Duration::from_millis(10), Duration::from_secs(1), Some(3), 7);
    assert_eq!((0..5).filter_map(|_| b.next_delay()).count(), 3);
    assert_eq!(b.attempt(), 3);
}

#[test]
fn backoff_seeds_spread_retries() {
    let delays: Vec<_> = (1..=8u64)
        .map(|seed| {
            Backoff::with_seed(Duration::from_secs(8), Duration::from_secs(8), None, seed).next_delay().unwrap()
        })
        .collect();
    let first = delays[0];
    assert!(delays.iter().any(|&d| d != first), "jitter should differ between clients");
}
