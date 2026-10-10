//! Thin Windows helpers shared by the tray and the settings window.

pub mod autostart;
pub mod instance;
pub mod paths;

use std::path::Path;

/// UTF-16 with a trailing NUL, for `PCWSTR` parameters.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Windows UI language as a LANGID (e.g. 0x0419 for ru-RU).
pub fn ui_language() -> u16 {
    // SAFETY: no arguments, no preconditions.
    unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() }
}

/// Modal warning box without an owner window.
pub fn message_box(title: &str, text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONWARNING, MB_OK, MessageBoxW};
    let (t, c) = (wide(text), wide(title));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_OK | MB_ICONWARNING);
    }
}

/// Opens a folder in Explorer (best effort).
pub fn open_folder(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}
