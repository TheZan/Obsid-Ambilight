mod common;

use std::fs;

use obsid_ambilight::diagnostics;
use obsid_ambilight::platform::{self, instance::InstanceGuard, paths};

#[test]
fn mutex_allows_one_holder() {
    let name = format!("Local\\ObsidAmbilight.Test.{}", std::process::id());
    let first = InstanceGuard::acquire(&name).unwrap();
    assert!(first.is_some());
    assert!(InstanceGuard::acquire(&name).unwrap().is_none(), "second holder refused");
    drop(first);
    assert!(InstanceGuard::acquire(&name).unwrap().is_some(), "free again after drop");
}

#[test]
fn paths_live_under_appdata() {
    let dir = paths::app_dir().unwrap();
    assert!(dir.ends_with("ObsidAmbilight"));
    assert_eq!(paths::config_path().unwrap(), dir.join("config.toml"));
    assert_eq!(paths::logs_dir().unwrap(), dir.join("logs"));
}

#[test]
fn wide_strings_are_nul_terminated() {
    assert_eq!(platform::wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
}

#[test]
fn rotate_keeps_small_logs() {
    let dir = common::temp_dir("rotate-small");
    let log = dir.join("app.log");
    fs::write(&log, "short").unwrap();
    diagnostics::rotate(&log, 100).unwrap();
    assert!(log.exists());
    assert!(!dir.join("app.log.1").exists());
}

#[test]
fn rotate_moves_big_log_and_replaces_old_copy() {
    let dir = common::temp_dir("rotate-big");
    let log = dir.join("app.log");
    fs::write(dir.join("app.log.1"), "older").unwrap();
    fs::write(&log, vec![b'x'; 200]).unwrap();
    diagnostics::rotate(&log, 100).unwrap();
    assert!(!log.exists());
    assert_eq!(fs::read(dir.join("app.log.1")).unwrap().len(), 200);
}

#[test]
fn rotate_ignores_missing_file() {
    let dir = common::temp_dir("rotate-missing");
    diagnostics::rotate(&dir.join("none.log"), 100).unwrap();
}
