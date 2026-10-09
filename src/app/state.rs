//! Data shared between the worker, the tray and the settings window.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::discovery::DiscoveredDevice;

/// A strip as known to the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// Discovery id; empty when connected by manual IP without discovery.
    pub id: String,
    pub name: String,
    pub ip: Ipv4Addr,
    pub fw_ver: Option<String>,
}

impl DeviceInfo {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() { crate::protocol::device::MODEL } else { &self.name }
    }
}

impl From<&DiscoveredDevice> for DeviceInfo {
    fn from(d: &DiscoveredDevice) -> Self {
        Self { id: d.id.clone(), name: d.name.clone(), ip: d.ip, fw_ver: d.fw_ver.clone() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    NoDevice,
    Connecting,
    Ready,
    Offline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TestState {
    Idle,
    Running,
    Ok { millis: u64 },
    Failed { message: String },
}

/// Settings window tabs; also used on the command line (`--tab device`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tab {
    #[default]
    Device,
    Lighting,
    General,
    Diagnostics,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Device, Tab::Lighting, Tab::General, Tab::Diagnostics];

    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Lighting => "lighting",
            Self::General => "general",
            Self::Diagnostics => "diagnostics",
        }
    }

    pub fn from_arg(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_arg() == s)
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL.get(i).copied().unwrap_or_default()
    }
}

/// Everything the UI shows, published by the worker after every change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: String,
    pub connection: Connection,
    /// Seconds until the next reconnect attempt while offline.
    pub retry_in_secs: Option<u64>,
    pub device: Option<DeviceInfo>,
    /// False when connected by IP without confirming the device id.
    pub verified: bool,
    pub session_open: bool,
    pub searching: bool,
    pub discovered: Vec<DeviceInfo>,
    pub test: TestState,
    pub errors: Vec<String>,
    pub config_warnings: Vec<String>,
    pub config: Config,
}
