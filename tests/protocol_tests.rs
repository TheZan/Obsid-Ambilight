use std::net::{Ipv4Addr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use obsid_ambilight::protocol::device::{Chameleon2, ExitStrategy};
use obsid_ambilight::protocol::packets::{
    Call, Effect, Incoming, MAX_LEDS, MAX_PACKET, ReplyOutcome, Token, decode_leds, encode_leds, encode_request,
    parse_incoming, redact,
};
use obsid_ambilight::protocol::session::{Session, SessionConfig};
use obsid_ambilight::protocol::transport::{MockTransport, UdpTransport};
use obsid_ambilight::protocol::{ProtocolError, Rgb};
use serde_json::{Value, json};

const TOKEN: &str = "1420cc0cfd377cf441181585db77611c";

/// Behaviour of the scripted fake strip.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    /// Ignores keep-alives (to simulate a vanished device after the handshake).
    DropKeepAlive,
    /// Rejects every tokened command as if the session expired.
    ExpiredToken,
}

/// Minimal fake of the device: answers the handshake with a token (using a
/// different id, like the official example) and acknowledges tokened commands.
fn fake_device(mode: Mode) -> impl FnMut(&str) -> Vec<String> + Send + 'static {
    move |req: &str| {
        let v: Value = serde_json::from_str(req.trim()).expect("request must be JSON");
        let id = v["id"].as_u64().unwrap();
        let method = v["method"].as_str().unwrap();
        if method == "udp_sess_new" {
            return vec![format!(r#"{{"id":{},"method":"udp_sess_token","params":{{"token":"{TOKEN}"}}}}"#, id + 1)];
        }
        if v["token"] != TOKEN {
            return vec![format!(r#"{{"id":{id},"error":{{"code":-1,"message":"invalid token"}}}}"#)];
        }
        match (mode, method) {
            (Mode::DropKeepAlive, "udp_sess_keep_alive") => vec![],
            (Mode::ExpiredToken, "udp_sess_keep_alive") => vec![format!(r#"{{"id":{id},"result":["ok"]}}"#)],
            (Mode::ExpiredToken, _) => {
                vec![format!(r#"{{"id":{id},"error":{{"code":-5000,"message":"session token expired"}}}}"#)]
            }
            _ => vec![format!(r#"{{"id":{id},"result":["ok"],"token":"{TOKEN}"}}"#)],
        }
    }
}

fn open_session(mode: Mode) -> Session<MockTransport> {
    let mut s = Session::new(MockTransport::new(fake_device(mode)), SessionConfig::default());
    s.open().expect("handshake");
    s
}

fn sent_methods(s: &Session<MockTransport>) -> Vec<String> {
    s.transport()
        .sent
        .iter()
        .map(|p| serde_json::from_str::<Value>(p).unwrap()["method"].as_str().unwrap().to_owned())
        .collect()
}

// ---------- colours and LED packing ----------

#[test]
fn rgb_hex_round_trip() {
    let c = Rgb::from_hex("#F5D4A1").unwrap();
    assert_eq!(c, Rgb::new(0xF5, 0xD4, 0xA1));
    assert_eq!(c.to_string(), "#F5D4A1");
    assert_eq!(Rgb::from_hex("00ff00"), Some(Rgb::new(0, 255, 0)));
    assert_eq!(Rgb::from_hex("#12345"), None);
    assert_eq!(Rgb::from_hex("#12345G"), None);
    assert_eq!(Rgb::new(1, 0, 1).to_u24(), 65537);
}

#[test]
fn leds_are_base64_of_rgb_bytes() {
    // Base64("\xFF\x00\x00") == "/wAA", Base64("\x00\xFF\x00") == "AP8A".
    assert_eq!(encode_leds(&[Rgb::new(255, 0, 0)]).unwrap(), "/wAA");
    assert_eq!(encode_leds(&[Rgb::new(255, 0, 0), Rgb::new(0, 255, 0)]).unwrap(), "/wAAAP8A");
    assert_eq!(encode_leds(&[Rgb::new(0, 0, 0)]).unwrap(), "AAAA");
}

#[test]
fn leds_round_trip_and_limits() {
    let frame: Vec<Rgb> = (0..MAX_LEDS).map(|i| Rgb::new(i as u8, (i * 2) as u8, 255 - i as u8)).collect();
    let encoded = encode_leds(&frame).unwrap();
    assert_eq!(encoded.len(), MAX_LEDS * 4);
    assert_eq!(decode_leds(&encoded).unwrap(), frame);

    assert!(matches!(encode_leds(&[]), Err(ProtocolError::InvalidInput(_))));
    assert!(matches!(encode_leds(&vec![Rgb::BLACK; MAX_LEDS + 1]), Err(ProtocolError::InvalidInput(_))));
}

#[test]
fn full_frame_fits_one_datagram() {
    let token = Token::parse(TOKEN).unwrap();
    let call = Call::update_leds(&vec![Rgb::new(255, 255, 255); MAX_LEDS]).unwrap();
    let packet = encode_request(999_999, &call, Some(&token)).unwrap();
    assert!(packet.len() <= MAX_PACKET, "{} bytes", packet.len());
}

// ---------- request encoding ----------

#[test]
fn handshake_request_matches_spec() {
    assert_eq!(
        encode_request(1, &Call::session_new(), None).unwrap(),
        "{\"id\":1,\"method\":\"udp_sess_new\",\"params\":[]}\r\n"
    );
}

#[test]
fn tokened_request_layout() {
    let token = Token::parse(TOKEN).unwrap();
    let p = encode_request(7, &Call::set_power(true, Effect::Sudden), Some(&token)).unwrap();
    assert_eq!(
        p,
        format!("{{\"id\":7,\"method\":\"set_power\",\"params\":[\"on\",\"sudden\",0],\"token\":\"{TOKEN}\"}}\r\n")
    );
    let ka = encode_request(2, &Call::keep_alive(10), Some(&token)).unwrap();
    assert!(ka.contains(r#""params":["keeplive_interval","10"]"#), "{ka}");
    let fx = encode_request(3, &Call::activate_fx_mode("direct"), Some(&token)).unwrap();
    assert!(fx.contains(r#""params":[{"mode":"direct"}]"#), "{fx}");
    let smooth = encode_request(4, &Call::set_bright(10, Effect::Smooth(500)).unwrap(), None).unwrap();
    assert!(smooth.contains(r#""params":[10,"smooth",500]"#), "{smooth}");
}

#[test]
fn invalid_command_arguments_are_rejected() {
    assert!(Call::set_bright(0, Effect::Sudden).is_err());
    assert!(Call::set_bright(101, Effect::Sudden).is_err());
    assert!(Call::set_bright(100, Effect::Sudden).is_ok());
    assert!(Call::set_rgb(Rgb::BLACK, Effect::Sudden).is_err());
}

// ---------- response parsing ----------

#[test]
fn parses_token_reply() {
    let msg = format!("{{\"id\":2,\"method\":\"udp_sess_token\",\"params\":{{\"token\":\"{TOKEN}\"}}}}\r\n");
    match parse_incoming(&msg).unwrap() {
        Incoming::SessionToken { id, token } => {
            assert_eq!(id, Some(2));
            assert_eq!(token, Token::parse(TOKEN).unwrap());
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn parses_results_errors_and_notifications() {
    let ok = parse_incoming(&format!(r#"{{"id":1,"result":["ok"],"token":"{TOKEN}"}}"#)).unwrap();
    assert!(
        matches!(ok, Incoming::Reply { id: 1, outcome: ReplyOutcome::Ok(ref v), token: Some(_) } if v == &vec![json!("ok")])
    );

    let err = parse_incoming(r#"{"id":5,"error":{"code":-1,"message":"unsupported method"}}"#).unwrap();
    assert!(
        matches!(err, Incoming::Reply { id: 5, outcome: ReplyOutcome::Error { code: -1, ref message }, token: None } if message == "unsupported method")
    );

    let note = parse_incoming(r#"{"method":"props","params":{"power":"on"}}"#).unwrap();
    assert!(matches!(note, Incoming::Notification { ref method, .. } if method == "props"));
}

#[test]
fn rejects_malformed_input() {
    assert!(parse_incoming("not json").is_err());
    assert!(parse_incoming("[1,2]").is_err());
    assert!(parse_incoming(r#"{"result":["ok"]}"#).is_err());
    assert!(parse_incoming(r#"{"id":1}"#).is_err());
    assert!(parse_incoming(r#"{"id":1,"method":"udp_sess_token","params":{}}"#).is_err());
    assert!(parse_incoming(r#"{"id":1,"method":"udp_sess_token","params":{"token":"x\"; drop"}}"#).is_err());
}

// ---------- token secrecy ----------

#[test]
fn tokens_are_never_printed() {
    let t = Token::parse(TOKEN).unwrap();
    assert!(!format!("{t:?}").contains(TOKEN));
    assert!(!format!("{t}").contains(TOKEN));

    let packet = format!(r#"{{"id":1,"method":"set_power","params":["on"],"token":"{TOKEN}"}}"#);
    let red = redact(&packet);
    assert!(!red.contains(TOKEN));
    assert!(red.contains(r#""token":"***""#), "{red}");
    let nested = redact(&format!(r#"{{"params":{{"token": "{TOKEN}"}},"token":"{TOKEN}"}}"#));
    assert!(!nested.contains(TOKEN), "{nested}");
    assert_eq!(redact("no secrets here"), "no secrets here");
}

#[test]
fn session_errors_do_not_leak_token() {
    let mut s = open_session(Mode::ExpiredToken);
    let e = s.request(&Call::set_power(true, Effect::Sudden)).unwrap_err();
    assert!(!e.to_string().contains(TOKEN));
    assert!(!format!("{:?}", s.stats()).contains(TOKEN));
}

// ---------- session behaviour (mock transport) ----------

#[test]
fn handshake_then_keep_alive() {
    let s = open_session(Mode::Normal);
    assert!(s.is_open());
    assert_eq!(sent_methods(&s), ["udp_sess_new", "udp_sess_keep_alive"]);
    assert_eq!(s.stats().handshakes, 1);
    assert!(!s.keep_alive_due(Instant::now()));
    assert!(s.keep_alive_due(Instant::now() + Duration::from_secs(4)));
}

#[test]
fn handshake_times_out_against_silent_device() {
    let mut s = Session::new(MockTransport::silent(), SessionConfig::default());
    assert!(matches!(s.open(), Err(ProtocolError::Timeout)));
    assert!(!s.is_open());
    // Bounded retries: exactly `handshake_attempts` requests.
    assert_eq!(s.transport().sent.len(), SessionConfig::default().handshake_attempts as usize);
}

#[test]
fn commands_require_session() {
    let mut s = Session::new(MockTransport::silent(), SessionConfig::default());
    assert!(matches!(s.request(&Call::set_power(true, Effect::Sudden)), Err(ProtocolError::NoSession)));
    assert!(matches!(s.send(&Call::activate_fx_mode("direct")), Err(ProtocolError::NoSession)));
}

#[test]
fn replies_are_matched_by_id() {
    let mut s = open_session(Mode::Normal);
    // Stale replies (old id, foreign token, garbage) must be skipped.
    s.transport_mut().push_incoming(r#"{"id":1,"result":["stale"]}"#);
    s.transport_mut().push_incoming(r#"{"id":3,"result":["ok"],"token":"ffffffffffffffffffffffffffffffff"}"#);
    s.transport_mut().push_incoming("garbage");
    let result = s.request(&Call::set_power(true, Effect::Sudden)).unwrap();
    assert_eq!(result, vec![json!("ok")]);
    assert_eq!(s.stats().ignored, 3);
}

#[test]
fn missed_keep_alives_drop_the_session() {
    let mut s = Session::new(MockTransport::new(fake_device(Mode::DropKeepAlive)), SessionConfig::default());
    // The handshake succeeds; its own unanswered keep-alive is the first miss.
    s.open().unwrap();
    assert!(s.is_open());
    assert!(matches!(s.keep_alive(), Err(ProtocolError::Timeout)));
    assert!(s.is_open());
    assert!(matches!(s.keep_alive(), Err(ProtocolError::NoSession)));
    assert!(!s.is_open());
}

#[test]
fn token_error_drops_session() {
    let mut s = open_session(Mode::ExpiredToken);
    let e = s.request(&Call::set_power(true, Effect::Sudden)).unwrap_err();
    assert!(matches!(e, ProtocolError::Device { code: -5000, .. }));
    assert!(!s.is_open());
}

#[test]
fn token_error_on_frame_drops_session_and_direct_mode() {
    let mut d = device(2);
    d.enter_direct().unwrap();
    d.session_mut().transport_mut().push_incoming(r#"{"id":99,"error":{"code":-5000,"message":"session expired"}}"#);
    let e = d.send_frame(&[Rgb::BLACK; 2]).unwrap_err();
    assert!(matches!(e, ProtocolError::NoSession));
    assert!(!d.session().is_open());
    assert!(!d.is_direct());
}

#[test]
fn fire_and_forget_drains_replies() {
    let mut s = open_session(Mode::Normal);
    for _ in 0..5 {
        s.send(&Call::update_leds(&[Rgb::new(1, 2, 3)]).unwrap()).unwrap();
    }
    // Every ack was consumed immediately; nothing accumulates.
    assert_eq!(s.drain().unwrap(), 0);
}

// ---------- device layer ----------

fn device(leds: usize) -> Chameleon2<MockTransport> {
    Chameleon2::new(open_session(Mode::Normal), leds).unwrap()
}

#[test]
fn led_count_is_validated() {
    let s = || Session::new(MockTransport::silent(), SessionConfig::default());
    assert!(Chameleon2::new(s(), 0).is_err());
    assert!(Chameleon2::new(s(), MAX_LEDS + 1).is_err());
    assert!(Chameleon2::new(s(), MAX_LEDS).is_ok());
}

#[test]
fn frames_need_direct_mode_and_exact_length() {
    let mut d = device(4);
    assert!(d.send_frame(&[Rgb::BLACK; 4]).is_err());
    d.enter_direct().unwrap();
    assert!(d.is_direct());
    assert!(d.send_frame(&[Rgb::BLACK; 3]).is_err());
    d.send_frame(&[Rgb::new(9, 9, 9); 4]).unwrap();
    let last = d.session().transport().sent.last().unwrap().clone();
    let v: Value = serde_json::from_str(&last).unwrap();
    assert_eq!(v["method"], "update_leds");
    assert_eq!(decode_leds(v["params"][0].as_str().unwrap()).unwrap(), vec![Rgb::new(9, 9, 9); 4]);
}

#[test]
fn static_and_exit_sequences() {
    let mut d = device(10);
    d.apply_static(Rgb::new(10, 20, 30), 40).unwrap();
    d.enter_direct().unwrap();
    d.exit_direct(ExitStrategy::SetRgb, Rgb::new(1, 2, 3), 50).unwrap();
    assert!(!d.is_direct());
    let methods = sent_methods(d.session());
    assert_eq!(
        methods[2..],
        ["set_power", "set_rgb", "set_bright", "activate_fx_mode", "set_rgb", "set_bright"].map(String::from)
    );

    d.enter_direct().unwrap();
    d.set_power(false).unwrap();
    assert!(!d.is_direct(), "power off leaves direct mode");
}

#[test]
fn deactivate_exit_sends_deactivate_then_static() {
    let mut d = device(10);
    d.enter_direct().unwrap();
    d.exit_direct(ExitStrategy::Deactivate, Rgb::new(1, 2, 3), 50).unwrap();
    assert!(!d.is_direct());
    let sent = &d.session().transport().sent;
    let deactivate: Value = serde_json::from_str(&sent[3]).unwrap();
    assert_eq!(deactivate["method"], "deactivate_fx_mode");
    assert_eq!(deactivate["params"], json!([{ "mode": "direct" }]));
    assert_eq!(sent_methods(d.session())[4..], ["set_rgb", "set_bright"].map(String::from));
}

#[test]
fn lost_session_is_reopened_on_next_command() {
    let mut d = device(10);
    d.session_mut().close();
    d.set_power(true).unwrap();
    assert_eq!(d.session().stats().handshakes, 2);
}

// ---------- real UDP transport against a loopback fake device ----------

#[test]
fn udp_transport_handshake_over_loopback() {
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    server.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_srv = Arc::clone(&seen);
    let handle = thread::spawn(move || {
        let mut respond = fake_device(Mode::Normal);
        let mut buf = [0u8; 2048];
        // handshake, keep-alive, set_power
        for _ in 0..3 {
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            for reply in respond(&req) {
                server.send_to(reply.as_bytes(), from).unwrap();
            }
            seen_srv.lock().unwrap().push(req);
        }
    });

    let transport = UdpTransport::connect(Ipv4Addr::LOCALHOST, port).unwrap();
    let mut s = Session::new(transport, SessionConfig::default());
    s.open().unwrap();
    s.request(&Call::set_power(true, Effect::Sudden)).unwrap();
    handle.join().unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert!(seen.iter().all(|p| p.ends_with("\r\n")));
}

#[test]
fn udp_transport_times_out_without_device() {
    // Bind and drop to get a port nobody listens on; Windows may report ICMP
    // port-unreachable as a reset, which must surface as a timeout, not a crash.
    let port = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let transport = UdpTransport::connect(Ipv4Addr::LOCALHOST, port).unwrap();
    let cfg = SessionConfig { reply_timeout: Duration::from_millis(100), handshake_attempts: 2, ..Default::default() };
    let mut s = Session::new(transport, cfg);
    let start = Instant::now();
    assert!(matches!(s.open(), Err(ProtocolError::Timeout)));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn led_count_can_change_in_direct_mode_and_direct_mode_can_be_left() {
    let mut d = device(4);
    d.enter_direct().unwrap();
    assert!(d.set_led_count(0).is_err());
    assert!(d.set_led_count(MAX_LEDS + 1).is_err());
    d.set_led_count(6).unwrap();
    d.send_frame(&[Rgb::new(1, 1, 1); 6]).unwrap();
    d.leave_direct().unwrap();
    assert!(!d.is_direct());
    assert_eq!(sent_methods(d.session()).last().unwrap(), "deactivate_fx_mode");
}
