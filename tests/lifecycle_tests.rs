use std::time::Duration;

use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::state::Dark;
use obsid_ambilight::ui::lifecycle::{END_SESSION_WAIT, LifecycleEvent as E, SLEEP_WAIT, event};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WM_ENDSESSION,
    WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

#[test]
fn session_changes_map_to_lock_and_unlock() {
    assert_eq!(event(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK as usize, None), Some(E::Lock));
    assert_eq!(event(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK as usize, None), Some(E::Unlock));
    assert_eq!(event(WM_WTSSESSION_CHANGE, 1, None), None, "console connect is not a lock");
}

#[test]
fn power_broadcasts_map_to_sleep_and_display() {
    let pb = |w: u32, d| event(WM_POWERBROADCAST, w as usize, d);
    assert_eq!(pb(PBT_APMSUSPEND, None), Some(E::Suspend));
    assert_eq!(pb(PBT_APMRESUMEAUTOMATIC, None), Some(E::Resume));
    assert_eq!(pb(PBT_APMRESUMESUSPEND, None), Some(E::Resume));
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, Some(0)), Some(E::DisplayOff));
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, Some(1)), Some(E::DisplayOn));
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, Some(2)), Some(E::DisplayOn), "dimmed is still on");
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, None), None, "another power setting");
}

#[test]
fn only_a_real_session_end_counts() {
    assert_eq!(event(WM_ENDSESSION, 1, None), Some(E::EndSession));
}

#[test]
fn a_cancelled_session_end_is_ignored() {
    assert_eq!(event(WM_ENDSESSION, 0, None), None);
}

#[test]
fn unrelated_messages_are_ignored() {
    assert_eq!(event(0x0200, 0, None), None);
}

#[test]
fn events_become_worker_commands() {
    let d = |reason| Command::Darken { reason };
    let u = |reason| Command::Undarken { reason };
    assert_eq!(E::Lock.commands(), vec![d(Dark::Locked)]);
    assert_eq!(E::Unlock.commands(), vec![u(Dark::Locked), u(Dark::Sleep)], "unlock also clears a lost resume");
    assert_eq!(E::Suspend.commands(), vec![d(Dark::Sleep)]);
    assert_eq!(E::Resume.commands(), vec![u(Dark::Sleep)]);
    assert_eq!(E::DisplayOff.commands(), vec![d(Dark::DisplayOff)]);
    assert_eq!(E::DisplayOn.commands(), vec![u(Dark::DisplayOff)]);
    assert_eq!(E::EndSession.commands(), vec![Command::EndSession]);
}

#[test]
fn only_sleep_and_session_end_are_awaited() {
    assert_eq!(E::Suspend.wait(), Some(SLEEP_WAIT));
    assert_eq!(E::EndSession.wait(), Some(END_SESSION_WAIT));
    assert_eq!(E::Lock.wait(), None);
    assert_eq!(SLEEP_WAIT, Duration::from_millis(1500));
    assert_eq!(END_SESSION_WAIT, Duration::from_secs(2));
}
