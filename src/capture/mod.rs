//! Screen capture for Ambilight. `dxgi`, `outputs` and `display` talk to
//! Windows; consumers only see the [`Grabber`] trait.

pub mod display;
pub mod dxgi;
pub mod outputs;
pub mod thread;

use std::sync::Arc;

use crate::ambilight::image::Image;

/// What a grabber captures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GrabberInfo {
    /// DXGI output name, e.g. `\\.\DISPLAY1`.
    pub monitor: String,
    pub adapter: String,
    pub width: u32,
    pub height: u32,
    pub hdr: bool,
    pub sdr_white_nits: f32,
    /// The requested monitor was not found; the primary one is captured.
    pub fallback: bool,
}

/// Source of small frames. Created and used on the capture thread only.
pub trait Grabber {
    fn info(&self) -> &GrabberInfo;
    /// The newest frame if the screen changed since the last call, `Ok(None)`
    /// otherwise. `Err` means the grabber is unusable and must be reopened.
    fn grab(&mut self) -> Result<Option<Image>, String>;
}

/// Opens a grabber for a monitor name (empty = primary).
pub type GrabberFactory = Arc<dyn Fn(&str) -> Result<Box<dyn Grabber>, String> + Send + Sync>;

/// UTF-16 buffer with a trailing NUL → `String`.
pub(crate) fn from_wide(w: &[u16]) -> String {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}
