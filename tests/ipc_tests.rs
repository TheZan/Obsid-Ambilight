use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{Snapshot, Tab};
use obsid_ambilight::config::Config;
use obsid_ambilight::ipc::pipe::{self, Client};
use obsid_ambilight::ipc::{Request, Response, decode, encode};
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;

static N: AtomicU32 = AtomicU32::new(0);

fn unique_pipe() -> String {
    format!(r"\\.\pipe\obsid-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn snapshot() -> Snapshot {
    Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false)
}

#[test]
fn messages_round_trip_as_single_json_lines() {
    let req = Request::Command { command: Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: None } };
    let line = encode(&req);
    assert!(line.ends_with('\n') && !line.trim_end().contains('\n'));
    assert_eq!(decode::<Request>(&line).unwrap(), req);

    let resp = Response::State { snapshot: Box::new(snapshot()) };
    assert_eq!(decode::<Response>(&encode(&resp)).unwrap(), resp);
    assert_eq!(
        decode::<Request>(&encode(&Request::OpenSettings { tab: Some(Tab::Lighting) })).unwrap(),
        Request::OpenSettings { tab: Some(Tab::Lighting) }
    );
}

#[test]
fn garbage_is_rejected() {
    assert!(decode::<Request>("not json").is_err());
    assert!(decode::<Request>("{\"req\":\"nope\"}").is_err());
}

#[test]
fn pipe_name_is_per_user_and_session() {
    let name = pipe::pipe_name().unwrap();
    assert!(name.starts_with(r"\\.\pipe\ObsidAmbilight-"), "{name}");
    assert!(name.contains("S-1-"), "{name}");
}

#[test]
fn request_response_over_a_real_pipe() {
    let name = unique_pipe();
    let snap = snapshot();
    let s2 = snap.clone();
    pipe::serve(
        name.clone(),
        Arc::new(move |req| match req {
            Request::GetState => Response::State { snapshot: Box::new(s2.clone()) },
            Request::Command { .. } => Response::Ok,
            Request::OpenSettings { .. } => Response::Error { message: "no".into() },
            Request::TakeTab => Response::Tab { tab: Some(Tab::Lighting) },
        }),
    )
    .unwrap();

    let mut a = Client::connect(&name, Duration::from_secs(2)).unwrap();
    let mut b = Client::connect(&name, Duration::from_secs(2)).unwrap();
    assert_eq!(a.request(&Request::GetState).unwrap(), Response::State { snapshot: Box::new(snap) });
    assert_eq!(b.request(&Request::Command { command: Command::Discover }).unwrap(), Response::Ok);
    assert_eq!(a.request(&Request::OpenSettings { tab: None }).unwrap(), Response::Error { message: "no".into() });
    assert_eq!(b.request(&Request::TakeTab).unwrap(), Response::Tab { tab: Some(Tab::Lighting) });
}

#[test]
fn connecting_to_a_missing_pipe_fails_fast() {
    let start = std::time::Instant::now();
    assert!(Client::connect(&unique_pipe(), Duration::from_millis(300)).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn request_with_timeout_gives_up_on_a_silent_server() {
    let name = unique_pipe();
    pipe::serve(
        name.clone(),
        Arc::new(|_| {
            std::thread::sleep(Duration::from_secs(5));
            Response::Ok
        }),
    )
    .unwrap();
    let start = std::time::Instant::now();
    let r = pipe::request_with_timeout(&name, Request::GetState, Duration::from_millis(500));
    assert!(r.is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn tab_requests_round_trip_and_are_taken_once() {
    use obsid_ambilight::ipc::TabRequest;
    assert_eq!(decode::<Request>(&encode(&Request::TakeTab)).unwrap(), Request::TakeTab);
    let resp = Response::Tab { tab: Some(Tab::Lighting) };
    assert_eq!(decode::<Response>(&encode(&resp)).unwrap(), resp);

    let pending = TabRequest::default();
    let shared = pending.clone();
    assert_eq!(pending.take(), None);
    shared.set(Tab::Device);
    shared.set(Tab::Lighting);
    assert_eq!(pending.take(), Some(Tab::Lighting), "latest request wins");
    assert_eq!(pending.take(), None, "a request is delivered once");
}
