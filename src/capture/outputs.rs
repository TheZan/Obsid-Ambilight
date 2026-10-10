//! DXGI outputs attached to the desktop, on every adapter.

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput,
};

use super::from_wide;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorInfo {
    /// DXGI/GDI name, e.g. `\\.\DISPLAY1`.
    pub name: String,
    pub adapter: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

pub(crate) struct FoundOutput {
    pub adapter: IDXGIAdapter1,
    pub output: IDXGIOutput,
    pub name: String,
    pub adapter_name: String,
    pub rect: RECT,
}

impl FoundOutput {
    /// The primary monitor has its top-left corner at the desktop origin.
    pub fn is_primary(&self) -> bool {
        self.rect.left == 0 && self.rect.top == 0
    }

    fn info(&self) -> MonitorInfo {
        MonitorInfo {
            name: self.name.clone(),
            adapter: self.adapter_name.clone(),
            width: (self.rect.right - self.rect.left).max(0) as u32,
            height: (self.rect.bottom - self.rect.top).max(0) as u32,
            primary: self.is_primary(),
        }
    }
}

pub(crate) fn enumerate() -> windows::core::Result<Vec<FoundOutput>> {
    // SAFETY: plain COM factory creation and enumeration on this thread.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut found = Vec::new();
        for a in 0.. {
            let adapter = match factory.EnumAdapters1(a) {
                Ok(x) => x,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(e),
            };
            let adapter_name = from_wide(&adapter.GetDesc1()?.Description);
            for o in 0.. {
                let output = match adapter.EnumOutputs(o) {
                    Ok(x) => x,
                    Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(e) => return Err(e),
                };
                let d = output.GetDesc()?;
                if !d.AttachedToDesktop.as_bool() {
                    continue;
                }
                found.push(FoundOutput {
                    adapter: adapter.clone(),
                    output,
                    name: from_wide(&d.DeviceName),
                    adapter_name: adapter_name.clone(),
                    rect: d.DesktopCoordinates,
                });
            }
        }
        Ok(found)
    }
}

/// Monitors for the settings window.
pub fn list() -> Result<Vec<MonitorInfo>, String> {
    enumerate().map(|all| all.iter().map(FoundOutput::info).collect()).map_err(|e| e.message())
}
