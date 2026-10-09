//! Messages between the settings window and the background process: one
//! JSON object per line, request → response.

pub mod pipe;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::app::commands::Command;
use crate::app::state::{Snapshot, Tab};

/// Longest accepted line; protects the server from runaway clients.
pub const MAX_MESSAGE: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "req", rename_all = "snake_case")]
pub enum Request {
    GetState,
    OpenSettings { tab: Option<Tab> },
    Command { command: Command },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "resp", rename_all = "snake_case")]
pub enum Response {
    Ok,
    State { snapshot: Box<Snapshot> },
    Error { message: String },
}

pub fn encode<T: Serialize>(msg: &T) -> String {
    let mut s = serde_json::to_string(msg).expect("IPC messages always serialise");
    s.push('\n');
    s
}

pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, String> {
    serde_json::from_str(line.trim_end()).map_err(|e| e.to_string())
}
