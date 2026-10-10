use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::layout::HighlightTarget;
use obsid_ambilight::app::commands::{Command, coalesce};
use obsid_ambilight::app::controller::{
    Action, Controller, Event, HIGHLIGHT_TTL, LightChange, MAX_ERRORS, RETRY_BASE, Resolved, Target,
};
use obsid_ambilight::app::state::{Connection, Dark, DeviceInfo, TestState};
use obsid_ambilight::config::{AmbilightConfig, Config, ExitAction, Mode, Segment};
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;

const IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 85);

fn dev(id: &str, ip: Ipv4Addr) -> DeviceInfo {
    DeviceInfo { id: id.into(), name: String::new(), ip, fw_ver: Some("36".into()) }
}

fn ctrl(cfg: Config) -> Controller {
    Controller::new(cfg, Vec::new(), Backoff::with_seed(RETRY_BASE, Duration::from_secs(300), None, 7))
}

fn saved_cfg() -> Config {
    let mut c = Config::default();
    c.device.id = "dcda0cd778bc".into();
    c.device.last_ip = Some(IP);
    c
}

fn full_on(c: &Config) -> LightChange {
    LightChange::full(&c.light)
}

/// Startup with a saved strip, resolved successfully. Returns (controller, now).
fn ready() -> (Controller, Instant) {
    let now = Instant::now();
    let mut c = ctrl(saved_cfg());
    c.handle(Event::Startup, now);
    c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), now);
    (c, now)
}

#[test]
fn first_run_with_one_strip_selects_it_and_lights_it() {
    let now = Instant::now();
    let mut c = ctrl(Config::default());
    let a = c.handle(Event::Startup, now);
    assert!(a.contains(&Action::Discover));
    assert!(c.snapshot(false).searching);
    assert_eq!(c.snapshot(false).connection, Connection::NoDevice);

    let a = c.handle(Event::Discovered(Ok(vec![dev("abc", IP)])), now);
    assert!(a.contains(&Action::SaveConfig));
    assert!(a.contains(&Action::Apply { ip: IP, change: full_on(c.config()) }), "{a:?}");
    assert_eq!(c.config().device.id, "abc");
    assert_eq!(c.config().device.last_ip, Some(IP));
    let s = c.snapshot(false);
    assert_eq!(s.connection, Connection::Ready);
    assert!(!s.searching);
    assert!(s.verified);
}

#[test]
fn first_run_with_several_or_no_strips_opens_settings() {
    for found in [vec![], vec![dev("a", IP), dev("b", Ipv4Addr::new(192, 168, 1, 86))]] {
        let now = Instant::now();
        let mut c = ctrl(Config::default());
        c.handle(Event::Startup, now);
        let a = c.handle(Event::Discovered(Ok(found.clone())), now);
        assert_eq!(a, vec![Action::OpenSettings], "found {}", found.len());
        assert_eq!(c.snapshot(false).connection, Connection::NoDevice);
        assert_eq!(c.snapshot(false).discovered, found);
    }
}

#[test]
fn first_run_discovery_error_opens_settings_and_records_it() {
    let now = Instant::now();
    let mut c = ctrl(Config::default());
    c.handle(Event::Startup, now);
    let a = c.handle(Event::Discovered(Err("socket error".into())), now);
    assert_eq!(a, vec![Action::OpenSettings]);
    assert_eq!(c.snapshot(false).errors, vec!["socket error".to_string()]);
}

#[test]
fn saved_strip_is_resolved_then_lit_when_power_on_start() {
    let now = Instant::now();
    let mut c = ctrl(saved_cfg());
    let a = c.handle(Event::Startup, now);
    assert!(a.contains(&Action::Resolve(Target::Saved { id: "dcda0cd778bc".into(), last_ip: Some(IP) })));
    assert_eq!(c.snapshot(false).connection, Connection::Connecting);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), now);
    assert!(a.contains(&Action::Apply { ip: IP, change: full_on(c.config()) }), "{a:?}");
}

#[test]
fn power_on_start_turns_power_on_even_if_user_left_it_off() {
    let mut cfg = saved_cfg();
    cfg.light.power = false;
    let mut c = ctrl(cfg);
    let a = c.handle(Event::Startup, Instant::now());
    assert!(c.config().light.power);
    assert!(a.contains(&Action::SaveConfig));
}

#[test]
fn without_power_on_start_the_strip_is_left_alone() {
    let mut cfg = saved_cfg();
    cfg.startup.power_on = false;
    let now = Instant::now();
    let mut c = ctrl(cfg);
    c.handle(Event::Startup, now);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), now);
    assert!(!a.iter().any(|x| matches!(x, Action::Apply { .. })), "{a:?}");
}

#[test]
fn resolve_failure_goes_offline_with_growing_jittered_retries() {
    let now = Instant::now();
    let mut c = ctrl(saved_cfg());
    c.handle(Event::Startup, now);
    c.handle(Event::Resolved(Err("not found".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    let first = c.next_deadline().unwrap() - now;
    assert!(first >= Duration::from_secs(1) && first <= Duration::from_secs(2), "{first:?}");

    assert!(c.handle(Event::Tick, now + first / 2).is_empty(), "no retry before the deadline");
    let a = c.handle(Event::Tick, now + first);
    assert!(matches!(a.as_slice(), [Action::Resolve(Target::Saved { .. })]), "{a:?}");
    assert_eq!(c.snapshot(false).connection, Connection::Connecting);

    let t2 = now + first;
    c.handle(Event::Resolved(Err("not found".into())), t2);
    let second = c.next_deadline().unwrap() - t2;
    assert!(second >= Duration::from_secs(2) && second <= Duration::from_secs(4), "{second:?}");
    assert!(c.snapshot(false).retry_in_secs.is_some());
}

#[test]
fn apply_failure_keeps_change_and_reapplies_after_reconnect() {
    let (mut c, now) = ready();
    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(10, 20, 30)), brightness: None }), now);
    assert!(a.iter().any(|x| matches!(x, Action::Apply { .. })));
    c.handle(Event::Applied(Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);

    // A change made while offline is kept too.
    let a = c.handle(Event::Command(Command::SetStatic { color: None, brightness: Some(77) }), now);
    assert!(!a.iter().any(|x| matches!(x, Action::Apply { .. })), "nothing is sent while offline: {a:?}");

    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    let expected = LightChange { power: Some(true), color: Some(Rgb::new(10, 20, 30)), brightness: Some(77) };
    assert!(a.contains(&Action::Apply { ip: IP, change: expected }), "{a:?}");
}

#[test]
fn power_off_while_offline_is_applied_on_reconnect() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: true }), now);
    c.handle(Event::Applied(Err("timeout".into())), now);
    c.handle(Event::Command(Command::SetPower { on: false }), now);
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    assert!(a.contains(&Action::Apply { ip: IP, change: LightChange::off() }), "{a:?}");
}

#[test]
fn static_colour_implies_power_on_and_is_saved() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: false }), now);
    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: Some(50) }), now);
    assert!(c.config().light.power);
    assert_eq!(c.config().light.color, Rgb::new(1, 2, 3));
    assert_eq!(c.config().light.brightness, 50);
    assert!(a.contains(&Action::SaveConfig));
    assert!(a.contains(&Action::Apply { ip: IP, change: full_on(c.config()) }));
}

#[test]
fn invalid_values_are_rejected_without_saving() {
    let (mut c, now) = ready();
    for cmd in [
        Command::SetStatic { color: None, brightness: Some(0) },
        Command::SetStatic { color: None, brightness: Some(101) },
        Command::SetStatic { color: Some(Rgb::BLACK), brightness: None },
        Command::SetLedCount { count: 0 },
        Command::SetLedCount { count: 121 },
    ] {
        let before = c.config().clone();
        let a = c.handle(Event::Command(cmd.clone()), now);
        assert!(a.is_empty(), "{cmd:?} -> {a:?}");
        assert_eq!(c.config(), &before);
    }
    assert_eq!(c.snapshot(false).errors.len(), 5);
}

#[test]
fn settings_commands_update_config() {
    let (mut c, now) = ready();
    assert_eq!(c.handle(Event::Command(Command::SetLedCount { count: 90 }), now), vec![Action::SaveConfig]);
    assert_eq!(c.config().device.led_count, 90);
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    assert_eq!(c.config().app.exit_action, ExitAction::Off);
    c.handle(Event::Command(Command::SetPowerOnStart { enabled: false }), now);
    assert!(!c.config().startup.power_on);
    c.handle(Event::Command(Command::SetPresets { presets: vec![Rgb::new(9, 9, 9), Rgb::BLACK] }), now);
    assert_eq!(c.config().light.presets, vec![Rgb::new(9, 9, 9)]);
}

#[test]
fn manual_ip_takes_over_and_can_be_cleared() {
    let (mut c, now) = ready();
    let manual = Ipv4Addr::new(10, 0, 0, 9);
    let a = c.handle(Event::Command(Command::SetManualIp { ip: Some(manual) }), now);
    assert!(a.contains(&Action::SaveConfig));
    assert!(a.contains(&Action::Resolve(Target::Manual(manual))), "{a:?}");
    assert_eq!(c.snapshot(false).connection, Connection::Connecting);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("", manual), verified: false })), now);
    assert!(a.contains(&Action::Apply { ip: manual, change: full_on(c.config()) }), "{a:?}");
    assert_eq!(c.config().device.id, "dcda0cd778bc", "an empty resolved id keeps the saved one");
    assert!(!c.snapshot(false).verified);

    let a = c.handle(Event::Command(Command::SetManualIp { ip: None }), now);
    assert!(a.iter().any(|x| matches!(x, Action::Resolve(Target::Saved { .. }))), "{a:?}");
}

#[test]
fn selecting_a_discovered_strip_switches_to_it() {
    let (mut c, now) = ready();
    let other = Ipv4Addr::new(192, 168, 1, 99);
    c.handle(Event::Command(Command::Discover), now);
    c.handle(Event::Discovered(Ok(vec![dev("dcda0cd778bc", IP), dev("other", other)])), now);
    let a = c.handle(Event::Command(Command::SelectDevice { id: "other".into() }), now);
    assert_eq!(c.config().device.id, "other");
    assert!(a.contains(&Action::Apply { ip: other, change: full_on(c.config()) }), "{a:?}");
    let a = c.handle(Event::Command(Command::SelectDevice { id: "ghost".into() }), now);
    assert!(a.is_empty());
}

#[test]
fn discover_is_not_restarted_while_running() {
    let (mut c, now) = ready();
    assert_eq!(c.handle(Event::Command(Command::Discover), now), vec![Action::Discover]);
    assert!(c.handle(Event::Command(Command::Discover), now).is_empty());
}

#[test]
fn connection_test_reports_result() {
    let (mut c, now) = ready();
    assert_eq!(c.handle(Event::Command(Command::TestConnection), now), vec![Action::Test { ip: IP }]);
    assert_eq!(c.snapshot(false).test, TestState::Running);
    c.handle(Event::Tested(Ok(Duration::from_millis(31))), now);
    assert_eq!(c.snapshot(false).test, TestState::Ok { millis: 31 });
    c.handle(Event::Command(Command::TestConnection), now);
    c.handle(Event::Tested(Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).test, TestState::Failed { message: "timeout".into() });
}

#[test]
fn connection_test_without_strip_fails_immediately() {
    let mut c = ctrl(Config::default());
    assert!(c.handle(Event::Command(Command::TestConnection), Instant::now()).is_empty());
    assert!(matches!(c.snapshot(false).test, TestState::Failed { .. }));
}

#[test]
fn exit_powers_off_only_when_asked_and_connected() {
    let (mut c, now) = ready();
    assert_eq!(
        c.handle(Event::Command(Command::Exit), now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: None }]
    );

    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    assert_eq!(
        c.handle(Event::Command(Command::Exit), now),
        vec![Action::Shutdown { power_off_ip: Some(IP), stop_stream: None }]
    );

    let mut c = ctrl(saved_cfg());
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    c.handle(Event::Startup, now);
    c.handle(Event::Resolved(Err("x".into())), now);
    assert_eq!(
        c.handle(Event::Command(Command::Exit), now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: None }]
    );
}

#[test]
fn error_list_is_capped() {
    let (mut c, _) = ready();
    for i in 0..30 {
        c.record_error(format!("e{i}"));
    }
    let errors = c.snapshot(false).errors;
    assert_eq!(errors.len(), MAX_ERRORS);
    assert_eq!(errors.last().unwrap(), "e29");
}

#[test]
fn tick_without_deadline_does_nothing() {
    let (mut c, now) = ready();
    assert!(c.next_deadline().is_none());
    assert!(c.handle(Event::Tick, now + Duration::from_secs(999)).is_empty());
}

#[test]
fn snapshot_carries_config_and_warnings() {
    let c = Controller::new(Config::default(), vec!["w1".into()], Backoff::with_seed(RETRY_BASE, RETRY_BASE, None, 1));
    let s = c.snapshot(true);
    assert_eq!(s.config_warnings, vec!["w1".to_string()]);
    assert!(s.session_open);
    assert_eq!(s.config, Config::default());
    assert_eq!(s.version, env!("CARGO_PKG_VERSION"));
}

// ---------- coalescing ----------

#[test]
fn coalesce_merges_consecutive_light_changes() {
    let out = coalesce(vec![
        Command::SetStatic { color: Some(Rgb::new(1, 1, 1)), brightness: Some(10) },
        Command::SetStatic { color: Some(Rgb::new(2, 2, 2)), brightness: None },
        Command::SetStatic { color: None, brightness: Some(30) },
    ]);
    assert_eq!(out, vec![Command::SetStatic { color: Some(Rgb::new(2, 2, 2)), brightness: Some(30) }]);
}

#[test]
fn coalesce_keeps_last_power_and_order_of_other_commands() {
    let out = coalesce(vec![
        Command::SetPower { on: false },
        Command::SetPower { on: true },
        Command::Discover,
        Command::SetStatic { color: None, brightness: Some(5) },
        Command::SetPower { on: false },
        Command::SetPresets { presets: vec![] },
        Command::SetPresets { presets: vec![Rgb::new(1, 1, 1)] },
    ]);
    assert_eq!(
        out,
        vec![
            Command::SetPower { on: true },
            Command::Discover,
            Command::SetStatic { color: None, brightness: Some(5) },
            Command::SetPower { on: false },
            Command::SetPresets { presets: vec![Rgb::new(1, 1, 1)] },
        ]
    );
}

#[test]
fn exit_is_not_allowed_over_ipc() {
    assert!(!Command::Exit.allowed_over_ipc());
    assert!(Command::Discover.allowed_over_ipc());
    assert!(Command::SetPower { on: true }.is_light_change());
    assert!(!Command::Discover.is_light_change());
}

#[test]
fn backoff_keeps_growing_when_strip_is_found_but_commands_fail() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: true }), now);
    c.handle(Event::Applied(Err("timeout".into())), now);
    let first = c.next_deadline().unwrap() - now;

    // Discovery finds the strip again, but the re-sent change fails again.
    let t1 = now + first;
    c.handle(Event::Tick, t1);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), t1);
    assert!(a.iter().any(|x| matches!(x, Action::Apply { .. })));
    c.handle(Event::Applied(Err("timeout".into())), t1);
    let second = c.next_deadline().unwrap() - t1;
    assert!(second >= Duration::from_secs(2), "backoff must not reset before a command succeeds: {second:?}");

    // A successful apply resets it.
    let t2 = t1 + second;
    c.handle(Event::Tick, t2);
    c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), t2);
    c.handle(Event::Applied(Ok(())), t2);
    c.handle(Event::Command(Command::SetPower { on: true }), t2);
    c.handle(Event::Applied(Err("timeout".into())), t2);
    let third = c.next_deadline().unwrap() - t2;
    assert!(third <= Duration::from_secs(2), "reset after success: {third:?}");
}

// ---------- Ambilight ----------

fn ambilight_cfg() -> Config {
    let mut c = saved_cfg();
    c.app.mode = Mode::Ambilight;
    c
}

fn found(c: &mut Controller, at: Instant) -> Vec<Action> {
    c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at)
}

/// Started in Ambilight mode, connected, stream running.
fn streaming() -> (Controller, Instant) {
    let now = Instant::now();
    let mut c = ctrl(ambilight_cfg());
    c.handle(Event::Startup, now);
    assert_eq!(found(&mut c, now), vec![Action::StartStream { ip: IP }]);
    c.handle(Event::StreamStarted(Ok(())), now);
    (c, now)
}

#[test]
fn ambilight_mode_streams_after_connecting() {
    let (c, _) = streaming();
    assert!(c.streaming());
    assert!(c.capture_wanted());
    let s = c.snapshot(true);
    assert!(s.stream.streaming);
    assert!(!s.stream.highlight);
}

#[test]
fn switching_modes_enters_and_leaves_direct_mode() {
    let (mut c, now) = ready();
    let a = c.handle(Event::Command(Command::SetMode { mode: Mode::Ambilight }), now);
    assert_eq!(a, vec![Action::SaveConfig, Action::StartStream { ip: IP }]);
    c.handle(Event::StreamStarted(Ok(())), now);
    assert!(c.capture_wanted());

    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: None }), now);
    assert_eq!(c.config().app.mode, Mode::Static, "a static colour leaves Ambilight");
    assert_eq!(a, vec![Action::SaveConfig, Action::StopStream { ip: IP, then: full_on(c.config()) }]);
    assert!(!c.capture_wanted());
    c.handle(Event::StreamStopped(IP, Ok(())), now);

    c.handle(Event::Command(Command::SetMode { mode: Mode::Ambilight }), now);
    c.handle(Event::StreamStarted(Ok(())), now);
    let a = c.handle(Event::Command(Command::SetPower { on: false }), now);
    assert_eq!(a, vec![Action::SaveConfig, Action::StopStream { ip: IP, then: LightChange::off() }]);
    let a = c.handle(Event::Command(Command::SetPower { on: true }), now);
    assert_eq!(a, vec![Action::SaveConfig, Action::StartStream { ip: IP }], "power on resumes the chosen mode");
}

#[test]
fn choosing_a_mode_turns_the_strip_on() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: false }), now);
    c.handle(Event::Command(Command::SetMode { mode: Mode::Static }), now);
    assert!(c.config().light.power);
}

#[test]
fn ambilight_is_not_started_without_power_on_start() {
    let mut cfg = ambilight_cfg();
    cfg.startup.power_on = false;
    cfg.light.power = false;
    let now = Instant::now();
    let mut c = ctrl(cfg);
    c.handle(Event::Startup, now);
    assert!(found(&mut c, now).is_empty());
    assert!(!c.streaming());
}

#[test]
fn stream_failure_resumes_ambilight_after_reconnect() {
    let (mut c, now) = streaming();
    c.handle(Event::StreamFailed("keep-alive lost".into()), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    assert!(!c.streaming());
    assert!(!c.capture_wanted());
    let at = c.next_deadline().unwrap();
    assert!(matches!(c.handle(Event::Tick, at).as_slice(), [Action::Resolve(_)]));
    assert_eq!(found(&mut c, at), vec![Action::StartStream { ip: IP }]);
}

#[test]
fn a_failed_stream_start_goes_offline_and_retries() {
    let now = Instant::now();
    let mut c = ctrl(ambilight_cfg());
    c.handle(Event::Startup, now);
    found(&mut c, now);
    c.handle(Event::StreamStarted(Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    assert!(!c.streaming());
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    assert_eq!(found(&mut c, at), vec![Action::StartStream { ip: IP }]);
}

#[test]
fn a_failed_stream_stop_retries_the_static_state() {
    let (mut c, now) = streaming();
    c.handle(Event::Command(Command::SetMode { mode: Mode::Static }), now);
    c.handle(Event::StreamStopped(IP, Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    assert_eq!(found(&mut c, at), vec![Action::Apply { ip: IP, change: full_on(c.config()) }]);
}

#[test]
fn highlight_from_static_starts_and_ends_the_stream() {
    let (mut c, now) = ready();
    let a = c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(3) }), now);
    assert_eq!(a, vec![Action::StartStream { ip: IP }]);
    c.handle(Event::StreamStarted(Ok(())), now);
    assert_eq!(c.highlight(), Some(HighlightTarget::Led(3)));
    assert!(!c.capture_wanted(), "static mode needs no capture");
    assert_eq!(c.next_deadline(), Some(now + HIGHLIGHT_TTL));
    assert!(c.snapshot(true).stream.highlight);

    let t1 = now + Duration::from_secs(2);
    let a = c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(4) }), t1);
    assert!(a.is_empty(), "already streaming: {a:?}");
    assert!(c.handle(Event::Tick, now + HIGHLIGHT_TTL).is_empty(), "renewed at t1");
    let a = c.handle(Event::Tick, t1 + HIGHLIGHT_TTL);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: full_on(c.config()) }]);
    assert_eq!(c.highlight(), None);
    assert!(!c.streaming());
}

#[test]
fn highlight_in_ambilight_keeps_the_stream() {
    let (mut c, now) = streaming();
    let a = c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Segment(0) }), now);
    assert!(a.is_empty(), "{a:?}");
    assert!(c.capture_wanted(), "capture keeps running under the overlay");
    assert!(c.handle(Event::Command(Command::StopHighlight), now).is_empty());
    assert!(c.streaming());
}

#[test]
fn changes_during_a_highlight_wait_for_its_end() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(0) }), now);
    c.handle(Event::StreamStarted(Ok(())), now);
    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(7, 7, 7)), brightness: None }), now);
    assert_eq!(a, vec![Action::SaveConfig]);
    let a = c.handle(Event::Command(Command::StopHighlight), now);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: full_on(c.config()) }]);
    assert_eq!(c.config().light.color, Rgb::new(7, 7, 7));
}

#[test]
fn highlight_needs_a_connected_strip_and_a_real_target() {
    let mut c = ctrl(saved_cfg());
    assert!(
        c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(0) }), Instant::now()).is_empty()
    );
    assert_eq!(c.snapshot(false).errors.len(), 1);

    let (mut c, now) = ready();
    assert!(c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(60) }), now).is_empty());
    assert!(c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Segment(99) }), now).is_empty());
    assert_eq!(c.snapshot(false).errors.len(), 2);
    assert!(!c.streaming());
}

#[test]
fn exit_from_a_stream_leaves_direct_mode() {
    let (mut c, now) = streaming();
    let full = full_on(c.config());
    assert_eq!(
        c.handle(Event::Command(Command::Exit), now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: Some((IP, full)) }]
    );
    let (mut c, now) = streaming();
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    assert_eq!(
        c.handle(Event::Command(Command::Exit), now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: Some((IP, LightChange::off())) }]
    );
}

#[test]
fn ambilight_settings_are_checked_and_keep_the_segments() {
    let (mut c, now) = ready();
    let before = c.config().clone();
    let bad = AmbilightConfig { fps: 5, ..before.ambilight.clone() };
    assert!(c.handle(Event::Command(Command::SetAmbilight { settings: bad }), now).is_empty());
    assert_eq!(c.config(), &before);
    let good = AmbilightConfig { fps: 30, segments: Vec::new(), ..before.ambilight.clone() };
    assert_eq!(c.handle(Event::Command(Command::SetAmbilight { settings: good }), now), vec![Action::SaveConfig]);
    assert_eq!(c.config().ambilight.fps, 30);
    assert_eq!(c.config().ambilight.segments, before.ambilight.segments, "segments change only via SetLayout");
}

#[test]
fn layout_is_sanitized() {
    let (mut c, now) = ready();
    let segs = vec![
        Segment { from: 0, to: 9, start: [0.0, 0.0], end: [1.0, 0.0] },
        Segment { from: 5, to: 15, start: [0.0, 1.0], end: [1.0, 1.0] },
    ];
    assert_eq!(c.handle(Event::Command(Command::SetLayout { segments: segs.clone() }), now), vec![Action::SaveConfig]);
    assert_eq!(c.config().ambilight.segments, segs[..1]);
    assert_eq!(c.snapshot(false).errors.len(), 1);
}

#[test]
fn a_smaller_led_count_trims_the_layout() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetLedCount { count: 30 }), now);
    assert!(!c.config().ambilight.segments.is_empty());
    assert!(c.config().ambilight.segments.iter().all(|s| s.hi() < 30));
}

#[test]
fn selecting_another_strip_stops_the_stream_on_the_old_one() {
    let (mut c, now) = streaming();
    let other = Ipv4Addr::new(192, 168, 1, 99);
    c.handle(Event::Command(Command::Discover), now);
    c.handle(Event::Discovered(Ok(vec![dev("dcda0cd778bc", IP), dev("other", other)])), now);
    let a = c.handle(Event::Command(Command::SelectDevice { id: "other".into() }), now);
    assert_eq!(a.first(), Some(&Action::StopStream { ip: IP, then: full_on(c.config()) }));
    assert!(a.contains(&Action::StartStream { ip: other }), "{a:?}");
    c.handle(Event::StreamStopped(IP, Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Ready, "the old strip's failure does not matter");
}

#[test]
fn coalesce_keeps_the_last_of_ambilight_runs() {
    let a = AmbilightConfig::defaults(60);
    let b = AmbilightConfig { fps: 30, ..a.clone() };
    let seg = Segment { from: 0, to: 1, start: [0.0, 0.0], end: [1.0, 0.0] };
    let out = coalesce(vec![
        Command::SetAmbilight { settings: a },
        Command::SetAmbilight { settings: b.clone() },
        Command::SetLayout { segments: vec![] },
        Command::SetLayout { segments: vec![seg.clone()] },
        Command::Highlight { target: HighlightTarget::Led(1) },
        Command::Highlight { target: HighlightTarget::Led(2) },
    ]);
    assert_eq!(
        out,
        vec![
            Command::SetAmbilight { settings: b },
            Command::SetLayout { segments: vec![seg] },
            Command::Highlight { target: HighlightTarget::Led(2) },
        ]
    );
}

// ---------- Windows lifecycle ----------

fn cmd(c: &mut Controller, command: Command, at: Instant) -> Vec<Action> {
    c.handle(Event::Command(command), at)
}

#[test]
fn startup_syncs_autostart_first() {
    let mut cfg = saved_cfg();
    let a = ctrl(cfg.clone()).handle(Event::Startup, Instant::now());
    assert_eq!(a.first(), Some(&Action::SyncAutostart { enabled: true }));
    cfg.startup.autostart = false;
    let a = ctrl(cfg).handle(Event::Startup, Instant::now());
    assert_eq!(a.first(), Some(&Action::SyncAutostart { enabled: false }));
}

#[test]
fn set_autostart_saves_and_syncs() {
    let (mut c, now) = ready();
    let a = cmd(&mut c, Command::SetAutostart { enabled: false }, now);
    assert_eq!(a, vec![Action::SaveConfig, Action::SyncAutostart { enabled: false }]);
    assert!(!c.config().startup.autostart);
}

#[test]
fn autostart_failure_is_reported() {
    let (mut c, now) = ready();
    assert!(c.handle(Event::AutostartSynced(Err("access denied".into())), now).is_empty());
    assert!(c.snapshot(false).errors.iter().any(|e| e.contains("autostart") && e.contains("access denied")));
}

#[test]
fn locking_turns_a_static_strip_off_and_unlocking_restores_it() {
    let (mut c, now) = ready();
    let before = c.config().clone();
    assert_eq!(
        cmd(&mut c, Command::Darken { reason: Dark::Locked }, now),
        vec![Action::Apply { ip: IP, change: LightChange::off() }]
    );
    assert_eq!(c.snapshot(false).dark, vec![Dark::Locked]);
    assert!(cmd(&mut c, Command::Darken { reason: Dark::Locked }, now).is_empty(), "repeats are ignored");
    assert_eq!(
        cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now),
        vec![Action::Apply { ip: IP, change: full_on(c.config()) }]
    );
    assert!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now).is_empty());
    assert_eq!(c.config(), &before, "Windows events never change the config");
}

#[test]
fn locking_stops_ambilight_and_unlocking_restarts_it() {
    let (mut c, now) = streaming();
    let a = cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: LightChange::off() }]);
    assert!(!c.capture_wanted());
    c.handle(Event::StreamStopped(IP, Ok(())), now);
    assert_eq!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now), vec![Action::StartStream { ip: IP }]);
}

#[test]
fn the_strip_stays_dark_until_every_reason_is_gone() {
    let (mut c, now) = ready();
    cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    assert!(cmd(&mut c, Command::Darken { reason: Dark::DisplayOff }, now).is_empty());
    assert!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now).is_empty(), "display still off");
    assert_eq!(c.snapshot(false).dark, vec![Dark::DisplayOff]);
    let a = cmd(&mut c, Command::Undarken { reason: Dark::DisplayOff }, now);
    assert_eq!(a, vec![Action::Apply { ip: IP, change: full_on(c.config()) }]);
}

#[test]
fn changes_while_dark_apply_after_it() {
    let (mut c, now) = ready();
    cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    let a = cmd(&mut c, Command::SetStatic { color: Some(Rgb::new(5, 6, 7)), brightness: None }, now);
    assert_eq!(a, vec![Action::SaveConfig], "saved, not sent");
    assert_eq!(cmd(&mut c, Command::SetMode { mode: Mode::Ambilight }, now), vec![Action::SaveConfig]);
    assert_eq!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now), vec![Action::StartStream { ip: IP }]);
    assert_eq!(c.config().light.color, Rgb::new(5, 6, 7));
}

#[test]
fn darkness_while_offline_is_applied_after_reconnect() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: true }), now);
    c.handle(Event::Applied(Err("timeout".into())), now);
    assert!(cmd(&mut c, Command::Darken { reason: Dark::Sleep }, now).is_empty(), "nothing to send offline");
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    assert_eq!(a, vec![Action::Apply { ip: IP, change: LightChange::off() }]);
}

#[test]
fn sleep_with_a_failed_stop_resumes_after_reconnect() {
    let (mut c, now) = streaming();
    cmd(&mut c, Command::Darken { reason: Dark::Sleep }, now);
    c.handle(Event::StreamStopped(IP, Err("network down".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    assert!(cmd(&mut c, Command::Undarken { reason: Dark::Sleep }, now).is_empty(), "still offline");
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    assert_eq!(a, vec![Action::StartStream { ip: IP }]);
}

#[test]
fn highlights_are_refused_and_ended_while_dark() {
    let (mut c, now) = ready();
    cmd(&mut c, Command::Highlight { target: HighlightTarget::Led(1) }, now);
    c.handle(Event::StreamStarted(Ok(())), now);
    let a = cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: LightChange::off() }], "the overlay ends");
    assert_eq!(c.highlight(), None);
    assert!(cmd(&mut c, Command::Highlight { target: HighlightTarget::Led(1) }, now).is_empty());
    assert!(c.snapshot(false).errors.iter().any(|e| e.contains("highlight")));
}

#[test]
fn ending_the_session_always_turns_the_strip_off() {
    let (mut c, now) = ready();
    assert_eq!(c.config().app.exit_action, ExitAction::Keep);
    assert_eq!(
        cmd(&mut c, Command::EndSession, now),
        vec![Action::Shutdown { power_off_ip: Some(IP), stop_stream: None }]
    );
    let (mut c, now) = streaming();
    assert_eq!(
        cmd(&mut c, Command::EndSession, now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: Some((IP, LightChange::off())) }]
    );
    let mut c = ctrl(saved_cfg());
    assert_eq!(
        cmd(&mut c, Command::EndSession, Instant::now()),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: None }]
    );
}

#[test]
fn lifecycle_commands_are_tray_only() {
    for c in [Command::EndSession, Command::Darken { reason: Dark::Sleep }, Command::Undarken { reason: Dark::Sleep }] {
        assert!(!c.allowed_over_ipc(), "{c:?}");
    }
    assert!(Command::SetAutostart { enabled: true }.allowed_over_ipc());
}

#[test]
fn restored_dark_reasons_keep_the_strip_off_after_a_restart() {
    let now = Instant::now();
    let mut c = ctrl(saved_cfg());
    c.restore_dark(&[Dark::Locked]);
    c.handle(Event::Startup, now);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), now);
    assert_eq!(a, vec![Action::Apply { ip: IP, change: LightChange::off() }]);
    assert_eq!(c.snapshot(false).dark, vec![Dark::Locked]);
}
