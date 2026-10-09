//! Background process: hidden top-level window (it will also receive session
//! and power broadcasts in stage 3), tray icon and menu, the device worker,
//! the IPC server and the settings child process.

use std::process::{Child, ExitCode};
use std::sync::Arc;
use std::time::Duration;

use tracing::{error, info, warn};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::app::commands::Command;
use crate::app::link::UdpLink;
use crate::app::state::{Connection, Snapshot, Tab};
use crate::app::worker::{Notice, Worker, WorkerOptions};
use crate::i18n::{self, Strings};
use crate::ipc::{Request, Response, pipe};
use crate::platform::instance::{INSTANCE_MUTEX, InstanceGuard};
use crate::platform::{self, paths, wide};
use crate::ui::icons;
use crate::{config, diagnostics};

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_NOTICE: u32 = WM_APP + 2;
const WM_APP_OPEN: u32 = WM_APP + 3;

const TIMER_MENU: usize = 1;
const TIMER_EXIT: usize = 2;

const ID_POWER: u32 = 1;
const ID_CUSTOM: u32 = 2;
const ID_SEARCH: u32 = 3;
const ID_SETTINGS: u32 = 4;
const ID_EXIT: u32 = 5;
const ID_PRESET: u32 = 100; // + index
const ID_BRIGHT: u32 = 200; // + percent / 10

/// `HWND` is a raw pointer; this wrapper lets worker/IPC threads post to it.
#[derive(Clone, Copy)]
struct SendHwnd(HWND);
// SAFETY: PostMessageW may be called from any thread with any HWND value.
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

impl SendHwnd {
    fn post(self, msg: u32, wparam: usize) {
        // SAFETY: posting to a possibly destroyed window is harmless.
        unsafe { PostMessageW(self.0, msg, wparam, 0) };
    }
}

pub fn run() -> ExitCode {
    let (cfg_path, log_path) = match (paths::config_path(), paths::logs_dir()) {
        (Ok(c), Ok(l)) => (c, l.join("obsid-ambilight.log")),
        _ => {
            platform::message_box("Obsid Ambilight", "%APPDATA% is not available.");
            return ExitCode::FAILURE;
        }
    };
    let pipe_name = match pipe::pipe_name() {
        Ok(n) => n,
        Err(e) => {
            platform::message_box("Obsid Ambilight", &format!("Cannot start: {e}"));
            return ExitCode::FAILURE;
        }
    };
    // Take the instance mutex before touching the config or the log: a second
    // launch must not rotate the running instance's log or move its config aside.
    let _guard = match InstanceGuard::acquire(INSTANCE_MUTEX) {
        Ok(Some(g)) => g,
        Ok(None) => {
            let r = pipe::request_with_timeout(&pipe_name, Request::OpenSettings { tab: None }, Duration::from_secs(2));
            if !matches!(r, Ok(Response::Ok)) {
                // Read-only peek at the language; `config::load` could rename a broken file.
                let language = std::fs::read_to_string(&cfg_path)
                    .ok()
                    .and_then(|t| config::parse(&t).ok())
                    .map(|l| l.config.app.language)
                    .unwrap_or_default();
                let lang = i18n::resolve(language, platform::ui_language());
                platform::message_box("Obsid Ambilight", i18n::strings(lang).already_running);
            }
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            platform::message_box("Obsid Ambilight", &format!("Cannot start: {e}"));
            return ExitCode::FAILURE;
        }
    };

    let loaded = config::load(&cfg_path);
    let _ = diagnostics::init_logging(&log_path, loaded.config.advanced.log_level.as_tracing());
    info!(version = env!("CARGO_PKG_VERSION"), "starting");

    // SAFETY: called once before any window is created.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let hwnd = match create_window() {
        Ok(h) => h,
        Err(e) => {
            error!(%e, "cannot create the tray window");
            return ExitCode::FAILURE;
        }
    };
    let target = SendHwnd(hwnd);
    let notify = Arc::new(move |n: Notice| target.post(WM_APP_NOTICE, n as usize));
    let worker = Worker::spawn(
        loaded.config,
        loaded.warnings,
        Some(cfg_path),
        Box::new(UdpLink::new()),
        notify,
        WorkerOptions::default(),
    );

    let handle = worker.handle();
    let served = pipe::serve(
        pipe_name,
        Arc::new(move |req| match req {
            Request::GetState => Response::State { snapshot: Box::new(handle.snapshot()) },
            Request::OpenSettings { tab } => {
                target.post(WM_APP_OPEN, tab.unwrap_or_default().index());
                Response::Ok
            }
            Request::Command { command } if command.allowed_over_ipc() => {
                if handle.send(command) {
                    Response::Ok
                } else {
                    Response::Error { message: "worker stopped".into() }
                }
            }
            Request::Command { .. } => Response::Error { message: "command not allowed".into() },
        }),
    );
    if let Err(e) = served {
        warn!(%e, "IPC server not started; the settings window will not connect");
    }

    let app = Box::new(TrayApp {
        hwnd,
        worker: Some(worker),
        settings: None,
        icon: std::ptr::null_mut(),
        taskbar_created: unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) },
        suppress_next_up: false,
        exiting: false,
    });
    let app = Box::into_raw(app);
    // SAFETY: the window lives on this thread; the pointer is reclaimed below.
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
        (*app).refresh_icon(true);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drop(Box::from_raw(app));
    }
    info!("stopped");
    ExitCode::SUCCESS
}

fn create_window() -> std::io::Result<HWND> {
    let class = wide("ObsidAmbilightTray");
    // SAFETY: class and window creation with valid static data.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let title = wide("Obsid Ambilight");
        // A hidden top-level window (not HWND_MESSAGE) so that broadcast
        // messages such as WM_QUERYENDSESSION reach it in stage 3.
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() { Err(std::io::Error::last_os_error()) } else { Ok(hwnd) }
    }
}

struct TrayApp {
    hwnd: HWND,
    worker: Option<Worker>,
    settings: Option<Child>,
    icon: HICON,
    taskbar_created: u32,
    suppress_next_up: bool,
    exiting: bool,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: GWLP_USERDATA holds the TrayApp pointer set in `run` (or null).
    let app = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut TrayApp;
    if app.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
    // SAFETY: only this thread touches the TrayApp.
    let app = unsafe { &mut *app };
    match msg {
        WM_APP_TRAY => {
            match (lp as u32) & 0xFFFF {
                WM_LBUTTONUP if app.suppress_next_up => app.suppress_next_up = false,
                // Delay the menu so a double click can open settings instead.
                WM_LBUTTONUP => unsafe {
                    SetTimer(hwnd, TIMER_MENU, GetDoubleClickTime(), None);
                },
                WM_LBUTTONDBLCLK => {
                    unsafe { KillTimer(hwnd, TIMER_MENU) };
                    app.suppress_next_up = true;
                    app.open_settings(Tab::Device);
                }
                WM_RBUTTONUP => app.show_menu(),
                _ => {}
            }
            0
        }
        WM_APP_NOTICE => {
            match Notice::from_usize(wp) {
                Some(Notice::Status) => app.refresh_icon(false),
                Some(Notice::OpenSettings) => app.open_settings(Tab::Device),
                Some(Notice::Stopped) => app.finish_exit(),
                None => {}
            }
            0
        }
        WM_APP_OPEN => {
            app.open_settings(Tab::from_index(wp));
            0
        }
        WM_TIMER => {
            unsafe { KillTimer(hwnd, wp) };
            match wp {
                TIMER_MENU => app.show_menu(),
                TIMER_EXIT => app.finish_exit(),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            0
        }
        m if m == app.taskbar_created => {
            app.refresh_icon(true);
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

impl TrayApp {
    fn snapshot(&self) -> Option<Snapshot> {
        self.worker.as_ref().map(Worker::snapshot)
    }

    fn strings(snap: &Snapshot) -> &'static Strings {
        i18n::strings(i18n::resolve(snap.config.app.language, platform::ui_language()))
    }

    fn send(&self, cmd: Command) {
        if let Some(w) = &self.worker {
            w.send(cmd);
        }
    }

    /// Updates icon and tooltip; `add` re-registers the icon (start, Explorer restart).
    fn refresh_icon(&mut self, add: bool) {
        let Some(snap) = self.snapshot() else { return };
        let s = Self::strings(&snap);
        // SAFETY: plain metric query.
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
        let new_icon = create_icon(&icons::render(icons::kind_for(&snap), size), size);
        let mut nid = self.notify_data();
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        nid.hIcon = new_icon;
        let tip = wide(&tooltip(&snap, s));
        let n = tip.len().min(nid.szTip.len()) - 1;
        nid.szTip[..n].copy_from_slice(&tip[..n]);
        // SAFETY: fully initialised NOTIFYICONDATAW.
        unsafe {
            if add || Shell_NotifyIconW(NIM_MODIFY, &nid) == 0 {
                Shell_NotifyIconW(NIM_ADD, &nid);
            }
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
            }
        }
        self.icon = new_icon;
    }

    fn notify_data(&self) -> NOTIFYICONDATAW {
        // SAFETY: zero is a valid NOTIFYICONDATAW bit pattern.
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = 1;
        nid
    }

    fn show_menu(&mut self) {
        let Some(snap) = self.snapshot() else { return };
        if self.exiting {
            return;
        }
        let s = Self::strings(&snap);
        let mut bitmaps: Vec<HBITMAP> = Vec::new();
        // SAFETY: menu handles are created, used and destroyed on this thread.
        let cmd = unsafe {
            let menu = CreatePopupMenu();
            append(menu, MF_STRING | MF_GRAYED, 0, &status_line(&snap, s));
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            let power_label = if snap.config.light.power { s.power_off_item } else { s.power_on_item };
            append(menu, MF_STRING, ID_POWER as usize, power_label);

            let colors = CreatePopupMenu();
            for (i, c) in snap.config.light.presets.iter().enumerate() {
                let id = ID_PRESET + i as u32;
                let checked = if *c == snap.config.light.color { MF_CHECKED } else { 0 };
                append(colors, MF_STRING | checked, id as usize, &c.to_string());
                let bmp = swatch(*c);
                if !bmp.is_null() {
                    let mii = MENUITEMINFOW {
                        cbSize: size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_BITMAP,
                        hbmpItem: bmp,
                        ..std::mem::zeroed()
                    };
                    SetMenuItemInfoW(colors, id, 0, &mii);
                    bitmaps.push(bmp);
                }
            }
            AppendMenuW(colors, MF_SEPARATOR, 0, std::ptr::null());
            append(colors, MF_STRING, ID_CUSTOM as usize, s.custom_color);
            append(menu, MF_POPUP, colors as usize, s.color_menu);

            let bright = CreatePopupMenu();
            for step in 1..=10u32 {
                let pct = step * 10;
                let checked = if u32::from(snap.config.light.brightness) == pct { MF_CHECKED } else { 0 };
                append(bright, MF_STRING | checked, (ID_BRIGHT + step) as usize, &format!("{pct} %"));
            }
            append(menu, MF_POPUP, bright as usize, s.brightness_menu);

            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            append(menu, MF_STRING, ID_SEARCH as usize, s.search_again);
            append(menu, MF_STRING, ID_SETTINGS as usize, s.settings_item);
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            append(menu, MF_STRING, ID_EXIT as usize, s.exit_item);

            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            // Required so the menu closes when the user clicks elsewhere.
            SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
                pt.x,
                pt.y,
                0,
                self.hwnd,
                std::ptr::null(),
            );
            PostMessageW(self.hwnd, WM_NULL, 0, 0);
            DestroyMenu(menu);
            for b in bitmaps {
                DeleteObject(b);
            }
            cmd as u32
        };
        self.on_menu(cmd, &snap);
    }

    fn on_menu(&mut self, id: u32, snap: &Snapshot) {
        match id {
            ID_POWER => self.send(Command::SetPower { on: !snap.config.light.power }),
            ID_CUSTOM => self.open_settings(Tab::Lighting),
            ID_SEARCH => self.send(Command::Discover),
            ID_SETTINGS => self.open_settings(Tab::Device),
            ID_EXIT => self.begin_exit(),
            id if (ID_PRESET..ID_PRESET + 100).contains(&id) => {
                if let Some(c) = snap.config.light.presets.get((id - ID_PRESET) as usize) {
                    self.send(Command::SetStatic { color: Some(*c), brightness: None });
                }
            }
            id if (ID_BRIGHT + 1..=ID_BRIGHT + 10).contains(&id) => {
                self.send(Command::SetStatic { color: None, brightness: Some(((id - ID_BRIGHT) * 10) as u8) });
            }
            _ => {}
        }
    }

    fn open_settings(&mut self, tab: Tab) {
        if self.exiting {
            return;
        }
        if let Some(child) = &mut self.settings
            && matches!(child.try_wait(), Ok(None))
        {
            bring_to_front(child.id());
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                error!(%e, "cannot locate own executable");
                return;
            }
        };
        match std::process::Command::new(exe).args(["--settings", "--tab", tab.as_arg()]).spawn() {
            Ok(child) => {
                // SAFETY: plain call; lets the new process take the foreground.
                unsafe { AllowSetForegroundWindow(child.id()) };
                self.settings = Some(child);
            }
            Err(e) => error!(%e, "cannot start the settings window"),
        }
    }

    fn begin_exit(&mut self) {
        if self.exiting {
            return;
        }
        self.exiting = true;
        info!("exit requested");
        self.send(Command::Exit);
        // SAFETY: timer on our own window; fires if the worker never reports back.
        unsafe { SetTimer(self.hwnd, TIMER_EXIT, 2000, None) };
    }

    fn finish_exit(&mut self) {
        if let Some(mut child) = self.settings.take() {
            let _ = child.kill();
        }
        if let Some(w) = self.worker.take() {
            w.join(Duration::from_millis(500));
        }
        let nid = self.notify_data();
        // SAFETY: removing our own icon, destroying our own window.
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &nid);
            if !self.icon.is_null() {
                DestroyIcon(self.icon);
                self.icon = std::ptr::null_mut();
            }
            DestroyWindow(self.hwnd);
        }
    }
}

fn status_line(snap: &Snapshot, s: &Strings) -> String {
    match snap.connection {
        Connection::Ready => {
            let ip = snap.device.as_ref().map(|d| d.ip.to_string()).unwrap_or_default();
            i18n::fill(s.status_connected, &[&ip])
        }
        Connection::Connecting => s.status_connecting.to_owned(),
        Connection::NoDevice if snap.searching => s.status_connecting.to_owned(),
        Connection::NoDevice => s.status_no_device.to_owned(),
        Connection::Offline => s.status_offline.to_owned(),
    }
}

fn tooltip(snap: &Snapshot, s: &Strings) -> String {
    let body = match snap.connection {
        Connection::Ready => {
            let ip = snap.device.as_ref().map(|d| d.ip.to_string()).unwrap_or_default();
            let l = &snap.config.light;
            if l.power {
                format!("{} {} · {}% · {ip}", s.mode_static, l.color, l.brightness)
            } else {
                format!("{} · {ip}", s.mode_off)
            }
        }
        _ => status_line(snap, s),
    };
    format!("Obsid Ambilight — {body}")
}

unsafe fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str) {
    let w = wide(text);
    // SAFETY: menu handle valid, text NUL-terminated (copied by the API).
    unsafe { AppendMenuW(menu, flags, id, w.as_ptr()) };
}

/// Opaque square in `c`, sized for menu check marks.
fn swatch(c: crate::protocol::Rgb) -> HBITMAP {
    // SAFETY: metric query.
    let size = unsafe { GetSystemMetrics(SM_CXMENUCHECK) }.max(12) as u32;
    let px = [c.b, c.g, c.r, 255u8];
    let pixels: Vec<u8> = (0..size * size).flat_map(|_| px).collect();
    dib(&pixels, size)
}

/// 32-bpp top-down DIB from BGRA pixels.
fn dib(bgra: &[u8], size: u32) -> HBITMAP {
    // SAFETY: bitmap header fully initialised; the bits pointer is only written
    // within the allocated size.
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(std::ptr::null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if !bmp.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast::<u8>(), bgra.len());
        }
        bmp
    }
}

/// HICON from RGBA pixels (alpha channel used, monochrome mask unused).
fn create_icon(rgba: &[u8], size: u32) -> HICON {
    let bgra: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
    let color = dib(&bgra, size);
    // SAFETY: mask bitmap of the right size; both bitmaps deleted after the
    // icon has copied them.
    unsafe {
        let mask_bits = vec![0u8; (size.div_ceil(16) * 2 * size) as usize];
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, mask_bits.as_ptr().cast());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

/// Shows and focuses the visible top-level window of process `pid`.
fn bring_to_front(pid: u32) {
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> windows_sys::core::BOOL {
        let pid = lp as u32;
        let mut owner = 0u32;
        // SAFETY: valid window handle from EnumWindows.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
            if owner == pid && IsWindowVisible(hwnd) != 0 {
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                SetForegroundWindow(hwnd);
                return 0;
            }
        }
        1
    }
    // SAFETY: callback only reads the pid passed by value.
    unsafe { EnumWindows(Some(each), pid as LPARAM) };
}
