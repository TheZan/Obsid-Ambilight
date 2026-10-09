use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use obsid_ambilight::app::commands::{Command, coalesce};
use obsid_ambilight::app::controller::{
    Action, Controller, Event, LightChange, MAX_ERRORS, RETRY_BASE, Resolved, Target,
};
use obsid_ambilight::app::state::{Connection, DeviceInfo, TestState};
use obsid_ambilight::config::{Config, ExitAction};
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
    assert_eq!(c.handle(Event::Command(Command::Exit), now), vec![Action::Shutdown { power_off_ip: None }]);

    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    assert_eq!(c.handle(Event::Command(Command::Exit), now), vec![Action::Shutdown { power_off_ip: Some(IP) }]);

    let mut c = ctrl(saved_cfg());
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    c.handle(Event::Startup, now);
    c.handle(Event::Resolved(Err("x".into())), now);
    assert_eq!(c.handle(Event::Command(Command::Exit), now), vec![Action::Shutdown { power_off_ip: None }]);
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
