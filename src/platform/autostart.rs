//! The per-user `Run` entry that starts the app with Windows. Every function
//! works on a subkey of `HKEY_CURRENT_USER`, so tests can use their own key.

use std::io;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};

use super::wide;

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const VALUE_NAME: &str = "ObsidAmbilight";

fn check(code: WIN32_ERROR) -> io::Result<()> {
    if code == ERROR_SUCCESS { Ok(()) } else { Err(io::Error::from_raw_os_error(code as i32)) }
}

/// What the Run entry should contain for this exe.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// The value, or `None` when it (or the key) does not exist.
pub fn read(key: &str, name: &str) -> io::Result<Option<String>> {
    let (k, n) = (wide(key), wide(name));
    let mut size = 0u32;
    // SAFETY: NUL-terminated strings; the first call only asks for the size.
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            n.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(code)?;
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: `buf` holds `size` bytes.
    check(unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            n.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    })?;
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(Some(String::from_utf16_lossy(&buf[..len])))
}

pub fn write(key: &str, name: &str, value: &str) -> io::Result<()> {
    let (k, n, v) = (wide(key), wide(name), wide(value));
    let mut hkey: HKEY = std::ptr::null_mut();
    // SAFETY: valid strings and out-pointer; the key is closed below.
    unsafe {
        check(RegCreateKeyExW(
            HKEY_CURRENT_USER,
            k.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        ))?;
        let result = check(RegSetValueExW(hkey, n.as_ptr(), 0, REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32));
        RegCloseKey(hkey);
        result
    }
}

/// Removes the value; a missing value or key is fine.
pub fn remove(key: &str, name: &str) -> io::Result<()> {
    let (k, n) = (wide(key), wide(name));
    // SAFETY: NUL-terminated strings.
    let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
    if code == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(code) }
}

/// Deletes a key with everything under it (test cleanup).
pub fn delete_tree(key: &str) -> io::Result<()> {
    let k = wide(key);
    // SAFETY: NUL-terminated string.
    let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    if code == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(code) }
}

/// Makes the entry match `enabled` for `exe`, writing only when it differs.
pub fn sync(key: &str, name: &str, enabled: bool, exe: &Path) -> io::Result<()> {
    let current = read(key, name)?;
    let wanted = command_line(exe);
    match (enabled, current) {
        (true, Some(c)) if c == wanted => Ok(()),
        (true, _) => write(key, name, &wanted),
        (false, Some(_)) => remove(key, name),
        (false, None) => Ok(()),
    }
}
