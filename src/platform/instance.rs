//! Single-instance guard backed by a named mutex.

use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::CreateMutexW;

use super::wide;

/// Session-local name: one instance per logged-in user session.
pub const INSTANCE_MUTEX: &str = "Local\\ObsidAmbilight.Instance";

/// Holds the mutex for the life of the process.
pub struct InstanceGuard(HANDLE);

// SAFETY: a mutex handle may be closed from any thread.
unsafe impl Send for InstanceGuard {}

impl InstanceGuard {
    /// `Ok(None)` when another process already holds `name`.
    pub fn acquire(name: &str) -> io::Result<Option<Self>> {
        let w = wide(name);
        // SAFETY: valid NUL-terminated name, default security, not initially owned.
        let h = unsafe { CreateMutexW(std::ptr::null(), 0, w.as_ptr()) };
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: read immediately after the call that set it.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: we own this handle.
            unsafe { CloseHandle(h) };
            return Ok(None);
        }
        Ok(Some(Self(h)))
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        // SAFETY: handle owned by this guard, closed once.
        unsafe { CloseHandle(self.0) };
    }
}
