# Stage 3 — Windows Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The strip goes dark when Windows locks, sleeps, turns the display off, shuts down or logs off, comes back to the user's mode afterwards, and the app starts with Windows.

**Architecture:** The tray's hidden window subscribes to session, power and display notifications; a pure mapper turns messages into lifecycle events, and the tray turns them into worker commands (`Darken`/`Undarken`/`EndSession`), waiting for the worker before sleep and session end. The pure `Controller` keeps a set of "dark" reasons that overrides the desired strip state without touching the config. Autostart is a `HKCU\…\Run` value synced by the worker through an injected function.

**Tech Stack:** Rust 2024 (1.95), `windows-sys` 0.61 (+ `Win32_System_Power`, `Win32_System_Registry`, `Win32_System_SystemServices`), existing app modules.

**Spec:** `docs/superpowers/specs/2026-10-10-stage3-lifecycle-design.md`

## Global Constraints

- Dark reasons: `Sleep`, `Locked`, `DisplayOff`; while any is set the strip is off; events never change `config.toml` (power, mode, colour stay as the user chose).
- Lock/unlock: `WTS_SESSION_LOCK` → `Darken(Locked)`, `WTS_SESSION_UNLOCK` → `Undarken(Locked)` and `Undarken(Sleep)`.
- Display: `GUID_CONSOLE_DISPLAY_STATE` 0 → `Darken(DisplayOff)`; 1 or 2 → `Undarken(DisplayOff)`.
- Sleep: `PBT_APMSUSPEND` → `Darken(Sleep)`, wait ≤ 1.5 s; `PBT_APMRESUMEAUTOMATIC` / `PBT_APMRESUMESUSPEND` → `Undarken(Sleep)`.
- Session end: `WM_QUERYENDSESSION` → TRUE without delay; `WM_ENDSESSION` (wParam TRUE) → `EndSession`, wait ≤ 2 s, then remove the icon and exit. `EndSession` always turns the strip off, whatever `exit_action` is.
- Autostart: value `ObsidAmbilight` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, data = quoted full exe path; `[startup] autostart = true` by default (missing key → true); synced at every background start; only the background process writes the registry.
- Tests never touch the real Run key: registry tests use `HKCU\Software\ObsidAmbilightTest\…` and delete it.
- Logs and controller errors in English; UI strings in `Strings` RU/EN.
- Commits: one per task, subject line only, no body, no `Co-Authored-By`.
- Every task ends with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green.

## Review Focus

1. **Sleep while streaming and the network drops at the same moment:** leaving direct mode fails, the app goes Offline; after resume the strip must come back in Ambilight, not stay frozen or off. Pinned in Task 2 (`sleep_with_a_failed_stop_resumes_after_reconnect`).
2. **A cancelled shutdown** (`WM_ENDSESSION` with wParam FALSE, e.g. an app blocked logoff): the app must keep running. Pinned in Task 4 (`a_cancelled_session_end_is_ignored`).
3. **The exe moved or rebuilt elsewhere:** the Run value must follow the current path at the next start. Pinned in Task 1 (`sync_updates_a_stale_path`).
4. **The user changes colour or power while locked** (settings window over remote desktop, a stale tray menu): saved, nothing sent, the right state after unlock. Pinned in Task 2 (`changes_while_dark_apply_after_it`).
5. **Writing the Run key fails** (policy, permissions): the app keeps running and shows the error. Pinned in Task 2 (`autostart_failure_is_reported`) and Task 3 (`a_failing_autostart_is_reported_in_the_snapshot`).

---

## File Structure

```text
Cargo.toml                       windows-sys + Win32_System_Power, Win32_System_Registry, Win32_System_SystemServices (Task 1)
src/config.rs                    StartupConfig.autostart (Task 1)
src/platform/autostart.rs        NEW Run value: read/write/remove/sync, test cleanup (Task 1)
src/platform/mod.rs              pub mod autostart (Task 1)
src/app/state.rs                 Dark, Snapshot.dark (Task 2)
src/app/commands.rs              Darken, Undarken, EndSession, SetAutostart; IPC filter (Task 2)
src/app/controller.rs            dark set, EndSession, SyncAutostart/AutostartSynced (Task 2)
src/app/worker.rs                send_and_wait, AutostartFn, SyncAutostart action (Task 3)
src/ui/lifecycle.rs              NEW messages → LifecycleEvent → commands (Task 4)
src/ui/mod.rs                    pub mod lifecycle (Task 4)
src/ui/tray.rs                   autostart fn (Task 3); subscriptions, wndproc, on_lifecycle, dark tooltip (Task 5)
src/ui/icons.rs                  Off icon while dark (Task 5)
src/i18n.rs                      strings + dark_reasons() (Tasks 5, 6)
src/ui/settings/general.rs       autostart checkbox (Task 6)
src/ui/settings/diagnostics.rs   «Пауза» row, summary line (Task 6)
tests/autostart_tests.rs, lifecycle_tests.rs    NEW
tests/config_tests.rs, controller_tests.rs, worker_tests.rs, icons_tests.rs, tray_tests.rs, settings_tests.rs   extended
README.md, docs/testing.md       Stage 3 (Task 7)
```

---

### Task 1: Autostart setting and the Run key

**Files:**
- Create: `src/platform/autostart.rs`, `tests/autostart_tests.rs`
- Modify: `Cargo.toml`, `src/platform/mod.rs`, `src/config.rs`, `tests/config_tests.rs`

**Interfaces:**
- Produces: `config::StartupConfig.autostart: bool` (default `true`); `platform::autostart::{RUN_KEY, VALUE_NAME, command_line(&Path) -> String, read(&str, &str) -> io::Result<Option<String>>, write(&str, &str, &str) -> io::Result<()>, remove(&str, &str) -> io::Result<()>, delete_tree(&str) -> io::Result<()>, sync(&str, &str, bool, &Path) -> io::Result<()>}` — `key` is a subkey of `HKEY_CURRENT_USER`.

- [ ] **Step 1: Add the features**

In `Cargo.toml`, add to the `windows-sys` feature list (keep it sorted): `"Win32_System_Power"`, `"Win32_System_Registry"`, `"Win32_System_SystemServices"`. Run `cargo build`; expected: builds.

- [ ] **Step 2: Write the failing tests**

Create `tests/autostart_tests.rs`:

```rust
use std::path::Path;

use obsid_ambilight::platform::autostart::{self, command_line};

/// A throw-away key per test under HKCU\Software\ObsidAmbilightTest.
struct TestKey(String);

impl TestKey {
    fn new(name: &str) -> Self {
        let k = Self(format!(r"Software\ObsidAmbilightTest\{name}-{}\Run", std::process::id()));
        let _ = autostart::delete_tree(&k.0);
        k
    }
}

impl Drop for TestKey {
    fn drop(&mut self) {
        let _ = autostart::delete_tree(&self.0);
    }
}

#[test]
fn command_line_quotes_the_path() {
    assert_eq!(command_line(Path::new(r"C:\Program Files\Obsid\a.exe")), r#""C:\Program Files\Obsid\a.exe""#);
}

#[test]
fn values_are_written_read_and_removed() {
    let key = TestKey::new("rw");
    assert_eq!(autostart::read(&key.0, "App").unwrap(), None, "missing key reads as none");
    autostart::write(&key.0, "App", r#""C:\x\a.exe""#).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap().as_deref(), Some(r#""C:\x\a.exe""#));
    autostart::remove(&key.0, "App").unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), None);
    autostart::remove(&key.0, "App").unwrap();
}

#[test]
fn sync_adds_and_removes_the_entry() {
    let key = TestKey::new("sync");
    let exe = Path::new(r"C:\Apps\obsid-ambilight.exe");
    autostart::sync(&key.0, "App", true, exe).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), Some(command_line(exe)));
    autostart::sync(&key.0, "App", false, exe).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), None);
}

#[test]
fn sync_updates_a_stale_path() {
    let key = TestKey::new("stale");
    autostart::write(&key.0, "App", r#""C:\Old\obsid-ambilight.exe""#).unwrap();
    let exe = Path::new(r"C:\New\obsid-ambilight.exe");
    autostart::sync(&key.0, "App", true, exe).unwrap();
    assert_eq!(autostart::read(&key.0, "App").unwrap(), Some(command_line(exe)));
}
```

Append to `tests/config_tests.rs`:

```rust
#[test]
fn autostart_defaults_to_on_and_round_trips() {
    assert!(config::parse(SPEC_EXAMPLE).unwrap().config.startup.autostart, "stage 1/2 files turn it on");
    let l = config::parse("[startup]\nautostart = false\n").unwrap();
    assert!(!l.config.startup.autostart);
    assert!(l.warnings.is_empty());
    let dir = common::temp_dir("autostart");
    let path = dir.join("config.toml");
    let mut c = Config::default();
    c.startup.autostart = false;
    config::save(&path, &c).unwrap();
    assert_eq!(config::load(&path).config, c);
}
```

Run: `cargo test --test autostart_tests --test config_tests`
Expected: compile errors (`platform::autostart`, `startup.autostart`).

- [ ] **Step 3: Config field**

In `src/config.rs`, add to `StartupConfig`:

```rust
    /// Keep the `HKCU\…\Run` entry so the app starts with Windows.
    pub autostart: bool,
```

Default: `startup: StartupConfig { power_on: true, autostart: true },`. Parse:

```rust
        startup: StartupConfig {
            power_on: r.field("startup", "power_on", d.startup.power_on, Value::as_bool),
            autostart: r.field("startup", "autostart", d.startup.autostart, Value::as_bool),
        },
```

- [ ] **Step 4: The registry module**

Create `src/platform/autostart.rs`:

```rust
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
        RegGetValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
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
```

Add `pub mod autostart;` to `src/platform/mod.rs`. Note for the implementer: `RegDeleteTreeW` deletes the named key and its subtree; if a signature differs from the code (pointer casts), follow the compiler and `~/.cargo/registry/src/*/windows-sys-0.61.2/src/Windows/Win32/System/Registry/mod.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --test autostart_tests --test config_tests`
Expected: PASS. Then check the real Run key was not touched: `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run` shows no `ObsidAmbilight` entry added by the tests, and `reg query HKCU\Software\ObsidAmbilightTest` reports the key does not exist (an empty parent may remain; delete it with `reg delete HKCU\Software\ObsidAmbilightTest /f` if so).

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add Cargo.toml Cargo.lock src/config.rs src/platform tests/autostart_tests.rs tests/config_tests.rs
git commit -m "Add the autostart setting and the Run key helpers"
```

---

### Task 2: Controller — dark reasons, session end, autostart sync

**Files:**
- Modify: `src/app/state.rs`, `src/app/commands.rs`, `src/app/controller.rs`, `src/app/worker.rs` (temporary arm), `tests/controller_tests.rs`

**Interfaces:**
- Consumes: Task 1 (`StartupConfig.autostart`).
- Produces:
  - `state::Dark { Sleep, Locked, DisplayOff }` (`Copy`, `Ord`, serde snake_case); `Snapshot.dark: Vec<Dark>` (sorted).
  - `Command::{Darken { reason: Dark }, Undarken { reason: Dark }, EndSession, SetAutostart { enabled: bool }}`; `allowed_over_ipc()` is false for `Exit`, `EndSession`, `Darken`, `Undarken`.
  - `Action::SyncAutostart { enabled: bool }`; `Event::AutostartSynced(Result<(), String>)`.
  - Startup emits `SyncAutostart { enabled: cfg.startup.autostart }` first.

- [ ] **Step 1: Write the failing tests**

In `tests/controller_tests.rs` add `Dark` to the `app::state` import, then append:

```rust
// ---------- Windows lifecycle ----------

fn cmd(c: &mut Controller, command: Command, at: Instant) -> Vec<Action> {
    c.handle(Event::Command(command), at)
}

#[test]
fn startup_syncs_autostart_first() {
    let mut cfg = saved_cfg();
    let a = ctrl(cfg.clone()).handle(Event::Startup, Instant::now());
    assert_eq!(a.first(), Some(&Action::SyncAutostart { enabled: true }));
    cfg.startup.autostart = false;
    let a = ctrl(cfg).handle(Event::Startup, Instant::now());
    assert_eq!(a.first(), Some(&Action::SyncAutostart { enabled: false }));
}

#[test]
fn set_autostart_saves_and_syncs() {
    let (mut c, now) = ready();
    let a = cmd(&mut c, Command::SetAutostart { enabled: false }, now);
    assert_eq!(a, vec![Action::SaveConfig, Action::SyncAutostart { enabled: false }]);
    assert!(!c.config().startup.autostart);
}

#[test]
fn autostart_failure_is_reported() {
    let (mut c, now) = ready();
    assert!(c.handle(Event::AutostartSynced(Err("access denied".into())), now).is_empty());
    assert!(c.snapshot(false).errors.iter().any(|e| e.contains("autostart") && e.contains("access denied")));
}

#[test]
fn locking_turns_a_static_strip_off_and_unlocking_restores_it() {
    let (mut c, now) = ready();
    let before = c.config().clone();
    assert_eq!(cmd(&mut c, Command::Darken { reason: Dark::Locked }, now), vec![Action::Apply { ip: IP, change: LightChange::off() }]);
    assert_eq!(c.snapshot(false).dark, vec![Dark::Locked]);
    assert!(cmd(&mut c, Command::Darken { reason: Dark::Locked }, now).is_empty(), "repeats are ignored");
    assert_eq!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now), vec![Action::Apply { ip: IP, change: full_on(c.config()) }]);
    assert!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now).is_empty());
    assert_eq!(c.config(), &before, "Windows events never change the config");
}

#[test]
fn locking_stops_ambilight_and_unlocking_restarts_it() {
    let (mut c, now) = streaming();
    let a = cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: LightChange::off() }]);
    assert!(!c.capture_wanted());
    c.handle(Event::StreamStopped(IP, Ok(())), now);
    assert_eq!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now), vec![Action::StartStream { ip: IP }]);
}

#[test]
fn the_strip_stays_dark_until_every_reason_is_gone() {
    let (mut c, now) = ready();
    cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    assert!(cmd(&mut c, Command::Darken { reason: Dark::DisplayOff }, now).is_empty());
    assert!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now).is_empty(), "display still off");
    assert_eq!(c.snapshot(false).dark, vec![Dark::DisplayOff]);
    let a = cmd(&mut c, Command::Undarken { reason: Dark::DisplayOff }, now);
    assert_eq!(a, vec![Action::Apply { ip: IP, change: full_on(c.config()) }]);
}

#[test]
fn changes_while_dark_apply_after_it() {
    let (mut c, now) = ready();
    cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    let a = cmd(&mut c, Command::SetStatic { color: Some(Rgb::new(5, 6, 7)), brightness: None }, now);
    assert_eq!(a, vec![Action::SaveConfig], "saved, not sent");
    assert_eq!(cmd(&mut c, Command::SetMode { mode: Mode::Ambilight }, now), vec![Action::SaveConfig]);
    assert_eq!(cmd(&mut c, Command::Undarken { reason: Dark::Locked }, now), vec![Action::StartStream { ip: IP }]);
    assert_eq!(c.config().light.color, Rgb::new(5, 6, 7));
}

#[test]
fn darkness_while_offline_is_applied_after_reconnect() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: true }), now);
    c.handle(Event::Applied(Err("timeout".into())), now);
    assert!(cmd(&mut c, Command::Darken { reason: Dark::Sleep }, now).is_empty(), "nothing to send offline");
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    assert_eq!(a, vec![Action::Apply { ip: IP, change: LightChange::off() }]);
}

#[test]
fn sleep_with_a_failed_stop_resumes_after_reconnect() {
    let (mut c, now) = streaming();
    cmd(&mut c, Command::Darken { reason: Dark::Sleep }, now);
    c.handle(Event::StreamStopped(IP, Err("network down".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    assert!(cmd(&mut c, Command::Undarken { reason: Dark::Sleep }, now).is_empty(), "still offline");
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    assert_eq!(a, vec![Action::StartStream { ip: IP }]);
}

#[test]
fn highlights_are_refused_and_ended_while_dark() {
    let (mut c, now) = ready();
    cmd(&mut c, Command::Highlight { target: HighlightTarget::Led(1) }, now);
    c.handle(Event::StreamStarted(Ok(())), now);
    let a = cmd(&mut c, Command::Darken { reason: Dark::Locked }, now);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: LightChange::off() }], "the overlay ends");
    assert_eq!(c.highlight(), None);
    assert!(cmd(&mut c, Command::Highlight { target: HighlightTarget::Led(1) }, now).is_empty());
    assert!(c.snapshot(false).errors.iter().any(|e| e.contains("highlight")));
}

#[test]
fn ending_the_session_always_turns_the_strip_off() {
    let (mut c, now) = ready();
    assert_eq!(c.config().app.exit_action, ExitAction::Keep);
    assert_eq!(cmd(&mut c, Command::EndSession, now), vec![Action::Shutdown { power_off_ip: Some(IP), stop_stream: None }]);
    let (mut c, now) = streaming();
    assert_eq!(
        cmd(&mut c, Command::EndSession, now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: Some((IP, LightChange::off())) }]
    );
    let mut c = ctrl(saved_cfg());
    assert_eq!(cmd(&mut c, Command::EndSession, Instant::now()), vec![Action::Shutdown { power_off_ip: None, stop_stream: None }]);
}

#[test]
fn lifecycle_commands_are_tray_only() {
    for c in [Command::EndSession, Command::Darken { reason: Dark::Sleep }, Command::Undarken { reason: Dark::Sleep }] {
        assert!(!c.allowed_over_ipc(), "{c:?}");
    }
    assert!(Command::SetAutostart { enabled: true }.allowed_over_ipc());
}
```

Run: `cargo test --test controller_tests`
Expected: compile errors (`Dark`, `Command::Darken`, `Action::SyncAutostart`, …).

- [ ] **Step 2: State**

In `src/app/state.rs` before `Snapshot` add:

```rust
/// Why the strip is dark regardless of the user's choice (Windows events).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dark {
    Sleep,
    Locked,
    DisplayOff,
}
```

and add `pub dark: Vec<Dark>,` after `pub stream: StreamStatus,` in `Snapshot`.

- [ ] **Step 3: Commands**

In `src/app/commands.rs` import `super::state::Dark`, add before `Exit`:

```rust
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
```

and change `allowed_over_ipc` to:

```rust
    pub fn allowed_over_ipc(&self) -> bool {
        !matches!(self, Self::Exit | Self::EndSession | Self::Darken { .. } | Self::Undarken { .. })
    }
```

- [ ] **Step 4: Controller**

In `src/app/controller.rs`:

1. Imports: `use std::collections::{BTreeSet, VecDeque};` and add `Dark` to the `super::state` import.
2. `Event`: add `/// Result of `Action::SyncAutostart`. AutostartSynced(Result<(), String>),`.
3. `Action`: add `/// Make the Windows Run entry match the setting. SyncAutostart { enabled: bool },`.
4. Field `/// Windows reasons that keep the strip dark. dark: BTreeSet<Dark>,` initialised with `BTreeSet::new()`.
5. In `snapshot`: `dark: self.dark.iter().copied().collect(),`.
6. In `handle`:

```rust
            Event::AutostartSynced(Ok(())) => Vec::new(),
            Event::AutostartSynced(Err(e)) => {
                self.record_error(format!("could not update autostart: {e}"));
                Vec::new()
            }
```

7. In `startup`, start with `let mut actions = vec![Action::SyncAutostart { enabled: self.cfg.startup.autostart }];` instead of `Vec::new()`.
8. `desired()` begins with:

```rust
        if !self.dark.is_empty() {
            return Desired::Light(LightChange::off());
        }
```

9. `save_and_drive` sends nothing while dark:

```rust
    fn save_and_drive(&mut self) -> Vec<Action> {
        let mut actions = vec![Action::SaveConfig];
        // While Windows keeps the strip dark, the change waits for the light.
        if self.dark.is_empty() {
            let desired = self.desired();
            actions.extend(self.drive(desired));
        }
        actions
    }
```

10. `start_highlight` refuses while dark (first lines):

```rust
        if !self.dark.is_empty() {
            self.record_error("cannot highlight LEDs while Windows keeps the strip dark".to_owned());
            return Vec::new();
        }
```

11. New command arms (before `Command::Exit`):

```rust
            Command::Darken { reason } => {
                let first = self.dark.is_empty();
                if !self.dark.insert(reason) || !first {
                    return Vec::new();
                }
                self.highlight = None;
                let desired = self.desired();
                self.drive(desired)
            }
            Command::Undarken { reason } => {
                if !self.dark.remove(&reason) || !self.dark.is_empty() {
                    return Vec::new();
                }
                let desired = self.desired();
                self.drive(desired)
            }
            Command::EndSession => {
                let (power_off_ip, stop_stream) = match (self.streaming, self.ready_ip()) {
                    (true, Some(ip)) => (None, Some((ip, LightChange::off()))),
                    (false, Some(ip)) => (Some(ip), None),
                    _ => (None, None),
                };
                vec![Action::Shutdown { power_off_ip, stop_stream }]
            }
            Command::SetAutostart { enabled } => {
                self.cfg.startup.autostart = enabled;
                vec![Action::SaveConfig, Action::SyncAutostart { enabled }]
            }
```

- [ ] **Step 5: Keep the worker compiling**

In `src/app/worker.rs` `process`, add a temporary arm (replaced in Task 3):

```rust
                Action::SyncAutostart { .. } => queue.push_back(Event::AutostartSynced(Ok(()))),
```

- [ ] **Step 6: Run the tests**

Run: `cargo test`
Expected: PASS. Existing tests comparing the whole `Startup` action list get `Action::SyncAutostart { enabled: true }` at the front if any fail.

- [ ] **Step 7: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/app tests/controller_tests.rs
git commit -m "Keep the strip dark for Windows lock, sleep and display off"
```

---

### Task 3: Worker — waiting for commands and the autostart sync

**Files:**
- Modify: `src/app/worker.rs`, `src/ui/tray.rs` (spawn call), `tests/worker_tests.rs`

**Interfaces:**
- Consumes: Task 1 (`autostart::{sync, RUN_KEY, VALUE_NAME}`), Task 2 (`Action::SyncAutostart`, `Event::AutostartSynced`, `Command::Darken`).
- Produces: `worker::AutostartFn = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>`; `Worker::spawn(config, warnings, config_path, link, grabbers, autostart: AutostartFn, notify, options)`; `WorkerHandle::send_and_wait(&self, Command, Duration) -> bool`.

- [ ] **Step 1: Write the failing tests**

In `tests/worker_tests.rs`:

1. Add `use obsid_ambilight::app::state::Dark;` and `use obsid_ambilight::app::worker::AutostartFn;`.
2. Add to `Script`: `/// Delay inside `apply` (a busy worker). apply_delay: Duration,` and at the start of `FakeLink::apply` (before locking for the push) add:

```rust
        let delay = self.0.lock().unwrap().apply_delay;
        std::thread::sleep(delay);
```

3. Replace `spawn_with` so it also takes the autostart function, and add helpers:

```rust
fn spawn_with(
    cfg: Config,
    link: FakeLink,
    path: Option<std::path::PathBuf>,
    grabbers: GrabberFactory,
) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    spawn_full(cfg, link, path, grabbers, Arc::new(|_| Ok(())))
}

fn spawn_full(
    cfg: Config,
    link: FakeLink,
    path: Option<std::path::PathBuf>,
    grabbers: GrabberFactory,
    autostart: AutostartFn,
) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    let notices = Arc::new(Mutex::new(Vec::new()));
    let n = Arc::clone(&notices);
    let w = Worker::spawn(
        cfg,
        Vec::new(),
        path,
        Box::new(link),
        grabbers,
        autostart,
        Arc::new(move |x: Notice| n.lock().unwrap().push(x)),
        fast(),
    );
    (w, notices)
}
```

4. Append:

```rust
// ---------- Windows lifecycle ----------

#[test]
fn startup_syncs_autostart_with_the_setting() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);
    let mut cfg = saved_cfg();
    cfg.startup.autostart = false;
    let (w, _) = spawn_full(cfg, FakeLink::with_strip(), None, red_screen().0, Arc::new(move |on| {
        s.lock().unwrap().push(on);
        Ok(())
    }));
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    assert_eq!(*seen.lock().unwrap(), vec![false]);
    w.send(Command::SetAutostart { enabled: true });
    let deadline = Instant::now() + Duration::from_secs(3);
    while seen.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(*seen.lock().unwrap(), vec![false, true]);
}

#[test]
fn a_failing_autostart_is_reported_in_the_snapshot() {
    let (w, _) =
        spawn_full(saved_cfg(), FakeLink::with_strip(), None, red_screen().0, Arc::new(|_| Err("access denied".into())));
    wait_for(&w, "autostart error", |s| s.errors.iter().any(|e| e.contains("autostart")));
    wait_for(&w, "still connects", |s| s.connection == Connection::Ready);
}

#[test]
fn send_and_wait_returns_once_the_command_is_handled() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    assert!(w.handle().send_and_wait(Command::Darken { reason: Dark::Sleep }, Duration::from_secs(2)));
    assert!(link.calls().contains(&Call::Apply(IP, LightChange::off())), "{:?}", link.calls());
}

#[test]
fn send_and_wait_gives_up_on_a_busy_worker() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    std::thread::sleep(Duration::from_millis(250));
    link.0.lock().unwrap().apply_delay = Duration::from_millis(600);
    let h = w.handle();
    assert!(!h.send_and_wait(Command::SetPower { on: false }, Duration::from_millis(100)));
    assert!(h.send_and_wait(Command::SetPower { on: true }, Duration::from_secs(3)), "later commands are still awaited");
}

#[test]
fn locking_in_ambilight_stops_capture_and_unlocking_restarts_it() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(ambilight_cfg(), link.clone(), None, grabbers);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert!(w.handle().send_and_wait(Command::Darken { reason: Dark::Locked }, Duration::from_secs(2)));
    assert!(link.calls().contains(&Call::StopStream(IP, LightChange::off(), false)));
    assert_eq!(alive.load(Ordering::SeqCst), 0);
    w.send(Command::Undarken { reason: Dark::Locked });
    wait_calls(&link, "second start", |c| c.iter().filter(|x| matches!(x, Call::StartStream(..))).count() == 2);
}
```

Run: `cargo test --test worker_tests`
Expected: compile errors (`AutostartFn`, `send_and_wait`, `spawn` arity).

- [ ] **Step 2: Implement**

In `src/app/worker.rs`:

1. `use std::sync::{Arc, Condvar, Mutex, PoisonError};`
2. Types:

```rust
/// Syncs the Windows Run entry; `autostart::sync` in the app, a fake in tests.
pub type AutostartFn = Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;

/// Commands sent and handled, so a sender can wait for its command.
#[derive(Default)]
struct Progress {
    /// Commands sent; the lock also keeps numbering and queueing in one order.
    sent: Mutex<u64>,
    done: Mutex<u64>,
    cv: Condvar,
}

impl Progress {
    fn advance(&self, n: u64) {
        *self.done.lock().unwrap_or_else(PoisonError::into_inner) += n;
        self.cv.notify_all();
    }
}
```

3. `WorkerHandle` gains `progress: Arc<Progress>`; `send` numbers the command under the `sent` lock:

```rust
    /// Queues a command; false if the worker has stopped.
    pub fn send(&self, cmd: Command) -> bool {
        self.enqueue(cmd).is_some()
    }

    /// Queues a command and waits until the worker has handled it, at most
    /// `timeout`. False on timeout or when the worker has stopped.
    pub fn send_and_wait(&self, cmd: Command, timeout: Duration) -> bool {
        let Some(ticket) = self.enqueue(cmd) else { return false };
        let done = self.progress.done.lock().unwrap_or_else(PoisonError::into_inner);
        let (done, _) = self
            .progress
            .cv
            .wait_timeout_while(done, timeout, |d| *d < ticket)
            .unwrap_or_else(PoisonError::into_inner);
        *done >= ticket
    }

    fn enqueue(&self, cmd: Command) -> Option<u64> {
        let mut sent = self.progress.sent.lock().unwrap_or_else(PoisonError::into_inner);
        self.tx.send(cmd).ok()?;
        *sent += 1;
        Some(*sent)
    }
```

4. `Worker::spawn` takes `autostart: AutostartFn` after `grabbers`; creates `let progress = Arc::new(Progress::default());`, puts a clone into the handle and `Ctx { …, autostart, progress }`; `Ctx` gains `autostart: AutostartFn` and `progress: Arc<Progress>`.

5. In `run`, count the batch before coalescing and advance after handling it:

```rust
                let handled = batch.len() as u64;
                for cmd in coalesce(batch) {
                    process(ctrl, link, ctx, &mut state, Event::Command(cmd));
                    if state.exit {
                        break;
                    }
                }
                ctx.progress.advance(handled);
```

(the throttling loop above may push more commands into `batch`; take `handled` right before `coalesce`.)

6. Replace the temporary arm in `process`:

```rust
                Action::SyncAutostart { enabled } => queue.push_back(Event::AutostartSynced((ctx.autostart)(enabled))),
```

- [ ] **Step 3: The real autostart function in the tray**

In `src/ui/tray.rs` add `use crate::app::worker::AutostartFn;` and `use crate::platform::autostart;`, and before `Worker::spawn`:

```rust
    let autostart: AutostartFn = Arc::new(|enabled| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        autostart::sync(autostart::RUN_KEY, autostart::VALUE_NAME, enabled, &exe).map_err(|e| e.to_string())
    });
```

and pass `autostart` after `grabbers`.

- [ ] **Step 4: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/app/worker.rs src/ui/tray.rs tests/worker_tests.rs
git commit -m "Let the tray wait for worker commands and sync autostart"
```

---

### Task 4: Lifecycle messages → events

**Files:**
- Create: `src/ui/lifecycle.rs`, `tests/lifecycle_tests.rs`
- Modify: `src/ui/mod.rs`

**Interfaces:**
- Consumes: Task 2 (`Command::{Darken, Undarken, EndSession}`, `Dark`).
- Produces: `ui::lifecycle::{LifecycleEvent { Lock, Unlock, Suspend, Resume, DisplayOff, DisplayOn, EndSession }, event(msg: u32, wparam: usize, display_state: Option<u32>) -> Option<LifecycleEvent>, LifecycleEvent::commands(self) -> Vec<Command>, LifecycleEvent::wait(self) -> Option<Duration>, SLEEP_WAIT, END_SESSION_WAIT}`.

- [ ] **Step 1: Write the failing tests**

Create `tests/lifecycle_tests.rs`:

```rust
use std::time::Duration;

use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::state::Dark;
use obsid_ambilight::ui::lifecycle::{END_SESSION_WAIT, LifecycleEvent as E, SLEEP_WAIT, event};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WM_ENDSESSION,
    WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

#[test]
fn session_changes_map_to_lock_and_unlock() {
    assert_eq!(event(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK as usize, None), Some(E::Lock));
    assert_eq!(event(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK as usize, None), Some(E::Unlock));
    assert_eq!(event(WM_WTSSESSION_CHANGE, 1, None), None, "console connect is not a lock");
}

#[test]
fn power_broadcasts_map_to_sleep_and_display() {
    let pb = |w: u32, d| event(WM_POWERBROADCAST, w as usize, d);
    assert_eq!(pb(PBT_APMSUSPEND, None), Some(E::Suspend));
    assert_eq!(pb(PBT_APMRESUMEAUTOMATIC, None), Some(E::Resume));
    assert_eq!(pb(PBT_APMRESUMESUSPEND, None), Some(E::Resume));
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, Some(0)), Some(E::DisplayOff));
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, Some(1)), Some(E::DisplayOn));
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, Some(2)), Some(E::DisplayOn), "dimmed is still on");
    assert_eq!(pb(PBT_POWERSETTINGCHANGE, None), None, "another power setting");
}

#[test]
fn only_a_real_session_end_counts() {
    assert_eq!(event(WM_ENDSESSION, 1, None), Some(E::EndSession));
}

#[test]
fn a_cancelled_session_end_is_ignored() {
    assert_eq!(event(WM_ENDSESSION, 0, None), None);
}

#[test]
fn unrelated_messages_are_ignored() {
    assert_eq!(event(0x0200, 0, None), None);
}

#[test]
fn events_become_worker_commands() {
    let d = |reason| Command::Darken { reason };
    let u = |reason| Command::Undarken { reason };
    assert_eq!(E::Lock.commands(), vec![d(Dark::Locked)]);
    assert_eq!(E::Unlock.commands(), vec![u(Dark::Locked), u(Dark::Sleep)], "unlock also clears a lost resume");
    assert_eq!(E::Suspend.commands(), vec![d(Dark::Sleep)]);
    assert_eq!(E::Resume.commands(), vec![u(Dark::Sleep)]);
    assert_eq!(E::DisplayOff.commands(), vec![d(Dark::DisplayOff)]);
    assert_eq!(E::DisplayOn.commands(), vec![u(Dark::DisplayOff)]);
    assert_eq!(E::EndSession.commands(), vec![Command::EndSession]);
}

#[test]
fn only_sleep_and_session_end_are_awaited() {
    assert_eq!(E::Suspend.wait(), Some(SLEEP_WAIT));
    assert_eq!(E::EndSession.wait(), Some(END_SESSION_WAIT));
    assert_eq!(E::Lock.wait(), None);
    assert_eq!(SLEEP_WAIT, Duration::from_millis(1500));
    assert_eq!(END_SESSION_WAIT, Duration::from_secs(2));
}
```

Run: `cargo test --test lifecycle_tests`
Expected: compile error — `ui::lifecycle` does not exist.

- [ ] **Step 2: Implement**

Create `src/ui/lifecycle.rs`:

```rust
//! Windows session, power and display messages → lifecycle events → worker
//! commands. Pure: the tray reads the messages and sends the commands.

use std::time::Duration;

use windows_sys::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WM_ENDSESSION,
    WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

use crate::app::commands::Command;
use crate::app::state::Dark;

/// Windows gives about two seconds before sleeping; leave some margin.
pub const SLEEP_WAIT: Duration = Duration::from_millis(1500);
pub const END_SESSION_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    Lock,
    Unlock,
    Suspend,
    Resume,
    DisplayOff,
    DisplayOn,
    EndSession,
}

/// `display_state` is the DWORD of a `GUID_CONSOLE_DISPLAY_STATE` power
/// setting change (0 off, 1 on, 2 dimmed), read by the caller from lParam;
/// `None` for other messages and other power settings.
pub fn event(msg: u32, wparam: usize, display_state: Option<u32>) -> Option<LifecycleEvent> {
    let w = wparam as u32;
    match msg {
        WM_WTSSESSION_CHANGE if w == WTS_SESSION_LOCK => Some(LifecycleEvent::Lock),
        WM_WTSSESSION_CHANGE if w == WTS_SESSION_UNLOCK => Some(LifecycleEvent::Unlock),
        WM_POWERBROADCAST if w == PBT_APMSUSPEND => Some(LifecycleEvent::Suspend),
        WM_POWERBROADCAST if w == PBT_APMRESUMEAUTOMATIC || w == PBT_APMRESUMESUSPEND => Some(LifecycleEvent::Resume),
        WM_POWERBROADCAST if w == PBT_POWERSETTINGCHANGE => match display_state? {
            0 => Some(LifecycleEvent::DisplayOff),
            _ => Some(LifecycleEvent::DisplayOn),
        },
        WM_ENDSESSION if wparam != 0 => Some(LifecycleEvent::EndSession),
        _ => None,
    }
}

impl LifecycleEvent {
    /// Worker commands for this event. Unlocking also clears `Sleep`, in case
    /// the resume message was lost.
    pub fn commands(self) -> Vec<Command> {
        let darken = |reason| Command::Darken { reason };
        let undarken = |reason| Command::Undarken { reason };
        match self {
            Self::Lock => vec![darken(Dark::Locked)],
            Self::Unlock => vec![undarken(Dark::Locked), undarken(Dark::Sleep)],
            Self::Suspend => vec![darken(Dark::Sleep)],
            Self::Resume => vec![undarken(Dark::Sleep)],
            Self::DisplayOff => vec![darken(Dark::DisplayOff)],
            Self::DisplayOn => vec![undarken(Dark::DisplayOff)],
            Self::EndSession => vec![Command::EndSession],
        }
    }

    /// How long the tray waits for the worker before letting Windows go on.
    pub fn wait(self) -> Option<Duration> {
        match self {
            Self::Suspend => Some(SLEEP_WAIT),
            Self::EndSession => Some(END_SESSION_WAIT),
            _ => None,
        }
    }
}
```

Add `pub mod lifecycle;` to `src/ui/mod.rs`.

- [ ] **Step 3: Run the tests**

Run: `cargo test --test lifecycle_tests`
Expected: PASS (7 tests).

- [ ] **Step 4: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/ui/lifecycle.rs src/ui/mod.rs tests/lifecycle_tests.rs
git commit -m "Map Windows session and power messages to lifecycle events"
```

---

### Task 5: Tray — subscriptions, waits, dark icon and tooltip

**Files:**
- Modify: `src/ui/tray.rs`, `src/ui/icons.rs`, `src/i18n.rs`, `tests/icons_tests.rs`, `tests/tray_tests.rs`

**Interfaces:**
- Consumes: Tasks 2–4 (`Snapshot.dark`, `WorkerHandle::send_and_wait`, `lifecycle::{event, LifecycleEvent}`).
- Produces: `i18n::dark_reasons(&Strings, &[Dark]) -> String`; strings `dark_sleep`, `dark_locked`, `dark_display`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/icons_tests.rs` (add `Dark` to the `app::state` import):

```rust
#[test]
fn a_dark_strip_shows_the_off_icon() {
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false);
    s.connection = Connection::Ready;
    s.dark = vec![Dark::Locked];
    assert_eq!(kind_for(&s), IconKind::Off);
    s.config.app.mode = Mode::Ambilight;
    assert_eq!(kind_for(&s), IconKind::Off);
}
```

Append to `tests/tray_tests.rs` (add `Dark` to the `app::state` import and `use obsid_ambilight::i18n::{RU, dark_reasons};`):

```rust
#[test]
fn a_dark_tooltip_says_why() {
    let mut s = ready();
    s.dark = vec![Dark::Locked, Dark::DisplayOff];
    assert_eq!(tooltip(&s, &EN), "Obsid Ambilight — Off · screen locked, display off · 192.168.1.85");
}

#[test]
fn dark_reasons_are_translated() {
    assert_eq!(dark_reasons(&RU, &[Dark::Sleep]), "сон");
    assert_eq!(dark_reasons(&EN, &[]), "");
}
```

Run: `cargo test --test icons_tests --test tray_tests`
Expected: compile errors (`dark_reasons`).

- [ ] **Step 2: Strings**

In `src/i18n.rs`: add fields after `capture_unavailable`:

```rust
    pub dark_sleep: &'static str,
    pub dark_locked: &'static str,
    pub dark_display: &'static str,
```

RU: `"сон"`, `"экран заблокирован"`, `"экран выключен"`; EN: `"sleep"`, `"screen locked"`, `"display off"`. Add `use crate::app::state::Dark;` and:

```rust
/// Why the strip is dark, e.g. "screen locked, display off"; empty when it is not.
pub fn dark_reasons(s: &Strings, dark: &[Dark]) -> String {
    dark.iter()
        .map(|d| match d {
            Dark::Sleep => s.dark_sleep,
            Dark::Locked => s.dark_locked,
            Dark::DisplayOff => s.dark_display,
        })
        .collect::<Vec<_>>()
        .join(", ")
}
```

- [ ] **Step 3: Icon and tooltip**

In `src/ui/icons.rs` `kind_for`, first arm: `Connection::Ready if !s.dark.is_empty() => IconKind::Off,`.

In `src/ui/tray.rs` `tooltip`, inside `Connection::Ready`, first branch:

```rust
            if !snap.dark.is_empty() {
                format!("{} · {} · {ip}", s.mode_off, i18n::dark_reasons(s, &snap.dark))
            } else if !l.power {
```

- [ ] **Step 4: Subscriptions and messages**

In `src/ui/tray.rs`:

1. Imports:

```rust
use windows_sys::Win32::System::Power::{
    HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification, UnregisterPowerSettingNotification,
};
use windows_sys::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows_sys::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;

use crate::ui::lifecycle::{self, LifecycleEvent};
```

2. Fields in `TrayApp`: `power_notify: HPOWERNOTIFY,` and `/// `finish_exit` already ran. finished: bool,`; initialise `power_notify: subscribe(hwnd), finished: false` (call `subscribe` right after `create_window`, before `Worker::spawn`, and store the result in a local).

3. Helpers:

```rust
/// Session (lock) and display state notifications for the hidden window.
fn subscribe(hwnd: HWND) -> HPOWERNOTIFY {
    // SAFETY: plain registrations for our own window.
    unsafe {
        if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0 {
            warn!("lock notifications unavailable");
        }
        let h = RegisterPowerSettingNotification(hwnd, &GUID_CONSOLE_DISPLAY_STATE, DEVICE_NOTIFY_WINDOW_HANDLE);
        if h == 0 {
            warn!("display state notifications unavailable");
        }
        h
    }
}

/// The display state carried by a `PBT_POWERSETTINGCHANGE` broadcast, if it is one.
///
/// # Safety
/// `lp` must be the lParam of a `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE` message.
unsafe fn display_state(lp: LPARAM) -> Option<u32> {
    let s = unsafe { &*(lp as *const POWERBROADCAST_SETTING) };
    if s.PowerSetting != GUID_CONSOLE_DISPLAY_STATE || s.DataLength < 4 {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(s.Data.as_ptr().cast::<u32>()) })
}
```

(`RegisterPowerSettingNotification` takes a `HANDLE`; pass `hwnd` — both are raw pointers in windows-sys; add `as _` if the compiler asks.)

4. In `wndproc`, before the `taskbar_created` arm:

```rust
        WM_QUERYENDSESSION => 1,
        WM_WTSSESSION_CHANGE | WM_POWERBROADCAST | WM_ENDSESSION => {
            let display = if msg == WM_POWERBROADCAST && wp as u32 == PBT_POWERSETTINGCHANGE {
                // SAFETY: lParam of a PBT_POWERSETTINGCHANGE broadcast.
                unsafe { display_state(lp) }
            } else {
                None
            };
            if let Some(ev) = lifecycle::event(msg, wp, display) {
                app.on_lifecycle(ev);
            }
            if msg == WM_POWERBROADCAST { 1 } else { 0 }
        }
```

and in `WM_DESTROY`, before `PostQuitMessage`:

```rust
            unsafe {
                WTSUnRegisterSessionNotification(hwnd);
                if app.power_notify != 0 {
                    UnregisterPowerSettingNotification(app.power_notify);
                }
            }
```

5. `TrayApp::on_lifecycle`:

```rust
    fn on_lifecycle(&mut self, ev: LifecycleEvent) {
        info!(?ev, "windows lifecycle event");
        let Some(handle) = self.worker.as_ref().map(Worker::handle) else { return };
        for cmd in ev.commands() {
            match ev.wait() {
                Some(t) if !handle.send_and_wait(cmd, t) => warn!(?ev, "the strip did not answer in time"),
                Some(_) => {}
                None => {
                    handle.send(cmd);
                }
            }
        }
        if ev == LifecycleEvent::EndSession {
            self.exiting = true;
            self.finish_exit();
        }
    }
```

6. `finish_exit` runs once: first line `if std::mem::replace(&mut self.finished, true) { return; }`.

- [ ] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Try it**

`cargo run --release`, then Win+L and log back in: the strip goes off on lock and returns on unlock; the tray tooltip while locked is not observable, so check `%APPDATA%\ObsidAmbilight\logs\obsid-ambilight.log` for `windows lifecycle event ev=Lock` / `Unlock`.

- [ ] **Step 7: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/ui/tray.rs src/ui/icons.rs src/i18n.rs tests/icons_tests.rs tests/tray_tests.rs
git commit -m "Follow Windows lock, sleep, display and session end in the tray"
```

---

### Task 6: Settings — autostart checkbox and pause diagnostics

**Files:**
- Modify: `src/ui/settings/general.rs`, `src/ui/settings/diagnostics.rs`, `src/i18n.rs`, `tests/settings_tests.rs`

**Interfaces:**
- Consumes: Task 2 (`Command::SetAutostart`, `Snapshot.dark`), Task 5 (`i18n::dark_reasons`).
- Produces: strings `autostart_label`, `autostart_hint`, `pause_label`, `pause_none`; summary line `paused: …`.

- [ ] **Step 1: Write the failing test**

Append to `tests/settings_tests.rs`:

```rust
#[test]
fn summary_reports_dark_reasons_and_autostart() {
    use obsid_ambilight::app::state::Dark;
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(true);
    let text = summary(&s);
    assert!(text.contains("paused: none"), "{text}");
    assert!(text.contains("autostart: true"), "{text}");
    s.dark = vec![Dark::Locked];
    assert!(summary(&s).contains("paused: Locked"), "{}", summary(&s));
}
```

Run: `cargo test --test settings_tests summary_reports_dark`
Expected: FAIL (`paused:` missing).

- [ ] **Step 2: Strings**

Add to `Strings` (general block): `autostart_label`, `autostart_hint`; (diagnostics block): `pause_label`, `pause_none`.

| Field | RU | EN |
|---|---|---|
| `autostart_label` | `Запускать вместе с Windows` | `Start with Windows` |
| `autostart_hint` | `Можно также отключить в Диспетчере задач → Автозагрузка.` | `Can also be turned off in Task Manager → Startup apps.` |
| `pause_label` | `Пауза` | `Paused` |
| `pause_none` | `нет` | `no` |

- [ ] **Step 3: General tab**

In `src/ui/settings/general.rs`, after the «power on start» checkbox:

```rust
    ui.add_space(8.0);
    let mut autostart = cfg.startup.autostart;
    if ui.checkbox(&mut autostart, s.autostart_label).changed() {
        backend.send(Command::SetAutostart { enabled: autostart });
    }
    ui.weak(s.autostart_hint);
```

- [ ] **Step 4: Diagnostics**

In `src/ui/settings/diagnostics.rs` `summary`, after the `mode:` line:

```rust
    let paused = if snap.dark.is_empty() {
        "none".to_owned()
    } else {
        snap.dark.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>().join(", ")
    };
    let _ = writeln!(t, "paused: {paused}");
    let _ = writeln!(t, "autostart: {}", snap.config.startup.autostart);
```

and in `show`, after the mode row:

```rust
        ui.label(s.pause_label);
        ui.label(if snap.dark.is_empty() { s.pause_none.to_owned() } else { i18n::dark_reasons(s, &snap.dark) });
        ui.end_row();
```

- [ ] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/ui/settings src/i18n.rs tests/settings_tests.rs
git commit -m "Add the autostart checkbox and pause diagnostics"
```

---

### Task 7: Documentation, hardware checklist, verification

**Files:**
- Modify: `README.md`, `docs/testing.md`

- [ ] **Step 1: README**

Status table: replace `| Windows lifecycle (autostart, shutdown, sleep) | Stage 3 |` with `| Stage 3 — Windows lifecycle: autostart, lock, sleep, display off, shutdown | ✅ implemented, hardware checklist in docs/testing.md |`. In «Running the app» add: «The app starts with Windows (Settings → General → «Запускать вместе с Windows»). The strip goes dark while the screen is locked or off, while the PC sleeps, and at shutdown or logoff, and returns to its mode afterwards.»

- [ ] **Step 2: Checklist**

Append to `docs/testing.md`:

```markdown
## Stage 3 on hardware

Run `target\release\obsid-ambilight.exe`. After each row, check that Apple Home
still controls the strip.

| # | Scenario | Expected | Result |
|---|---|---|---|
| L1 | First start of the Stage 3 build | Task Manager → Startup apps lists Obsid Ambilight (enabled) | ⏳ |
| L2 | Win+L, wait 10 s, unlock — in Static and in Ambilight | Off while locked; mode returns after unlock | ⏳ |
| L3 | Display turns off by the power plan timer (set it to 1 min for the test), then move the mouse | Off with the display; mode returns with it | ⏳ |
| L4 | Sleep (Start → Power → Sleep), wake up | Off before sleeping; mode returns after waking (maybe after a network retry) | ⏳ |
| L5 | Restart Windows in Ambilight | Off during shutdown (no frozen frame); after logon the app starts by itself and Ambilight resumes | ⏳ |
| L6 | Sign out and back in | Off at sign-out; app starts and resumes at sign-in | ⏳ |
| L7 | Uncheck «Запускать вместе с Windows», then check it again | Entry disappears from / returns to Startup apps | ⏳ |
| L8 | Disable the app in Task Manager → Startup apps, restart Windows | App does not start; the checkbox still shows on (hint explains) | ⏳ |
| L9 | Lock while the settings window highlights a LED | Highlight ends, strip off; after unlock the mode returns | ⏳ |
```

- [ ] **Step 3: Full verification**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Expected: all green.

- [ ] **Step 4: Hardware run with the user**

Walk through L1–L9 with the user and record each result with the date.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/testing.md
git commit -m "Document Stage 3 and its hardware checklist"
```
