//! Commands accepted by the device worker.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use super::state::Dark;
use crate::ambilight::layout::HighlightTarget;
use crate::config::{AmbilightConfig, ExitAction, Language, Mode, Segment};
use crate::protocol::Rgb;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Static colour and/or brightness; implies power on.
    SetStatic {
        color: Option<Rgb>,
        brightness: Option<u8>,
    },
    SetPower {
        on: bool,
    },
    SelectDevice {
        id: String,
    },
    SetManualIp {
        ip: Option<Ipv4Addr>,
    },
    Discover,
    TestConnection,
    SetLanguage {
        language: Language,
    },
    SetPowerOnStart {
        enabled: bool,
    },
    SetExitAction {
        action: ExitAction,
    },
    SetLedCount {
        count: usize,
    },
    SetPresets {
        presets: Vec<Rgb>,
    },
    /// Static or Ambilight; implies power on.
    SetMode {
        mode: Mode,
    },
    /// Every Ambilight setting except `segments`, which is ignored here (see `SetLayout`).
    SetAmbilight {
        settings: AmbilightConfig,
    },
    SetLayout {
        segments: Vec<Segment>,
    },
    /// Layout editor: light one LED or segment for a few seconds.
    Highlight {
        target: HighlightTarget,
    },
    StopHighlight,
    /// Windows locked the session, went to sleep or turned the display off.
    Darken {
        reason: Dark,
    },
    Undarken {
        reason: Dark,
    },
    /// Windows is shutting down or logging off: strip off, then stop (tray only).
    EndSession,
    SetAutostart {
        enabled: bool,
    },
    /// Tray "Exit" only; refused over IPC.
    Exit,
}

impl Command {
    pub fn is_light_change(&self) -> bool {
        matches!(self, Self::SetStatic { .. } | Self::SetPower { .. })
    }

    pub fn allowed_over_ipc(&self) -> bool {
        !matches!(self, Self::Exit | Self::EndSession | Self::Darken { .. } | Self::Undarken { .. })
    }
}

/// Collapses a burst: adjacent `SetStatic` merge field by field (later wins),
/// adjacent `SetPower`, `SetPresets`, `SetAmbilight`, `SetLayout` and
/// `Highlight` keep the last one; order is preserved.
pub fn coalesce(cmds: Vec<Command>) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        match (out.last_mut(), cmd) {
            (
                Some(Command::SetStatic { color, brightness }),
                Command::SetStatic { color: new_color, brightness: new_brightness },
            ) => {
                if new_color.is_some() {
                    *color = new_color;
                }
                if new_brightness.is_some() {
                    *brightness = new_brightness;
                }
            }
            (Some(last @ Command::SetPower { .. }), cmd @ Command::SetPower { .. })
            | (Some(last @ Command::SetPresets { .. }), cmd @ Command::SetPresets { .. })
            | (Some(last @ Command::SetAmbilight { .. }), cmd @ Command::SetAmbilight { .. })
            | (Some(last @ Command::SetLayout { .. }), cmd @ Command::SetLayout { .. })
            | (Some(last @ Command::Highlight { .. }), cmd @ Command::Highlight { .. }) => *last = cmd,
            (_, cmd) => out.push(cmd),
        }
    }
    out
}
