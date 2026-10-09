//! Named pipe transport. The pipe name contains the session id and the user
//! SID, its DACL grants access to the current user only, and remote clients
//! are rejected. The server is synchronous: one thread per connection.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use tracing::{debug, warn};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcessToken};

use super::{MAX_MESSAGE, Request, Response, decode, encode};
use crate::platform::wide;

pub type Handler = Arc<dyn Fn(Request) -> Response + Send + Sync>;

/// `\\.\pipe\ObsidAmbilight-<session>-<user SID>`.
pub fn pipe_name() -> io::Result<String> {
    let mut session = 0u32;
    // SAFETY: valid out pointer.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(format!(r"\\.\pipe\ObsidAmbilight-{session}-{}", current_user_sid()?))
}

/// String SID of the user running this process.
fn current_user_sid() -> io::Result<String> {
    // SAFETY: standard token query sequence; every buffer is sized by the API
    // and every handle/allocation is released before returning.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0u32;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            return Err(io::Error::last_os_error());
        }
        let n = (0..).take_while(|&i| *s.add(i) != 0).count();
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        Ok(sid)
    }
}

/// Security descriptor allowing only the current user; freed on drop.
struct UserOnly(PSECURITY_DESCRIPTOR);

// SAFETY: the descriptor is immutable after creation and only read by the API.
unsafe impl Send for UserOnly {}

impl UserOnly {
    fn new() -> io::Result<Self> {
        let sddl = wide(&format!("D:P(A;;GA;;;{})", current_user_sid()?));
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: valid SDDL string, out pointer receives a LocalAlloc'ed descriptor.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(Self(sd)) }
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

impl Drop for UserOnly {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(self.0) };
    }
}

/// A pipe instance handle moved to the server thread.
struct OwnedPipe(HANDLE);

// SAFETY: a pipe handle can be used from any thread; ownership moves once.
unsafe impl Send for OwnedPipe {}

impl OwnedPipe {
    /// Taking `self` by value makes closures capture the whole wrapper.
    fn into_raw(self) -> HANDLE {
        self.0
    }
}

fn create_instance(name: &[u16], sd: &UserOnly, first: bool) -> io::Result<HANDLE> {
    let sa = sd.attributes();
    let open_mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
    // SAFETY: valid name and security attributes for the duration of the call.
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            &sa,
        )
    };
    if h == INVALID_HANDLE_VALUE { Err(io::Error::last_os_error()) } else { Ok(h) }
}

/// Starts serving `name` on a background thread. Fails if the pipe already
/// exists (another server owns it).
pub fn serve(name: String, handler: Handler) -> io::Result<()> {
    let w = wide(&name);
    let sd = UserOnly::new()?;
    let first = OwnedPipe(create_instance(&w, &sd, true)?);
    thread::Builder::new().name("ipc-server".into()).spawn(move || {
        let mut next = Some(first.into_raw());
        loop {
            let h = match next.take().map_or_else(|| create_instance(&w, &sd, false), Ok) {
                Ok(h) => h,
                Err(e) => {
                    warn!(error = %e, "cannot create pipe instance");
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };
            // SAFETY: valid pipe handle, synchronous connect.
            let connected = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) } != 0
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if !connected {
                // SAFETY: handle owned here.
                unsafe { CloseHandle(h) };
                continue;
            }
            // SAFETY: ownership of the connected handle moves into the File.
            let file = unsafe { File::from_raw_handle(h as RawHandle) };
            let handler = Arc::clone(&handler);
            let _ = thread::Builder::new().name("ipc-client".into()).spawn(move || serve_client(file, &handler));
        }
    })?;
    Ok(())
}

fn serve_client(file: File, handler: &Handler) {
    let Ok(read_half) = file.try_clone() else { return };
    let mut reader = BufReader::new(read_half);
    let mut writer = file;
    loop {
        let mut line = String::new();
        match reader.by_ref().take(MAX_MESSAGE as u64 + 1).read_line(&mut line) {
            Ok(0) => return,
            Ok(n) if n > MAX_MESSAGE => {
                warn!("IPC message too long, closing connection");
                return;
            }
            Ok(_) => {}
            Err(e) => {
                debug!(error = %e, "IPC read failed");
                return;
            }
        }
        let response = match decode::<Request>(&line) {
            Ok(req) => handler(req),
            Err(e) => Response::Error { message: format!("bad request: {e}") },
        };
        if writer.write_all(encode(&response).as_bytes()).is_err() {
            return;
        }
    }
}

pub struct Client {
    reader: BufReader<File>,
}

impl Client {
    /// Connects, waiting while all instances are busy, up to `timeout`.
    pub fn connect(name: &str, timeout: Duration) -> io::Result<Client> {
        let w = wide(name);
        let deadline = Instant::now() + timeout;
        loop {
            // SAFETY: valid name; synchronous handle.
            let h = unsafe {
                CreateFileW(
                    w.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if h != INVALID_HANDLE_VALUE {
                // SAFETY: ownership of the handle moves into the File.
                let file = unsafe { File::from_raw_handle(h as RawHandle) };
                return Ok(Client { reader: BufReader::new(file) });
            }
            let err = io::Error::last_os_error();
            let remaining = deadline.saturating_duration_since(Instant::now());
            if err.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) || remaining.is_zero() {
                return Err(err);
            }
            // SAFETY: valid name.
            unsafe { WaitNamedPipeW(w.as_ptr(), remaining.as_millis().min(u32::MAX as u128) as u32) };
        }
    }

    pub fn request(&mut self, req: &Request) -> io::Result<Response> {
        self.reader.get_mut().write_all(encode(req).as_bytes())?;
        let mut line = String::new();
        if self.reader.by_ref().take(MAX_MESSAGE as u64 + 1).read_line(&mut line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "server closed the pipe"));
        }
        decode(&line).map_err(io::Error::other)
    }
}

/// One request on a fresh connection, abandoned after `timeout` (the worker
/// thread is left behind if the server hangs; callers exit soon after).
pub fn request_with_timeout(name: &str, req: Request, timeout: Duration) -> io::Result<Response> {
    let (tx, rx) = mpsc::channel();
    let name = name.to_owned();
    thread::spawn(move || {
        let r = Client::connect(&name, timeout).and_then(|mut c| c.request(&req));
        let _ = tx.send(r);
    });
    rx.recv_timeout(timeout).unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "no IPC reply")))
}
