//! Windows session, power and display messages → lifecycle events → worker
//! commands. Pure: the tray reads the messages and sends the commands.

use std::time::Duration;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WM_ENDSESSION,
    WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

use crate::app::commands::Command;
use crate::app::state::Dark;

/// Windows gives about two seconds before sleeping; leave some margin.
pub const SLEEP_WAIT: Duration = Duration::from_millis(1500);
pub const END_SESSION_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    Lock,
    Unlock,
    Suspend,
    Resume,
    DisplayOff,
    DisplayOn,
    EndSession,
}

/// `display_state` is the DWORD of a `GUID_CONSOLE_DISPLAY_STATE` power
/// setting change (0 off, 1 on, 2 dimmed), read by the caller from lParam;
/// `None` for other messages and other power settings.
pub fn event(msg: u32, wparam: usize, display_state: Option<u32>) -> Option<LifecycleEvent> {
    let w = wparam as u32;
    match msg {
        WM_WTSSESSION_CHANGE if w == WTS_SESSION_LOCK => Some(LifecycleEvent::Lock),
        WM_WTSSESSION_CHANGE if w == WTS_SESSION_UNLOCK => Some(LifecycleEvent::Unlock),
        WM_POWERBROADCAST if w == PBT_APMSUSPEND => Some(LifecycleEvent::Suspend),
        WM_POWERBROADCAST if w == PBT_APMRESUMEAUTOMATIC || w == PBT_APMRESUMESUSPEND => Some(LifecycleEvent::Resume),
        WM_POWERBROADCAST if w == PBT_POWERSETTINGCHANGE => match display_state? {
            0 => Some(LifecycleEvent::DisplayOff),
            _ => Some(LifecycleEvent::DisplayOn),
        },
        WM_ENDSESSION if wparam != 0 => Some(LifecycleEvent::EndSession),
        _ => None,
    }
}

impl LifecycleEvent {
    /// Worker commands for this event. Unlocking also clears `Sleep`, in case
    /// the resume message was lost.
    pub fn commands(self) -> Vec<Command> {
        let darken = |reason| Command::Darken { reason };
        let undarken = |reason| Command::Undarken { reason };
        match self {
            Self::Lock => vec![darken(Dark::Locked)],
            Self::Unlock => vec![undarken(Dark::Locked), undarken(Dark::Sleep)],
            Self::Suspend => vec![darken(Dark::Sleep)],
            Self::Resume => vec![undarken(Dark::Sleep)],
            Self::DisplayOff => vec![darken(Dark::DisplayOff)],
            Self::DisplayOn => vec![undarken(Dark::DisplayOff)],
            Self::EndSession => vec![Command::EndSession],
        }
    }

    /// How long the tray waits for the worker before letting Windows go on.
    pub fn wait(self) -> Option<Duration> {
        match self {
            Self::Suspend => Some(SLEEP_WAIT),
            Self::EndSession => Some(END_SESSION_WAIT),
            _ => None,
        }
    }
}
