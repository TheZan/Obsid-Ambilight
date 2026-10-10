//! `UdpLink` against a fake strip on 127.0.0.1.

use std::net::{Ipv4Addr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use obsid_ambilight::app::link::UdpLink;
use obsid_ambilight::app::worker::DeviceLink;
use obsid_ambilight::protocol::Rgb;
use serde_json::Value;

const TOKEN: &str = "1420cc0cfd377cf441181585db77611c";

/// Fake strip: answers the handshake with a token, acknowledges tokened
/// commands, stays silent on `update_leds`. Counts handshakes.
struct FakeStrip {
    port: u16,
    handshakes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl FakeStrip {
    fn start() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let port = socket.local_addr().unwrap().port();
        let handshakes = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (h, st) = (Arc::clone(&handshakes), Arc::clone(&stop));
        thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !st.load(Ordering::SeqCst) {
                let Ok((n, from)) = socket.recv_from(&mut buf) else { continue };
                let v: Value = serde_json::from_slice(&buf[..n]).unwrap();
                let id = v["id"].as_u64().unwrap();
                let reply = match v["method"].as_str().unwrap() {
                    "udp_sess_new" => {
                        h.fetch_add(1, Ordering::SeqCst);
                        format!(r#"{{"id":{id},"method":"udp_sess_token","params":{{"token":"{TOKEN}"}}}}"#)
                    }
                    "update_leds" => continue,
                    _ => format!(r#"{{"id":{id}, "result":["ok"], "token":"{TOKEN}"}}"#),
                };
                socket.send_to(format!("{reply}\r\n").as_bytes(), from).unwrap();
            }
        });
        Self { port, handshakes, stop }
    }
}

impl Drop for FakeStrip {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[test]
fn a_connection_test_keeps_a_running_stream() {
    let strip = FakeStrip::start();
    let mut link = UdpLink::with_port(strip.port);
    link.start_stream(Ipv4Addr::LOCALHOST, 4).unwrap();
    assert_eq!(strip.handshakes.load(Ordering::SeqCst), 1);

    link.handshake(Ipv4Addr::LOCALHOST).unwrap();

    assert_eq!(strip.handshakes.load(Ordering::SeqCst), 1, "the stream session is reused");
    link.send_frame(&[Rgb::new(1, 2, 3); 4]).unwrap();
    link.stream_tick().unwrap();
}

#[test]
fn a_connection_test_without_a_stream_opens_a_session() {
    let strip = FakeStrip::start();
    let mut link = UdpLink::with_port(strip.port);
    link.handshake(Ipv4Addr::LOCALHOST).unwrap();
    link.handshake(Ipv4Addr::LOCALHOST).unwrap();
    assert_eq!(strip.handshakes.load(Ordering::SeqCst), 2, "each test is a fresh handshake");
}
