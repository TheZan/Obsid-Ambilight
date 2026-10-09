# Stage 1 — Static Controller Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A light Windows tray application that finds the Yeelight Obsid strip, keeps it on a persisted static colour/brightness, reconnects on its own, and offers an on-demand egui settings window, in Russian and English.

**Architecture:** One exe, two processes. The background process owns the tray (raw Win32), a single device worker thread driven by a pure state machine (`Controller`), the only writer of `config.toml`, and a named-pipe IPC server. `--settings` starts a separate eframe/egui process that talks to the background process over the pipe and exits when its window closes.

**Tech Stack:** Rust 2024 (1.85+), `windows-sys` 0.61, `serde` 1 + `toml` 1, `eframe`/`egui` 0.36 (glow), `serde_json`, `tracing` — plus the existing `protocol`, `discovery`, `reconnect` modules.

**Spec:** `docs/superpowers/specs/2026-10-09-stage1-static-controller-design.md`

## Global Constraints

- Platform: Windows 10/11 x64 only; no admin rights; no WebView; no TCP/HTTP ports.
- New dependencies allowed: `serde` (derive), `toml`, `eframe`. Nothing else without asking.
- Config file: `%APPDATA%\ObsidAmbilight\config.toml`; only the background process writes it; atomic write; session token never stored or logged.
- Session policy: UDP session opened on demand; no background keep-alive; no polling of the strip.
- Unicast M-SEARCH does not work on fw 36 — identity of a saved strip is checked only via multicast discovery by `id`.
- Retry pacing: 2 s doubling to 5 min with equal jitter, unlimited attempts.
- Light changes reach the strip at most ~5 per second (200 ms minimum interval); the last value is always sent.
- UDP reply timeout 0.6 s, at most 3 handshake attempts; exit power-off waits ≤ ~1 s.
- Languages: Russian and English; `app.language = auto|ru|en`; `auto` = Russian when the Windows UI language is Russian, otherwise English. Logs in English.
- Single instance: mutex `Local\ObsidAmbilight.Instance`; a second launch asks the first to open settings.
- Commits: one per task, subject line only, no body, no `Co-Authored-By`.
- Every task ends with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green.

## Review Focus

1. **The strip vanishes between commands** (unplugged, Wi-Fi drop): the next command must flip to Offline, keep the user's change pending, retry with backoff and re-apply it after reconnection — not lose it, not spin. Pinned in Task 3 (`apply_failure_keeps_change_and_reapplies_after_reconnect`).
2. **A bad value reaches the strip** (black from a preset or hand-edited config): `set_rgb` rejects black, which would push the worker into an endless Offline loop. Config, presets and `SetStatic` must refuse black. Pinned in Task 1 and Task 3.
3. **Rapid slider drags flood the strip**: hundreds of commands per second from the settings window must collapse to ≤ 5 applies/s with the final value applied. Pinned in Task 3 (`coalesce`) and Task 4 (`throttles_light_changes`).
4. **Corrupted or hand-edited config**: a broken file must not crash startup nor be silently lost; individual bad fields fall back. Pinned in Task 1.
5. **Second launch / hung first instance**: must open settings in the running instance, or show a message within ~2 s — never hang or start a duplicate. Pinned in Task 5 (mutex) and Task 6 (client timeout).

---

## File Structure

```text
Cargo.toml                        + serde, toml (Task 1), eframe (Task 9), windows-sys features
src/lib.rs                        + pub mod app, config, diagnostics, i18n, ipc, platform, ui
src/main.rs                       NEW  dispatcher: background | --settings [--tab X]
src/protocol/mod.rs               + serde Serialize/Deserialize for Rgb ("#RRGGBB")
src/config.rs                     NEW  Config types, lenient parse, load/backup, atomic save
src/i18n.rs                       NEW  Lang, Strings (RU/EN), resolve(), fill()
src/app/mod.rs                    NEW  module root
src/app/state.rs                  NEW  DeviceInfo, Connection, TestState, Tab, Snapshot
src/app/commands.rs               NEW  Command, coalesce()
src/app/controller.rs             NEW  pure state machine: Event -> Vec<Action>
src/app/worker.rs                 NEW  worker thread, DeviceLink trait, resolve(), supervision
src/app/link.rs                   NEW  UdpLink: DeviceLink over protocol/discovery
src/diagnostics.rs                NEW  log file + rotation, panic hook
src/platform/mod.rs               NEW  wide(), ui_language(), message_box(), open_folder()
src/platform/paths.rs             NEW  app_dir(), config_path(), logs_dir()
src/platform/instance.rs          NEW  InstanceGuard (named mutex)
src/ipc/mod.rs                    NEW  Request/Response, encode/decode
src/ipc/pipe.rs                   NEW  named pipe server/client, user-only DACL
src/ui/mod.rs                     NEW  module root
src/ui/icons.rs                   NEW  pure RGBA icon rendering
src/ui/tray.rs                    NEW  Win32 hidden window, tray icon, menu, settings child
src/ui/settings/mod.rs            NEW  eframe app, backend link thread, tabs frame
src/ui/settings/device.rs         NEW  Device tab
src/ui/settings/lighting.rs       NEW  Lighting tab
src/ui/settings/general.rs        NEW  General tab
src/ui/settings/diagnostics.rs    NEW  Diagnostics tab + summary()
tests/config_tests.rs             NEW
tests/i18n_tests.rs               NEW
tests/controller_tests.rs         NEW
tests/worker_tests.rs             NEW
tests/platform_tests.rs           NEW
tests/ipc_tests.rs                NEW
tests/icons_tests.rs              NEW
tests/settings_tests.rs           NEW
docs/testing.md, README.md        Stage 1 manual checklist, usage
```

Shared test helper used by several test files (copy into each file that needs it; integration test files cannot share modules without a `tests/common` dir, so create `tests/common/mod.rs` in Task 1 and `mod common;` where used):

```rust
// tests/common/mod.rs
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

static N: AtomicU32 = AtomicU32::new(0);

/// Fresh empty directory under %TEMP% for one test.
pub fn temp_dir(name: &str) -> PathBuf {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("obsid-test-{}-{name}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
```

---

### Task 1: Configuration (`config.rs`) and `Rgb` serde

**Files:**
- Modify: `Cargo.toml` (dependencies), `src/lib.rs`, `src/protocol/mod.rs`
- Create: `src/config.rs`, `tests/common/mod.rs`, `tests/config_tests.rs`

**Interfaces:**
- Consumes: `protocol::Rgb` (`from_hex`, `Display` as `#RRGGBB`, `BLACK`), `protocol::packets::MAX_LEDS`.
- Produces:
  - `Rgb: Serialize + Deserialize` (as `"#RRGGBB"`).
  - `config::{Config, DeviceConfig, LightConfig, StartupConfig, AppConfig, AdvancedConfig, Language, ExitAction, LogLevel, Loaded}`.
  - `config::parse(&str) -> Result<Loaded, String>`, `config::load(&Path) -> Loaded`, `config::save(&Path, &Config) -> io::Result<()>`, `config::sanitize_presets(Vec<Rgb>) -> (Vec<Rgb>, Vec<String>)`, `config::default_presets() -> Vec<Rgb>`, `config::MAX_PRESETS`, `LogLevel::as_tracing()`.
  - `Config: Serialize + Deserialize + Clone + PartialEq + Debug + Default`.

- [ ] **Step 1: Add dependencies**

Run:
```bash
cargo add serde@1.0.229 --features derive
cargo add toml@1.1.8
```

- [ ] **Step 2: Rgb serde**

Append to `src/protocol/mod.rs`:

```rust
impl serde::Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::from_hex(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid colour {s:?}")))
    }
}
```

- [ ] **Step 3: Write the failing tests**

`tests/common/mod.rs` — exactly the helper from the File Structure section.

`tests/config_tests.rs`:

```rust
mod common;

use std::fs;
use std::net::Ipv4Addr;

use obsid_ambilight::config::{self, Config, ExitAction, Language, LogLevel, MAX_PRESETS};
use obsid_ambilight::protocol::Rgb;

const SPEC_EXAMPLE: &str = r##"
version = 1

[device]
id = "dcda0cd778bc"
name = "Desk"
model = "Chameleon2"
last_ip = "192.168.1.85"
manual_ip = ""
led_count = 60

[static]
power = "on"
color = "#F5D4A1"
brightness = 40
presets = ["#F5D4A1", "#FFFFFF"]

[startup]
power_on = true

[app]
language = "ru"
exit_action = "off"

[advanced]
log_level = "debug"
"##;

#[test]
fn parses_the_spec_example() {
    let l = config::parse(SPEC_EXAMPLE).unwrap();
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    let c = l.config;
    assert_eq!(c.device.id, "dcda0cd778bc");
    assert_eq!(c.device.name, "Desk");
    assert_eq!(c.device.last_ip, Some(Ipv4Addr::new(192, 168, 1, 85)));
    assert_eq!(c.device.manual_ip, None);
    assert_eq!(c.device.led_count, 60);
    assert!(c.light.power);
    assert_eq!(c.light.color, Rgb::new(0xF5, 0xD4, 0xA1));
    assert_eq!(c.light.brightness, 40);
    assert_eq!(c.light.presets, vec![Rgb::new(0xF5, 0xD4, 0xA1), Rgb::new(255, 255, 255)]);
    assert!(c.startup.power_on);
    assert_eq!(c.app.language, Language::Ru);
    assert_eq!(c.app.exit_action, ExitAction::Off);
    assert_eq!(c.advanced.log_level, LogLevel::Debug);
}

#[test]
fn empty_text_gives_defaults_without_warnings() {
    let l = config::parse("").unwrap();
    assert!(l.warnings.is_empty());
    assert_eq!(l.config, Config::default());
    assert_eq!(l.config.light.presets, config::default_presets());
}

#[test]
fn unknown_keys_and_sections_are_ignored() {
    let l = config::parse("future = 1\n[ambilight]\nfps = 20\n[static]\nbrightness = 55\nshade = 3\n").unwrap();
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    assert_eq!(l.config.light.brightness, 55);
}

#[test]
fn each_bad_field_falls_back_alone() {
    let text = r##"
[device]
id = "abc"
last_ip = "300.1.1.1"
led_count = 500
[static]
color = "#XYZ"
brightness = 0
[app]
language = "de"
exit_action = "explode"
"##;
    let l = config::parse(text).unwrap();
    let d = Config::default();
    assert_eq!(l.config.device.id, "abc", "good fields survive");
    assert_eq!(l.config.device.last_ip, None);
    assert_eq!(l.config.device.led_count, d.device.led_count);
    assert_eq!(l.config.light.color, d.light.color);
    assert_eq!(l.config.light.brightness, d.light.brightness);
    assert_eq!(l.config.app.language, d.app.language);
    assert_eq!(l.config.app.exit_action, d.app.exit_action);
    assert_eq!(l.warnings.len(), 6, "{:?}", l.warnings);
    assert!(l.warnings.iter().any(|w| w.contains("static.brightness")));
}

#[test]
fn black_is_not_a_valid_static_colour() {
    let l = config::parse("[static]\ncolor = \"#000000\"\n").unwrap();
    assert_eq!(l.config.light.color, Config::default().light.color);
    assert_eq!(l.warnings.len(), 1);
}

#[test]
fn presets_are_cleaned() {
    let mut list: Vec<String> = (0..20).map(|i| format!("\"#0000{:02X}\"", i + 1)).collect();
    list.insert(0, "\"#0000FF\"".into());
    list.insert(1, "\"#0000ff\"".into()); // duplicate
    list.insert(2, "\"nope\"".into());
    list.insert(3, "\"#000000\"".into()); // black
    let l = config::parse(&format!("[static]\npresets = [{}]\n", list.join(","))).unwrap();
    let p = &l.config.light.presets;
    assert_eq!(p.len(), MAX_PRESETS);
    assert_eq!(p[0], Rgb::new(0, 0, 255));
    assert!(!p.contains(&Rgb::BLACK));
    assert_eq!(p.iter().filter(|c| **c == Rgb::new(0, 0, 255)).count(), 1);
    assert!(!l.warnings.is_empty());
}

#[test]
fn sanitize_presets_reports_what_it_dropped() {
    let (p, warnings) = config::sanitize_presets(vec![Rgb::new(1, 1, 1), Rgb::new(1, 1, 1), Rgb::BLACK]);
    assert_eq!(p, vec![Rgb::new(1, 1, 1)]);
    assert_eq!(warnings.len(), 1, "black is reported, duplicates silently removed");
}

#[test]
fn save_then_load_round_trips() {
    let dir = common::temp_dir("cfg-roundtrip");
    let path = dir.join("ObsidAmbilight").join("config.toml");
    let mut c = Config::default();
    c.device.id = "dcda0cd778bc".into();
    c.device.last_ip = Some(Ipv4Addr::new(10, 0, 0, 7));
    c.device.manual_ip = Some(Ipv4Addr::new(10, 0, 0, 8));
    c.light.power = false;
    c.app.language = Language::En;
    config::save(&path, &c).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("[static]"), "{text}");
    assert!(text.contains("power = \"off\""), "{text}");
    assert!(!dir.join("ObsidAmbilight").join("config.toml.tmp").exists());
    let l = config::load(&path);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    assert_eq!(l.config, c);
}

#[test]
fn empty_ip_is_written_as_empty_string() {
    let text = toml::to_string_pretty(&Config::default()).unwrap();
    assert!(text.contains("last_ip = \"\""), "{text}");
    assert!(text.contains("manual_ip = \"\""), "{text}");
}

#[test]
fn save_overwrites_existing_file() {
    let dir = common::temp_dir("cfg-overwrite");
    let path = dir.join("config.toml");
    fs::write(&path, "old").unwrap();
    config::save(&path, &Config::default()).unwrap();
    assert!(config::load(&path).warnings.is_empty());
}

#[test]
fn missing_file_gives_defaults_silently() {
    let dir = common::temp_dir("cfg-missing");
    let l = config::load(&dir.join("config.toml"));
    assert_eq!(l.config, Config::default());
    assert!(l.warnings.is_empty());
}

#[test]
fn broken_file_is_kept_aside_and_defaults_used() {
    let dir = common::temp_dir("cfg-broken");
    let path = dir.join("config.toml");
    fs::write(&path, "[device\nid = ").unwrap();
    let l = config::load(&path);
    assert_eq!(l.config, Config::default());
    assert_eq!(l.warnings.len(), 2, "{:?}", l.warnings);
    let kept: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("config.bad-") && n.ends_with(".toml"))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(fs::read_to_string(dir.join(&kept[0])).unwrap(), "[device\nid = ");
}

#[test]
fn non_utf8_file_is_treated_as_broken() {
    let dir = common::temp_dir("cfg-binary");
    let path = dir.join("config.toml");
    fs::write(&path, [0xFF, 0xFE, 0x00, 0x41]).unwrap();
    let l = config::load(&path);
    assert_eq!(l.config, Config::default());
    assert!(!l.warnings.is_empty());
}

#[test]
fn log_level_maps_to_tracing() {
    assert_eq!(LogLevel::Info.as_tracing(), tracing::Level::INFO);
    assert_eq!(LogLevel::Trace.as_tracing(), tracing::Level::TRACE);
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test --test config_tests`
Expected: compile error — `obsid_ambilight::config` not found.

- [ ] **Step 5: Implement `src/config.rs`**

```rust
//! Application settings in `%APPDATA%\ObsidAmbilight\config.toml`.
//!
//! Loading is lenient: an invalid field falls back to its default with a
//! warning, and a file that is not TOML at all is kept as
//! `config.bad-<unix-time>.toml` instead of being overwritten. Only the
//! background process writes the file, atomically.

use std::fs;
use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::protocol::Rgb;
use crate::protocol::packets::MAX_LEDS;

pub const CONFIG_VERSION: u32 = 1;
pub const MAX_PRESETS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    Auto,
    Ru,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExitAction {
    /// Leave the strip as it is.
    #[default]
    Keep,
    /// Switch the strip off.
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_tracing(self) -> tracing::Level {
        match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub device: DeviceConfig,
    #[serde(rename = "static")]
    pub light: LightConfig,
    pub startup: StartupConfig,
    pub app: AppConfig,
    pub advanced: AdvancedConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceConfig {
    /// Stable id from discovery; empty when no strip has been chosen.
    pub id: String,
    pub name: String,
    pub model: String,
    #[serde(with = "opt_ip")]
    pub last_ip: Option<Ipv4Addr>,
    /// When set, the app connects only to this address.
    #[serde(with = "opt_ip")]
    pub manual_ip: Option<Ipv4Addr>,
    pub led_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightConfig {
    /// Last power state chosen by the user.
    #[serde(with = "on_off")]
    pub power: bool,
    pub color: Rgb,
    pub brightness: u8,
    pub presets: Vec<Rgb>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartupConfig {
    /// Turn the strip on with the static colour when the app starts.
    pub power_on: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub language: Language,
    pub exit_action: ExitAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvancedConfig {
    pub log_level: LogLevel,
}

pub fn default_presets() -> Vec<Rgb> {
    ["#F5D4A1", "#FFFFFF", "#FFB46B", "#FF0000", "#00FF00", "#0000FF", "#8A2BE2", "#00CED1"]
        .iter()
        .filter_map(|h| Rgb::from_hex(h))
        .collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            device: DeviceConfig {
                id: String::new(),
                name: String::new(),
                model: crate::protocol::device::MODEL.to_owned(),
                last_ip: None,
                manual_ip: None,
                led_count: 60,
            },
            light: LightConfig {
                power: true,
                color: Rgb::new(0xF5, 0xD4, 0xA1),
                brightness: 40,
                presets: default_presets(),
            },
            startup: StartupConfig { power_on: true },
            app: AppConfig { language: Language::Auto, exit_action: ExitAction::Keep },
            advanced: AdvancedConfig { log_level: LogLevel::Info },
        }
    }
}

/// Result of loading: the usable config plus human-readable warnings.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    pub warnings: Vec<String>,
}

/// Parses config text. Errors only when the text is not TOML at all.
pub fn parse(text: &str) -> Result<Loaded, String> {
    let table: Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut r = Reader { table: &table, warnings: Vec::new() };
    let d = Config::default();
    let config = Config {
        version: CONFIG_VERSION,
        device: DeviceConfig {
            id: r.field("device", "id", d.device.id, as_string),
            name: r.field("device", "name", d.device.name, as_string),
            model: r.field("device", "model", d.device.model, as_string),
            last_ip: r.field("device", "last_ip", d.device.last_ip, as_opt_ip),
            manual_ip: r.field("device", "manual_ip", d.device.manual_ip, as_opt_ip),
            led_count: r.field("device", "led_count", d.device.led_count, |v| {
                v.as_integer().and_then(|n| usize::try_from(n).ok()).filter(|n| (1..=MAX_LEDS).contains(n))
            }),
        },
        light: LightConfig {
            power: r.field("static", "power", d.light.power, |v| match v.as_str()? {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            }),
            color: r.field("static", "color", d.light.color, |v| {
                v.as_str().and_then(Rgb::from_hex).filter(|c| *c != Rgb::BLACK)
            }),
            brightness: r.field("static", "brightness", d.light.brightness, |v| {
                v.as_integer().and_then(|n| u8::try_from(n).ok()).filter(|b| (1..=100).contains(b))
            }),
            presets: r.presets(d.light.presets),
        },
        startup: StartupConfig { power_on: r.field("startup", "power_on", d.startup.power_on, Value::as_bool) },
        app: AppConfig {
            language: r.field("app", "language", d.app.language, choice),
            exit_action: r.field("app", "exit_action", d.app.exit_action, choice),
        },
        advanced: AdvancedConfig { log_level: r.field("advanced", "log_level", d.advanced.log_level, choice) },
    };
    Ok(Loaded { config, warnings: r.warnings })
}

/// Loads the file. Never fails: problems become warnings and defaults.
pub fn load(path: &Path) -> Loaded {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Loaded { config: Config::default(), warnings: Vec::new() };
        }
        Err(e) => {
            return Loaded {
                config: Config::default(),
                warnings: vec![format!("cannot read {}: {e}; using defaults", path.display())],
            };
        }
    };
    let parsed = String::from_utf8(bytes).map_err(|e| e.to_string()).and_then(|t| parse(&t));
    match parsed {
        Ok(l) => l,
        Err(e) => {
            let mut warnings = vec![format!("config file is not valid TOML ({e}); using defaults")];
            match keep_broken(path) {
                Ok(p) => warnings.push(format!("the unreadable file was kept as {}", p.display())),
                Err(err) => warnings.push(format!("could not keep the unreadable file: {err}")),
            }
            Loaded { config: Config::default(), warnings }
        }
    }
}

/// Writes the config atomically: temp file, flush, then replace.
pub fn save(path: &Path, config: &Config) -> io::Result<()> {
    let text = toml::to_string_pretty(config).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

/// Removes duplicates and black, caps the list. Returns warnings for black
/// entries and for truncation (duplicates are dropped silently).
pub fn sanitize_presets(presets: Vec<Rgb>) -> (Vec<Rgb>, Vec<String>) {
    let mut out: Vec<Rgb> = Vec::new();
    let mut warnings = Vec::new();
    for c in presets {
        if c == Rgb::BLACK {
            warnings.push("static.presets: black cannot be a static colour, skipped".to_owned());
        } else if !out.contains(&c) {
            out.push(c);
        }
    }
    if out.len() > MAX_PRESETS {
        warnings.push(format!("static.presets: only the first {MAX_PRESETS} presets are kept"));
        out.truncate(MAX_PRESETS);
    }
    (out, warnings)
}

fn keep_broken(path: &Path) -> io::Result<PathBuf> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let dest = path.with_file_name(format!("config.bad-{secs}.toml"));
    fs::rename(path, &dest)?;
    Ok(dest)
}

struct Reader<'a> {
    table: &'a Table,
    warnings: Vec<String>,
}

impl Reader<'_> {
    fn get(&self, section: &str, key: &str) -> Option<&Value> {
        self.table.get(section).and_then(Value::as_table).and_then(|t| t.get(key))
    }

    fn field<T>(&mut self, section: &str, key: &str, default: T, parse: impl FnOnce(&Value) -> Option<T>) -> T {
        let Some(v) = self.get(section, key) else { return default };
        match parse(v) {
            Some(x) => x,
            None => {
                self.warnings.push(format!("{section}.{key}: invalid value {v}, using the default"));
                default
            }
        }
    }

    fn presets(&mut self, default: Vec<Rgb>) -> Vec<Rgb> {
        let Some(v) = self.get("static", "presets") else { return default };
        let Some(items) = v.as_array() else {
            self.warnings.push(format!("static.presets: invalid value {v}, using the default"));
            return default;
        };
        let mut colors = Vec::new();
        for item in items {
            match item.as_str().and_then(Rgb::from_hex) {
                Some(c) => colors.push(c),
                None => self.warnings.push(format!("static.presets: skipping invalid entry {item}")),
            }
        }
        let (clean, warnings) = sanitize_presets(colors);
        self.warnings.extend(warnings);
        clean
    }
}

fn as_string(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn as_opt_ip(v: &Value) -> Option<Option<Ipv4Addr>> {
    match v.as_str()? {
        "" => Some(None),
        s => s.parse().ok().map(Some),
    }
}

/// Enum values written as lowercase strings.
trait Choice: Copy + 'static {
    const NAMES: &'static [(&'static str, Self)];
}

impl Choice for Language {
    const NAMES: &'static [(&'static str, Self)] = &[("auto", Self::Auto), ("ru", Self::Ru), ("en", Self::En)];
}

impl Choice for ExitAction {
    const NAMES: &'static [(&'static str, Self)] = &[("keep", Self::Keep), ("off", Self::Off)];
}

impl Choice for LogLevel {
    const NAMES: &'static [(&'static str, Self)] = &[
        ("error", Self::Error),
        ("warn", Self::Warn),
        ("info", Self::Info),
        ("debug", Self::Debug),
        ("trace", Self::Trace),
    ];
}

fn choice<T: Choice>(v: &Value) -> Option<T> {
    let s = v.as_str()?;
    T::NAMES.iter().find(|(name, _)| *name == s).map(|(_, x)| *x)
}

mod opt_ip {
    use std::net::Ipv4Addr;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(ip: &Option<Ipv4Addr>, s: S) -> Result<S::Ok, S::Error> {
        match ip {
            Some(ip) => s.collect_str(ip),
            None => s.serialize_str(""),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Ipv4Addr>, D::Error> {
        let s = String::deserialize(d)?;
        if s.is_empty() { Ok(None) } else { s.parse().map(Some).map_err(serde::de::Error::custom) }
    }
}

mod on_off {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(on: &bool, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(if *on { "on" } else { "off" })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
        match String::deserialize(d)?.as_str() {
            "on" => Ok(true),
            "off" => Ok(false),
            other => Err(serde::de::Error::custom(format!("expected \"on\" or \"off\", got {other:?}"))),
        }
    }
}
```

Add `pub mod config;` to `src/lib.rs` (keep modules sorted: `config`, `discovery`, `protocol`, `reconnect`) and update its doc comment's last sentence to: "Stage 1 adds the tray application on top (see `docs/superpowers/specs`)."

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test config_tests`
Expected: 15 passed.

- [ ] **Step 7: Full checks**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all green.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/protocol/mod.rs src/config.rs tests/common/mod.rs tests/config_tests.rs
git commit -m "Add lenient TOML configuration with atomic save"
```

---

### Task 2: Localisation (`i18n.rs`)

**Files:**
- Modify: `src/lib.rs`
- Create: `src/i18n.rs`, `tests/i18n_tests.rs`

**Interfaces:**
- Consumes: `config::Language`.
- Produces: `i18n::{Lang, Strings, strings(Lang) -> &'static Strings, resolve(Language, u16) -> Lang, fill(&str, &[&str]) -> String}`; every field of `Strings` listed below is used by Tasks 8–9 under exactly these names.

- [ ] **Step 1: Write the failing tests**

`tests/i18n_tests.rs`:

```rust
use obsid_ambilight::config::Language;
use obsid_ambilight::i18n::{self, Lang};

#[test]
fn auto_follows_windows_ui_language() {
    assert_eq!(i18n::resolve(Language::Auto, 0x0419), Lang::Ru, "ru-RU");
    assert_eq!(i18n::resolve(Language::Auto, 0x0819), Lang::Ru, "ru-MD shares the primary id");
    assert_eq!(i18n::resolve(Language::Auto, 0x0409), Lang::En, "en-US");
    assert_eq!(i18n::resolve(Language::Auto, 0x0422), Lang::En, "uk-UA is not Russian");
    assert_eq!(i18n::resolve(Language::Auto, 0), Lang::En);
}

#[test]
fn explicit_choice_wins() {
    assert_eq!(i18n::resolve(Language::Ru, 0x0409), Lang::Ru);
    assert_eq!(i18n::resolve(Language::En, 0x0419), Lang::En);
}

#[test]
fn fill_replaces_placeholders_in_order() {
    assert_eq!(i18n::fill("{} of {}", &["1", "2"]), "1 of 2");
    assert_eq!(i18n::fill("no args", &["x"]), "no args");
    assert_eq!(i18n::fill("{} and {}", &["only"]), "only and {}");
}

#[test]
fn tables_differ_and_are_filled() {
    let ru = i18n::strings(Lang::Ru);
    let en = i18n::strings(Lang::En);
    assert_ne!(ru.tab_device, en.tab_device);
    assert!(ru.status_connected.contains("{}"));
    assert!(en.status_connected.contains("{}"));
    assert!(!ru.exit_item.is_empty() && !en.exit_item.is_empty());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test i18n_tests`
Expected: compile error — `obsid_ambilight::i18n` not found.

- [ ] **Step 3: Implement `src/i18n.rs`**

```rust
//! User-facing strings in Russian and English.
//!
//! Every string is a field of [`Strings`], so a missing translation is a
//! compile error. Templates use `{}` placeholders filled by [`fill`].

use crate::config::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}

/// Picks the UI language. `ui_langid` is a Windows LANGID; Russian is primary id 0x19.
pub fn resolve(setting: Language, ui_langid: u16) -> Lang {
    match setting {
        Language::Ru => Lang::Ru,
        Language::En => Lang::En,
        Language::Auto if ui_langid & 0x3FF == 0x19 => Lang::Ru,
        Language::Auto => Lang::En,
    }
}

pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::Ru => &RU,
        Lang::En => &EN,
    }
}

/// Replaces each `{}` with the next argument; extra placeholders stay as is.
pub fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut args = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        out.push_str(args.next().copied().unwrap_or("{}"));
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

pub struct Strings {
    // tray: status line and tooltip
    pub status_connected: &'static str,
    pub status_connecting: &'static str,
    pub status_offline: &'static str,
    pub status_no_device: &'static str,
    pub mode_static: &'static str,
    pub mode_off: &'static str,
    // tray: menu
    pub power_off_item: &'static str,
    pub power_on_item: &'static str,
    pub color_menu: &'static str,
    pub custom_color: &'static str,
    pub brightness_menu: &'static str,
    pub search_again: &'static str,
    pub settings_item: &'static str,
    pub exit_item: &'static str,
    pub already_running: &'static str,
    // settings: frame
    pub window_title: &'static str,
    pub tab_device: &'static str,
    pub tab_lighting: &'static str,
    pub tab_general: &'static str,
    pub tab_diagnostics: &'static str,
    pub backend_down: &'static str,
    pub start_backend: &'static str,
    // settings: device
    pub current_device: &'static str,
    pub no_device: &'static str,
    pub field_name: &'static str,
    pub field_id: &'static str,
    pub field_ip: &'static str,
    pub field_fw: &'static str,
    pub field_status: &'static str,
    pub unverified_id: &'static str,
    pub retry_in: &'static str,
    pub found_devices: &'static str,
    pub none_found: &'static str,
    pub searching: &'static str,
    pub search_btn: &'static str,
    pub select_btn: &'static str,
    pub selected: &'static str,
    pub manual_ip: &'static str,
    pub manual_ip_hint: &'static str,
    pub connect_btn: &'static str,
    pub clear_btn: &'static str,
    pub invalid_ip: &'static str,
    pub test_btn: &'static str,
    pub test_running: &'static str,
    pub test_ok: &'static str,
    pub test_failed: &'static str,
    pub led_count: &'static str,
    pub led_count_hint: &'static str,
    // settings: lighting
    pub lighting_power: &'static str,
    pub color_label: &'static str,
    pub hex_label: &'static str,
    pub black_not_allowed: &'static str,
    pub brightness_label: &'static str,
    pub presets_label: &'static str,
    pub add_preset: &'static str,
    pub preset_hint: &'static str,
    // settings: general
    pub language_label: &'static str,
    pub lang_auto: &'static str,
    pub power_on_start: &'static str,
    pub exit_action_label: &'static str,
    pub exit_keep: &'static str,
    pub exit_off: &'static str,
    // settings: diagnostics
    pub version_label: &'static str,
    pub connection_label: &'static str,
    pub session_label: &'static str,
    pub session_open: &'static str,
    pub session_closed: &'static str,
    pub errors_label: &'static str,
    pub no_errors: &'static str,
    pub config_warnings: &'static str,
    pub open_logs: &'static str,
    pub copy_summary: &'static str,
    pub copied: &'static str,
}

pub static RU: Strings = Strings {
    status_connected: "Подключено · {}",
    status_connecting: "Поиск ленты…",
    status_offline: "Устройство недоступно",
    status_no_device: "Лента не выбрана",
    mode_static: "Статичный",
    mode_off: "Выключено",
    power_off_item: "Выключить подсветку",
    power_on_item: "Включить подсветку",
    color_menu: "Цвет",
    custom_color: "Другой цвет…",
    brightness_menu: "Яркость",
    search_again: "Искать ленту снова",
    settings_item: "Настройки…",
    exit_item: "Выход",
    already_running: "Obsid Ambilight уже запущен, но не отвечает.",
    window_title: "Obsid Ambilight — настройки",
    tab_device: "Устройство",
    tab_lighting: "Подсветка",
    tab_general: "Общие",
    tab_diagnostics: "Диагностика",
    backend_down: "Фоновое приложение Obsid Ambilight не запущено.",
    start_backend: "Запустить",
    current_device: "Текущая лента",
    no_device: "Лента не выбрана",
    field_name: "Имя",
    field_id: "ID",
    field_ip: "IP",
    field_fw: "Прошивка",
    field_status: "Состояние",
    unverified_id: "подключено по IP, id не подтверждён",
    retry_in: "повтор через {} с",
    found_devices: "Найденные ленты",
    none_found: "Ленты не найдены",
    searching: "Идёт поиск…",
    search_btn: "Искать",
    select_btn: "Выбрать",
    selected: "выбрана",
    manual_ip: "Ручной IP",
    manual_ip_hint: "Если задан, приложение подключается только по этому адресу.",
    connect_btn: "Подключиться",
    clear_btn: "Сбросить",
    invalid_ip: "Некорректный IPv4-адрес",
    test_btn: "Проверить подключение",
    test_running: "Проверка…",
    test_ok: "Ответ за {} мс",
    test_failed: "Нет ответа: {}",
    led_count: "Число светодиодов",
    led_count_hint: "Понадобится для Ambilight (Этап 2).",
    lighting_power: "Подсветка включена",
    color_label: "Цвет",
    hex_label: "HEX",
    black_not_allowed: "Чёрный цвет нельзя задать — выключите подсветку.",
    brightness_label: "Яркость",
    presets_label: "Пресеты",
    add_preset: "Сохранить текущий цвет",
    preset_hint: "Клик — применить, правый клик — удалить.",
    language_label: "Язык",
    lang_auto: "Как в Windows",
    power_on_start: "Включать ленту при запуске приложения",
    exit_action_label: "При выходе из приложения",
    exit_keep: "Оставить текущий цвет",
    exit_off: "Выключить ленту",
    version_label: "Версия",
    connection_label: "Подключение",
    session_label: "UDP-сессия",
    session_open: "открыта",
    session_closed: "закрыта",
    errors_label: "Последние ошибки",
    no_errors: "Ошибок нет",
    config_warnings: "Предупреждения настроек",
    open_logs: "Открыть папку логов",
    copy_summary: "Скопировать сводку",
    copied: "Скопировано",
};

pub static EN: Strings = Strings {
    status_connected: "Connected · {}",
    status_connecting: "Looking for the strip…",
    status_offline: "Device unavailable",
    status_no_device: "No strip selected",
    mode_static: "Static",
    mode_off: "Off",
    power_off_item: "Turn lighting off",
    power_on_item: "Turn lighting on",
    color_menu: "Colour",
    custom_color: "Custom colour…",
    brightness_menu: "Brightness",
    search_again: "Search for the strip again",
    settings_item: "Settings…",
    exit_item: "Exit",
    already_running: "Obsid Ambilight is already running but is not responding.",
    window_title: "Obsid Ambilight — Settings",
    tab_device: "Device",
    tab_lighting: "Lighting",
    tab_general: "General",
    tab_diagnostics: "Diagnostics",
    backend_down: "The Obsid Ambilight background app is not running.",
    start_backend: "Start",
    current_device: "Current strip",
    no_device: "No strip selected",
    field_name: "Name",
    field_id: "ID",
    field_ip: "IP",
    field_fw: "Firmware",
    field_status: "Status",
    unverified_id: "connected by IP, id not verified",
    retry_in: "retrying in {} s",
    found_devices: "Found strips",
    none_found: "No strips found",
    searching: "Searching…",
    search_btn: "Search",
    select_btn: "Select",
    selected: "selected",
    manual_ip: "Manual IP",
    manual_ip_hint: "When set, the app connects to this address only.",
    connect_btn: "Connect",
    clear_btn: "Clear",
    invalid_ip: "Invalid IPv4 address",
    test_btn: "Test connection",
    test_running: "Testing…",
    test_ok: "Replied in {} ms",
    test_failed: "No reply: {}",
    led_count: "Number of LEDs",
    led_count_hint: "Used by Ambilight (stage 2).",
    lighting_power: "Lighting on",
    color_label: "Colour",
    hex_label: "HEX",
    black_not_allowed: "Black cannot be set — turn the lighting off instead.",
    brightness_label: "Brightness",
    presets_label: "Presets",
    add_preset: "Save current colour",
    preset_hint: "Click to apply, right-click to remove.",
    language_label: "Language",
    lang_auto: "Same as Windows",
    power_on_start: "Turn the strip on when the app starts",
    exit_action_label: "When the app exits",
    exit_keep: "Keep the current colour",
    exit_off: "Turn the strip off",
    version_label: "Version",
    connection_label: "Connection",
    session_label: "UDP session",
    session_open: "open",
    session_closed: "closed",
    errors_label: "Recent errors",
    no_errors: "No errors",
    config_warnings: "Settings warnings",
    open_logs: "Open log folder",
    copy_summary: "Copy summary",
    copied: "Copied",
};
```

Add `pub mod i18n;` to `src/lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test --test i18n_tests`
Expected: 4 passed.

- [ ] **Step 5: Full checks** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/i18n.rs tests/i18n_tests.rs
git commit -m "Add Russian and English UI strings"
```

---

### Task 3: App core — state, commands, controller

**Files:**
- Modify: `src/lib.rs`
- Create: `src/app/mod.rs`, `src/app/state.rs`, `src/app/commands.rs`, `src/app/controller.rs`, `tests/controller_tests.rs`

**Interfaces:**
- Consumes: `config::{Config, Language, ExitAction, sanitize_presets}`, `protocol::{Rgb, packets::MAX_LEDS}`, `reconnect::Backoff` (`with_seed`, `next_delay`, `reset`).
- Produces:
  - `app::state::{DeviceInfo { id, name, ip, fw_ver }, Connection { NoDevice, Connecting, Ready, Offline }, TestState { Idle, Running, Ok { millis }, Failed { message } }, Tab { Device, Lighting, General, Diagnostics }, Snapshot { version, connection, retry_in_secs, device, verified, session_open, searching, discovered, test, errors, config_warnings, config }}`; `Tab::{ALL, as_arg, from_arg, index, from_index}`; `DeviceInfo::display_name`.
  - `app::commands::{Command, coalesce(Vec<Command>) -> Vec<Command>}`; `Command::{is_light_change, allowed_over_ipc}`.
  - `app::controller::{Controller, Event, Action, Target, Resolved, LightChange, MAX_ERRORS, RETRY_BASE, RETRY_MAX}`; `Controller::{new(Config, Vec<String>, Backoff), handle(Event, Instant) -> Vec<Action>, snapshot(bool) -> Snapshot, config() -> &Config, next_deadline() -> Option<Instant>, record_error(String)}`; `LightChange::{full(&LightConfig), off()}`.

- [ ] **Step 1: Write the failing tests**

`tests/controller_tests.rs`:

```rust
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use obsid_ambilight::app::commands::{Command, coalesce};
use obsid_ambilight::app::controller::{
    Action, Controller, Event, LightChange, MAX_ERRORS, RETRY_BASE, Resolved, Target,
};
use obsid_ambilight::app::state::{Connection, DeviceInfo, TestState};
use obsid_ambilight::config::{Config, ExitAction};
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;

const IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 85);

fn dev(id: &str, ip: Ipv4Addr) -> DeviceInfo {
    DeviceInfo { id: id.into(), name: String::new(), ip, fw_ver: Some("36".into()) }
}

fn ctrl(cfg: Config) -> Controller {
    Controller::new(cfg, Vec::new(), Backoff::with_seed(RETRY_BASE, Duration::from_secs(300), None, 7))
}

fn saved_cfg() -> Config {
    let mut c = Config::default();
    c.device.id = "dcda0cd778bc".into();
    c.device.last_ip = Some(IP);
    c
}

fn full_on(c: &Config) -> LightChange {
    LightChange::full(&c.light)
}

/// Startup with a saved strip, resolved successfully. Returns (controller, now).
fn ready() -> (Controller, Instant) {
    let now = Instant::now();
    let mut c = ctrl(saved_cfg());
    c.handle(Event::Startup, now);
    c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), now);
    (c, now)
}

#[test]
fn first_run_with_one_strip_selects_it_and_lights_it() {
    let now = Instant::now();
    let mut c = ctrl(Config::default());
    let a = c.handle(Event::Startup, now);
    assert!(a.contains(&Action::Discover));
    assert!(c.snapshot(false).searching);
    assert_eq!(c.snapshot(false).connection, Connection::NoDevice);

    let a = c.handle(Event::Discovered(Ok(vec![dev("abc", IP)])), now);
    assert!(a.contains(&Action::SaveConfig));
    assert!(a.contains(&Action::Apply { ip: IP, change: full_on(c.config()) }), "{a:?}");
    assert_eq!(c.config().device.id, "abc");
    assert_eq!(c.config().device.last_ip, Some(IP));
    let s = c.snapshot(false);
    assert_eq!(s.connection, Connection::Ready);
    assert!(!s.searching);
    assert!(s.verified);
}

#[test]
fn first_run_with_several_or_no_strips_opens_settings() {
    for found in [vec![], vec![dev("a", IP), dev("b", Ipv4Addr::new(192, 168, 1, 86))]] {
        let now = Instant::now();
        let mut c = ctrl(Config::default());
        c.handle(Event::Startup, now);
        let a = c.handle(Event::Discovered(Ok(found.clone())), now);
        assert_eq!(a, vec![Action::OpenSettings], "found {}", found.len());
        assert_eq!(c.snapshot(false).connection, Connection::NoDevice);
        assert_eq!(c.snapshot(false).discovered, found);
    }
}

#[test]
fn first_run_discovery_error_opens_settings_and_records_it() {
    let now = Instant::now();
    let mut c = ctrl(Config::default());
    c.handle(Event::Startup, now);
    let a = c.handle(Event::Discovered(Err("socket error".into())), now);
    assert_eq!(a, vec![Action::OpenSettings]);
    assert_eq!(c.snapshot(false).errors, vec!["socket error".to_string()]);
}

#[test]
fn saved_strip_is_resolved_then_lit_when_power_on_start() {
    let now = Instant::now();
    let mut c = ctrl(saved_cfg());
    let a = c.handle(Event::Startup, now);
    assert!(a.contains(&Action::Resolve(Target::Saved { id: "dcda0cd778bc".into(), last_ip: Some(IP) })));
    assert_eq!(c.snapshot(false).connection, Connection::Connecting);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), now);
    assert!(a.contains(&Action::Apply { ip: IP, change: full_on(c.config()) }), "{a:?}");
}

#[test]
fn power_on_start_turns_power_on_even_if_user_left_it_off() {
    let mut cfg = saved_cfg();
    cfg.light.power = false;
    let mut c = ctrl(cfg);
    let a = c.handle(Event::Startup, Instant::now());
    assert!(c.config().light.power);
    assert!(a.contains(&Action::SaveConfig));
}

#[test]
fn without_power_on_start_the_strip_is_left_alone() {
    let mut cfg = saved_cfg();
    cfg.startup.power_on = false;
    let now = Instant::now();
    let mut c = ctrl(cfg);
    c.handle(Event::Startup, now);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), now);
    assert!(!a.iter().any(|x| matches!(x, Action::Apply { .. })), "{a:?}");
}

#[test]
fn resolve_failure_goes_offline_with_growing_jittered_retries() {
    let now = Instant::now();
    let mut c = ctrl(saved_cfg());
    c.handle(Event::Startup, now);
    c.handle(Event::Resolved(Err("not found".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    let first = c.next_deadline().unwrap() - now;
    assert!(first >= Duration::from_secs(1) && first <= Duration::from_secs(2), "{first:?}");

    assert!(c.handle(Event::Tick, now + first / 2).is_empty(), "no retry before the deadline");
    let a = c.handle(Event::Tick, now + first);
    assert!(matches!(a.as_slice(), [Action::Resolve(Target::Saved { .. })]), "{a:?}");
    assert_eq!(c.snapshot(false).connection, Connection::Connecting);

    let t2 = now + first;
    c.handle(Event::Resolved(Err("not found".into())), t2);
    let second = c.next_deadline().unwrap() - t2;
    assert!(second >= Duration::from_secs(2) && second <= Duration::from_secs(4), "{second:?}");
    assert!(c.snapshot(false).retry_in_secs.is_some());
}

#[test]
fn apply_failure_keeps_change_and_reapplies_after_reconnect() {
    let (mut c, now) = ready();
    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(10, 20, 30)), brightness: None }), now);
    assert!(a.iter().any(|x| matches!(x, Action::Apply { .. })));
    c.handle(Event::Applied(Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);

    // A change made while offline is kept too.
    let a = c.handle(Event::Command(Command::SetStatic { color: None, brightness: Some(77) }), now);
    assert!(!a.iter().any(|x| matches!(x, Action::Apply { .. })), "nothing is sent while offline: {a:?}");

    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    let expected = LightChange { power: Some(true), color: Some(Rgb::new(10, 20, 30)), brightness: Some(77) };
    assert!(a.contains(&Action::Apply { ip: IP, change: expected }), "{a:?}");
}

#[test]
fn power_off_while_offline_is_applied_on_reconnect() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: true }), now);
    c.handle(Event::Applied(Err("timeout".into())), now);
    c.handle(Event::Command(Command::SetPower { on: false }), now);
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at);
    assert!(a.contains(&Action::Apply { ip: IP, change: LightChange::off() }), "{a:?}");
}

#[test]
fn static_colour_implies_power_on_and_is_saved() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: false }), now);
    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: Some(50) }), now);
    assert!(c.config().light.power);
    assert_eq!(c.config().light.color, Rgb::new(1, 2, 3));
    assert_eq!(c.config().light.brightness, 50);
    assert!(a.contains(&Action::SaveConfig));
    assert!(a.contains(&Action::Apply { ip: IP, change: full_on(c.config()) }));
}

#[test]
fn invalid_values_are_rejected_without_saving() {
    let (mut c, now) = ready();
    for cmd in [
        Command::SetStatic { color: None, brightness: Some(0) },
        Command::SetStatic { color: None, brightness: Some(101) },
        Command::SetStatic { color: Some(Rgb::BLACK), brightness: None },
        Command::SetLedCount { count: 0 },
        Command::SetLedCount { count: 121 },
    ] {
        let before = c.config().clone();
        let a = c.handle(Event::Command(cmd.clone()), now);
        assert!(a.is_empty(), "{cmd:?} -> {a:?}");
        assert_eq!(c.config(), &before);
    }
    assert_eq!(c.snapshot(false).errors.len(), 5);
}

#[test]
fn settings_commands_update_config() {
    let (mut c, now) = ready();
    assert_eq!(c.handle(Event::Command(Command::SetLedCount { count: 90 }), now), vec![Action::SaveConfig]);
    assert_eq!(c.config().device.led_count, 90);
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    assert_eq!(c.config().app.exit_action, ExitAction::Off);
    c.handle(Event::Command(Command::SetPowerOnStart { enabled: false }), now);
    assert!(!c.config().startup.power_on);
    c.handle(Event::Command(Command::SetPresets { presets: vec![Rgb::new(9, 9, 9), Rgb::BLACK] }), now);
    assert_eq!(c.config().light.presets, vec![Rgb::new(9, 9, 9)]);
}

#[test]
fn manual_ip_takes_over_and_can_be_cleared() {
    let (mut c, now) = ready();
    let manual = Ipv4Addr::new(10, 0, 0, 9);
    let a = c.handle(Event::Command(Command::SetManualIp { ip: Some(manual) }), now);
    assert!(a.contains(&Action::SaveConfig));
    assert!(a.contains(&Action::Resolve(Target::Manual(manual))), "{a:?}");
    assert_eq!(c.snapshot(false).connection, Connection::Connecting);
    let a = c.handle(Event::Resolved(Ok(Resolved { device: dev("", manual), verified: false })), now);
    assert!(a.contains(&Action::Apply { ip: manual, change: full_on(c.config()) }), "{a:?}");
    assert_eq!(c.config().device.id, "dcda0cd778bc", "an empty resolved id keeps the saved one");
    assert!(!c.snapshot(false).verified);

    let a = c.handle(Event::Command(Command::SetManualIp { ip: None }), now);
    assert!(a.iter().any(|x| matches!(x, Action::Resolve(Target::Saved { .. }))), "{a:?}");
}

#[test]
fn selecting_a_discovered_strip_switches_to_it() {
    let (mut c, now) = ready();
    let other = Ipv4Addr::new(192, 168, 1, 99);
    c.handle(Event::Command(Command::Discover), now);
    c.handle(Event::Discovered(Ok(vec![dev("dcda0cd778bc", IP), dev("other", other)])), now);
    let a = c.handle(Event::Command(Command::SelectDevice { id: "other".into() }), now);
    assert_eq!(c.config().device.id, "other");
    assert!(a.contains(&Action::Apply { ip: other, change: full_on(c.config()) }), "{a:?}");
    let a = c.handle(Event::Command(Command::SelectDevice { id: "ghost".into() }), now);
    assert!(a.is_empty());
}

#[test]
fn discover_is_not_restarted_while_running() {
    let (mut c, now) = ready();
    assert_eq!(c.handle(Event::Command(Command::Discover), now), vec![Action::Discover]);
    assert!(c.handle(Event::Command(Command::Discover), now).is_empty());
}

#[test]
fn connection_test_reports_result() {
    let (mut c, now) = ready();
    assert_eq!(c.handle(Event::Command(Command::TestConnection), now), vec![Action::Test { ip: IP }]);
    assert_eq!(c.snapshot(false).test, TestState::Running);
    c.handle(Event::Tested(Ok(Duration::from_millis(31))), now);
    assert_eq!(c.snapshot(false).test, TestState::Ok { millis: 31 });
    c.handle(Event::Command(Command::TestConnection), now);
    c.handle(Event::Tested(Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).test, TestState::Failed { message: "timeout".into() });
}

#[test]
fn connection_test_without_strip_fails_immediately() {
    let mut c = ctrl(Config::default());
    assert!(c.handle(Event::Command(Command::TestConnection), Instant::now()).is_empty());
    assert!(matches!(c.snapshot(false).test, TestState::Failed { .. }));
}

#[test]
fn exit_powers_off_only_when_asked_and_connected() {
    let (mut c, now) = ready();
    assert_eq!(c.handle(Event::Command(Command::Exit), now), vec![Action::Shutdown { power_off_ip: None }]);

    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    assert_eq!(c.handle(Event::Command(Command::Exit), now), vec![Action::Shutdown { power_off_ip: Some(IP) }]);

    let mut c = ctrl(saved_cfg());
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    c.handle(Event::Startup, now);
    c.handle(Event::Resolved(Err("x".into())), now);
    assert_eq!(c.handle(Event::Command(Command::Exit), now), vec![Action::Shutdown { power_off_ip: None }]);
}

#[test]
fn error_list_is_capped() {
    let (mut c, _) = ready();
    for i in 0..30 {
        c.record_error(format!("e{i}"));
    }
    let errors = c.snapshot(false).errors;
    assert_eq!(errors.len(), MAX_ERRORS);
    assert_eq!(errors.last().unwrap(), "e29");
}

#[test]
fn tick_without_deadline_does_nothing() {
    let (mut c, now) = ready();
    assert!(c.next_deadline().is_none());
    assert!(c.handle(Event::Tick, now + Duration::from_secs(999)).is_empty());
}

#[test]
fn snapshot_carries_config_and_warnings() {
    let c = Controller::new(Config::default(), vec!["w1".into()], Backoff::with_seed(RETRY_BASE, RETRY_BASE, None, 1));
    let s = c.snapshot(true);
    assert_eq!(s.config_warnings, vec!["w1".to_string()]);
    assert!(s.session_open);
    assert_eq!(s.config, Config::default());
    assert_eq!(s.version, env!("CARGO_PKG_VERSION"));
}

// ---------- coalescing ----------

#[test]
fn coalesce_merges_consecutive_light_changes() {
    let out = coalesce(vec![
        Command::SetStatic { color: Some(Rgb::new(1, 1, 1)), brightness: Some(10) },
        Command::SetStatic { color: Some(Rgb::new(2, 2, 2)), brightness: None },
        Command::SetStatic { color: None, brightness: Some(30) },
    ]);
    assert_eq!(out, vec![Command::SetStatic { color: Some(Rgb::new(2, 2, 2)), brightness: Some(30) }]);
}

#[test]
fn coalesce_keeps_last_power_and_order_of_other_commands() {
    let out = coalesce(vec![
        Command::SetPower { on: false },
        Command::SetPower { on: true },
        Command::Discover,
        Command::SetStatic { color: None, brightness: Some(5) },
        Command::SetPower { on: false },
        Command::SetPresets { presets: vec![] },
        Command::SetPresets { presets: vec![Rgb::new(1, 1, 1)] },
    ]);
    assert_eq!(
        out,
        vec![
            Command::SetPower { on: true },
            Command::Discover,
            Command::SetStatic { color: None, brightness: Some(5) },
            Command::SetPower { on: false },
            Command::SetPresets { presets: vec![Rgb::new(1, 1, 1)] },
        ]
    );
}

#[test]
fn exit_is_not_allowed_over_ipc() {
    assert!(!Command::Exit.allowed_over_ipc());
    assert!(Command::Discover.allowed_over_ipc());
    assert!(Command::SetPower { on: true }.is_light_change());
    assert!(!Command::Discover.is_light_change());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test controller_tests`
Expected: compile error — `obsid_ambilight::app` not found.

- [ ] **Step 3: Implement `src/app/mod.rs`**

```rust
//! Application core: the device controller and its thread.

pub mod commands;
pub mod controller;
pub mod state;
```

(Task 4 adds `link` and `worker`.) Add `pub mod app;` to `src/lib.rs`.

- [ ] **Step 4: Implement `src/app/state.rs`**

```rust
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
```

- [ ] **Step 5: Implement `src/app/commands.rs`**

```rust
//! Commands accepted by the device worker.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::config::{ExitAction, Language};
use crate::protocol::Rgb;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Static colour and/or brightness; implies power on.
    SetStatic { color: Option<Rgb>, brightness: Option<u8> },
    SetPower { on: bool },
    SelectDevice { id: String },
    SetManualIp { ip: Option<Ipv4Addr> },
    Discover,
    TestConnection,
    SetLanguage { language: Language },
    SetPowerOnStart { enabled: bool },
    SetExitAction { action: ExitAction },
    SetLedCount { count: usize },
    SetPresets { presets: Vec<Rgb> },
    /// Tray "Exit" only; refused over IPC.
    Exit,
}

impl Command {
    pub fn is_light_change(&self) -> bool {
        matches!(self, Self::SetStatic { .. } | Self::SetPower { .. })
    }

    pub fn allowed_over_ipc(&self) -> bool {
        !matches!(self, Self::Exit)
    }
}

/// Collapses a burst: adjacent `SetStatic` merge field by field (later wins),
/// adjacent `SetPower` and `SetPresets` keep the last one; order is preserved.
pub fn coalesce(cmds: Vec<Command>) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::with_capacity(cmds.len());
    for cmd in cmds {
        match (out.last_mut(), cmd) {
            (
                Some(Command::SetStatic { color, brightness }),
                Command::SetStatic { color: new_color, brightness: new_brightness },
            ) => {
                if new_color.is_some() {
                    *color = new_color;
                }
                if new_brightness.is_some() {
                    *brightness = new_brightness;
                }
            }
            (Some(last @ Command::SetPower { .. }), cmd @ Command::SetPower { .. })
            | (Some(last @ Command::SetPresets { .. }), cmd @ Command::SetPresets { .. }) => *last = cmd,
            (_, cmd) => out.push(cmd),
        }
    }
    out
}
```

- [ ] **Step 6: Implement `src/app/controller.rs`**

```rust
//! Pure state machine of the device worker. It never touches the network or
//! the clock: the worker feeds it events with the current time and executes
//! the actions it returns, feeding results back as events.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::commands::Command;
use super::state::{Connection, DeviceInfo, Snapshot, TestState};
use crate::config::{self, Config, ExitAction, LightConfig};
use crate::protocol::Rgb;
use crate::protocol::packets::MAX_LEDS;
use crate::reconnect::Backoff;

pub const MAX_ERRORS: usize = 20;
pub const RETRY_BASE: Duration = Duration::from_secs(2);
pub const RETRY_MAX: Duration = Duration::from_secs(300);

/// Commands to send to the strip, in order: power, colour, brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightChange {
    pub power: Option<bool>,
    pub color: Option<Rgb>,
    pub brightness: Option<u8>,
}

impl LightChange {
    /// The full user state: on + colour + brightness, or just off.
    pub fn full(light: &LightConfig) -> Self {
        if light.power {
            Self { power: Some(true), color: Some(light.color), brightness: Some(light.brightness) }
        } else {
            Self::off()
        }
    }

    pub fn off() -> Self {
        Self { power: Some(false), color: None, brightness: None }
    }
}

/// How to find the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Multicast discovery by id, then the last known IP.
    Saved { id: String, last_ip: Option<Ipv4Addr> },
    /// User-entered address, used as is.
    Manual(Ipv4Addr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub device: DeviceInfo,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Startup,
    Command(Command),
    Resolved(Result<Resolved, String>),
    Discovered(Result<Vec<DeviceInfo>, String>),
    Applied(Result<(), String>),
    Tested(Result<Duration, String>),
    /// Time has passed; lets due retries fire.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Resolve(Target),
    Discover,
    Apply { ip: Ipv4Addr, change: LightChange },
    Test { ip: Ipv4Addr },
    SaveConfig,
    OpenSettings,
    Shutdown { power_off_ip: Option<Ipv4Addr> },
}

pub struct Controller {
    cfg: Config,
    connection: Connection,
    device: Option<DeviceInfo>,
    verified: bool,
    discovered: Vec<DeviceInfo>,
    searching: bool,
    /// First run: select the strip automatically if discovery finds exactly one.
    auto_select: bool,
    /// Change to send once the strip is reachable.
    pending: Option<LightChange>,
    retry_at: Option<Instant>,
    retry_in_secs: Option<u64>,
    backoff: Backoff,
    test: TestState,
    errors: VecDeque<String>,
    config_warnings: Vec<String>,
}

impl Controller {
    pub fn new(cfg: Config, config_warnings: Vec<String>, backoff: Backoff) -> Self {
        Self {
            cfg,
            connection: Connection::NoDevice,
            device: None,
            verified: false,
            discovered: Vec::new(),
            searching: false,
            auto_select: false,
            pending: None,
            retry_at: None,
            retry_in_secs: None,
            backoff,
            test: TestState::Idle,
            errors: VecDeque::new(),
            config_warnings,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// When the controller next needs a `Tick`.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.retry_at
    }

    pub fn record_error(&mut self, message: String) {
        warn!(%message, "error");
        if self.errors.len() == MAX_ERRORS {
            self.errors.pop_front();
        }
        self.errors.push_back(message);
    }

    pub fn snapshot(&self, session_open: bool) -> Snapshot {
        Snapshot {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            connection: self.connection,
            retry_in_secs: self.retry_in_secs,
            device: self.device.clone(),
            verified: self.verified,
            session_open,
            searching: self.searching,
            discovered: self.discovered.clone(),
            test: self.test.clone(),
            errors: self.errors.iter().cloned().collect(),
            config_warnings: self.config_warnings.clone(),
            config: self.cfg.clone(),
        }
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match event {
            Event::Startup => self.startup(),
            Event::Command(cmd) => self.command(cmd),
            Event::Tick => match self.retry_at {
                Some(at) if self.connection == Connection::Offline && now >= at => self.begin_resolve(),
                _ => Vec::new(),
            },
            Event::Resolved(Ok(r)) => self.on_resolved(r),
            Event::Resolved(Err(e)) => {
                self.record_error(e);
                self.go_offline(now);
                Vec::new()
            }
            Event::Discovered(result) => self.on_discovered(result),
            Event::Applied(Ok(())) => Vec::new(),
            Event::Applied(Err(e)) => {
                self.record_error(e);
                // Whatever was being sent, re-send the user's full state later.
                self.pending = Some(LightChange::full(&self.cfg.light));
                self.go_offline(now);
                Vec::new()
            }
            Event::Tested(result) => {
                self.test = match result {
                    Ok(d) => TestState::Ok { millis: d.as_millis() as u64 },
                    Err(message) => TestState::Failed { message },
                };
                Vec::new()
            }
        }
    }

    fn startup(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.cfg.startup.power_on {
            if !self.cfg.light.power {
                self.cfg.light.power = true;
                actions.push(Action::SaveConfig);
            }
            self.pending = Some(LightChange::full(&self.cfg.light));
        }
        if self.target().is_some() {
            actions.extend(self.begin_resolve());
        } else {
            self.auto_select = true;
            self.searching = true;
            actions.push(Action::Discover);
        }
        actions
    }

    fn target(&self) -> Option<Target> {
        if let Some(ip) = self.cfg.device.manual_ip {
            Some(Target::Manual(ip))
        } else if !self.cfg.device.id.is_empty() {
            Some(Target::Saved { id: self.cfg.device.id.clone(), last_ip: self.cfg.device.last_ip })
        } else {
            None
        }
    }

    fn begin_resolve(&mut self) -> Vec<Action> {
        self.retry_at = None;
        self.retry_in_secs = None;
        match self.target() {
            Some(t) => {
                self.connection = Connection::Connecting;
                vec![Action::Resolve(t)]
            }
            None => {
                self.connection = Connection::NoDevice;
                self.device = None;
                Vec::new()
            }
        }
    }

    fn go_offline(&mut self, now: Instant) {
        let delay = self.backoff.next_delay().unwrap_or(RETRY_MAX);
        self.connection = Connection::Offline;
        self.retry_at = Some(now + delay);
        self.retry_in_secs = Some(delay.as_secs().max(1));
        info!(?delay, "strip offline, will retry");
    }

    fn on_resolved(&mut self, r: Resolved) -> Vec<Action> {
        let mut actions = Vec::new();
        let d = &mut self.cfg.device;
        let before = d.clone();
        if !r.device.id.is_empty() {
            d.id = r.device.id.clone();
        }
        if !r.device.name.is_empty() {
            d.name = r.device.name.clone();
        }
        if d.manual_ip.is_none() {
            d.last_ip = Some(r.device.ip);
        }
        if *d != before {
            actions.push(Action::SaveConfig);
        }
        self.become_ready(r.device, r.verified);
        if let Some(change) = self.pending.take() {
            actions.push(self.apply_action(change));
        }
        actions
    }

    fn become_ready(&mut self, device: DeviceInfo, verified: bool) {
        self.connection = Connection::Ready;
        self.device = Some(device);
        self.verified = verified;
        self.retry_at = None;
        self.retry_in_secs = None;
        self.backoff.reset();
    }

    fn on_discovered(&mut self, result: Result<Vec<DeviceInfo>, String>) -> Vec<Action> {
        self.searching = false;
        match result {
            Ok(list) => self.discovered = list,
            Err(e) => self.record_error(e),
        }
        if !std::mem::take(&mut self.auto_select) {
            return Vec::new();
        }
        if self.discovered.len() == 1 {
            let dev = self.discovered[0].clone();
            self.select(dev)
        } else {
            vec![Action::OpenSettings]
        }
    }

    fn select(&mut self, dev: DeviceInfo) -> Vec<Action> {
        let d = &mut self.cfg.device;
        d.id = dev.id.clone();
        d.name = dev.name.clone();
        d.last_ip = Some(dev.ip);
        d.manual_ip = None;
        self.become_ready(dev, true);
        let mut actions = vec![Action::SaveConfig];
        let change = self.pending.take().or(self.cfg.light.power.then(|| LightChange::full(&self.cfg.light)));
        if let Some(change) = change {
            actions.push(self.apply_action(change));
        }
        actions
    }

    fn apply_action(&self, change: LightChange) -> Action {
        let ip = self.device.as_ref().map(|d| d.ip).expect("apply_action needs a device");
        Action::Apply { ip, change }
    }

    /// Sends now when ready, otherwise keeps it for the next connection.
    fn light(&mut self, change: LightChange) -> Vec<Action> {
        if self.connection == Connection::Ready && self.device.is_some() {
            vec![self.apply_action(change)]
        } else {
            self.pending = Some(change);
            Vec::new()
        }
    }

    fn command(&mut self, cmd: Command) -> Vec<Action> {
        match cmd {
            Command::SetStatic { color, brightness } => {
                if let Some(b) = brightness.filter(|b| !(1..=100).contains(b)) {
                    self.record_error(format!("brightness {b} is outside 1..=100"));
                    return Vec::new();
                }
                if color == Some(Rgb::BLACK) {
                    self.record_error("black cannot be a static colour".to_owned());
                    return Vec::new();
                }
                let l = &mut self.cfg.light;
                l.power = true;
                if let Some(c) = color {
                    l.color = c;
                }
                if let Some(b) = brightness {
                    l.brightness = b;
                }
                let mut actions = vec![Action::SaveConfig];
                actions.extend(self.light(LightChange::full(&self.cfg.light)));
                actions
            }
            Command::SetPower { on } => {
                self.cfg.light.power = on;
                let mut actions = vec![Action::SaveConfig];
                actions.extend(self.light(LightChange::full(&self.cfg.light)));
                actions
            }
            Command::SelectDevice { id } => match self.discovered.iter().find(|d| d.id == id).cloned() {
                Some(dev) => self.select(dev),
                None => {
                    self.record_error(format!("strip {id} is not in the discovery list"));
                    Vec::new()
                }
            },
            Command::SetManualIp { ip } => {
                self.cfg.device.manual_ip = ip;
                self.device = None;
                if self.cfg.light.power {
                    self.pending = Some(LightChange::full(&self.cfg.light));
                }
                let mut actions = vec![Action::SaveConfig];
                actions.extend(self.begin_resolve());
                actions
            }
            Command::Discover => {
                if self.searching {
                    return Vec::new();
                }
                self.searching = true;
                vec![Action::Discover]
            }
            Command::TestConnection => {
                match self.device.as_ref().map(|d| d.ip).or(self.cfg.device.manual_ip).or(self.cfg.device.last_ip) {
                    Some(ip) => {
                        self.test = TestState::Running;
                        vec![Action::Test { ip }]
                    }
                    None => {
                        self.test = TestState::Failed { message: "no strip selected".to_owned() };
                        Vec::new()
                    }
                }
            }
            Command::SetLanguage { language } => {
                self.cfg.app.language = language;
                vec![Action::SaveConfig]
            }
            Command::SetPowerOnStart { enabled } => {
                self.cfg.startup.power_on = enabled;
                vec![Action::SaveConfig]
            }
            Command::SetExitAction { action } => {
                self.cfg.app.exit_action = action;
                vec![Action::SaveConfig]
            }
            Command::SetLedCount { count } => {
                if !(1..=MAX_LEDS).contains(&count) {
                    self.record_error(format!("LED count {count} is outside 1..={MAX_LEDS}"));
                    return Vec::new();
                }
                self.cfg.device.led_count = count;
                vec![Action::SaveConfig]
            }
            Command::SetPresets { presets } => {
                let (clean, warnings) = config::sanitize_presets(presets);
                for w in warnings {
                    self.record_error(w);
                }
                self.cfg.light.presets = clean;
                vec![Action::SaveConfig]
            }
            Command::Exit => {
                let power_off_ip = match (self.cfg.app.exit_action, self.connection, &self.device) {
                    (ExitAction::Off, Connection::Ready, Some(d)) => Some(d.ip),
                    _ => None,
                };
                vec![Action::Shutdown { power_off_ip }]
            }
        }
    }
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test --test controller_tests`
Expected: 24 passed.

- [ ] **Step 8: Full checks** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 9: Commit**

```bash
git add src/lib.rs src/app tests/controller_tests.rs
git commit -m "Add device controller state machine"
```

---

### Task 4: Worker thread and UDP device link

**Files:**
- Modify: `src/app/mod.rs`
- Create: `src/app/worker.rs`, `src/app/link.rs`, `tests/worker_tests.rs`

**Interfaces:**
- Consumes: `Controller`, `Event`, `Action`, `Target`, `Resolved`, `LightChange` (Task 3); `coalesce`, `Command` (Task 3); `config::save` (Task 1); `protocol::device::Chameleon2`, `protocol::session::SessionConfig`, `discovery::{discover, DiscoveryOptions}`, `reconnect::Backoff`.
- Produces:
  - `app::worker::{DeviceLink, Notice, Worker, WorkerHandle, WorkerOptions, resolve}`.
  - `trait DeviceLink: Send { fn discover(&mut self) -> Result<Vec<DeviceInfo>, String>; fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String>; fn apply(&mut self, ip: Ipv4Addr, change: LightChange) -> Result<(), String>; fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String>; fn session_open(&self) -> bool; }`
  - `enum Notice { Status = 1, OpenSettings = 2, Stopped = 3 }` with `Notice::from_usize(usize) -> Option<Notice>`.
  - `Worker::spawn(Config, Vec<String>, Option<PathBuf>, Box<dyn DeviceLink>, Arc<dyn Fn(Notice) + Send + Sync>, WorkerOptions) -> Worker`; `Worker::{send(Command) -> bool, snapshot() -> Snapshot, handle() -> WorkerHandle, join(Duration) -> bool}`; `WorkerHandle: Clone + Send + Sync` with the same `send`/`snapshot`.
  - `app::link::UdpLink::new()`.

- [ ] **Step 1: Write the failing tests**

`tests/worker_tests.rs`:

```rust
mod common;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::controller::{LightChange, Target};
use obsid_ambilight::app::state::{Connection, DeviceInfo, Snapshot};
use obsid_ambilight::app::worker::{DeviceLink, Notice, Worker, WorkerOptions, resolve};
use obsid_ambilight::config::{self, Config, ExitAction};
use obsid_ambilight::protocol::Rgb;

const IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 85);

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Discover,
    Handshake(Ipv4Addr),
    Apply(Ipv4Addr, LightChange),
    ExitOff(Ipv4Addr),
}

#[derive(Default)]
struct Script {
    calls: Vec<Call>,
    devices: Vec<DeviceInfo>,
    fail_handshake: bool,
    panic_on_first_discover: bool,
}

#[derive(Clone, Default)]
struct FakeLink(Arc<Mutex<Script>>);

impl FakeLink {
    fn with_strip() -> Self {
        let l = Self::default();
        l.0.lock().unwrap().devices = vec![dev("dcda0cd778bc", IP)];
        l
    }
    fn calls(&self) -> Vec<Call> {
        self.0.lock().unwrap().calls.clone()
    }
}

impl DeviceLink for FakeLink {
    fn discover(&mut self) -> Result<Vec<DeviceInfo>, String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::Discover);
        if std::mem::take(&mut s.panic_on_first_discover) {
            drop(s);
            panic!("simulated worker crash");
        }
        Ok(s.devices.clone())
    }
    fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::Handshake(ip));
        if s.fail_handshake { Err("timeout".into()) } else { Ok(Duration::from_millis(12)) }
    }
    fn apply(&mut self, ip: Ipv4Addr, change: LightChange) -> Result<(), String> {
        self.0.lock().unwrap().calls.push(Call::Apply(ip, change));
        Ok(())
    }
    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String> {
        self.0.lock().unwrap().calls.push(Call::ExitOff(ip));
        Ok(())
    }
    fn session_open(&self) -> bool {
        false
    }
}

fn dev(id: &str, ip: Ipv4Addr) -> DeviceInfo {
    DeviceInfo { id: id.into(), name: String::new(), ip, fw_ver: None }
}

fn saved_cfg() -> Config {
    let mut c = Config::default();
    c.device.id = "dcda0cd778bc".into();
    c.device.last_ip = Some(IP);
    c
}

fn fast() -> WorkerOptions {
    WorkerOptions { save_delay: Duration::from_millis(50), min_apply_interval: Duration::from_millis(200) }
}

fn spawn(cfg: Config, link: FakeLink, path: Option<std::path::PathBuf>) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    let notices = Arc::new(Mutex::new(Vec::new()));
    let n = Arc::clone(&notices);
    let w = Worker::spawn(cfg, Vec::new(), path, Box::new(link), Arc::new(move |x: Notice| n.lock().unwrap().push(x)), fast());
    (w, notices)
}

fn wait_for(w: &Worker, what: &str, pred: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let s = w.snapshot();
        if pred(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {s:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn resolve_saved_strip_by_discovery() {
    let mut link = FakeLink::with_strip();
    let r = resolve(&mut link, &Target::Saved { id: "dcda0cd778bc".into(), last_ip: None }).unwrap();
    assert!(r.verified);
    assert_eq!(r.device.ip, IP);
}

#[test]
fn resolve_falls_back_to_last_ip_unverified() {
    let mut link = FakeLink::default();
    let r = resolve(&mut link, &Target::Saved { id: "dcda0cd778bc".into(), last_ip: Some(IP) }).unwrap();
    assert!(!r.verified);
    assert_eq!(r.device.id, "dcda0cd778bc");
    assert_eq!(link.calls(), vec![Call::Discover, Call::Handshake(IP)]);
}

#[test]
fn resolve_fails_when_nothing_answers() {
    let mut link = FakeLink::default();
    link.0.lock().unwrap().fail_handshake = true;
    assert!(resolve(&mut link, &Target::Saved { id: "x".into(), last_ip: Some(IP) }).is_err());
    assert!(resolve(&mut link, &Target::Saved { id: "x".into(), last_ip: None }).is_err());
    assert!(resolve(&mut link, &Target::Manual(IP)).is_err());
}

#[test]
fn startup_connects_and_applies_static() {
    let link = FakeLink::with_strip();
    let (w, notices) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !link.calls().iter().any(|c| matches!(c, Call::Apply(..))) {
        assert!(Instant::now() < deadline, "{:?}", link.calls());
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(notices.lock().unwrap().contains(&Notice::Status));
}

#[test]
fn throttles_light_changes_and_sends_the_last_value() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    std::thread::sleep(Duration::from_millis(250));
    let before = link.calls().len();
    let start = Instant::now();
    for b in 1..=100u8 {
        w.send(Command::SetStatic { color: None, brightness: Some(b) });
        std::thread::sleep(Duration::from_millis(5));
    }
    wait_for(&w, "last brightness", |s| s.config.light.brightness == 100);
    std::thread::sleep(Duration::from_millis(400));
    let applies: Vec<_> = link.calls()[before..].iter().filter(|c| matches!(c, Call::Apply(..))).cloned().collect();
    let elapsed = start.elapsed().as_secs_f64() + 0.4;
    assert!(applies.len() as f64 <= elapsed * 5.0 + 2.0, "{} applies in {elapsed:.1} s", applies.len());
    assert_eq!(applies.last().unwrap(), &Call::Apply(IP, LightChange { power: Some(true), color: Some(Config::default().light.color), brightness: Some(100) }));
}

#[test]
fn config_is_saved_after_a_change() {
    let dir = common::temp_dir("worker-save");
    let path = dir.join("config.toml");
    let (w, _) = spawn(saved_cfg(), FakeLink::with_strip(), Some(path.clone()));
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    w.send(Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: None });
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if config::load(&path).config.light.color == Rgb::new(1, 2, 3) {
            break;
        }
        assert!(Instant::now() < deadline, "config not saved");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn exit_with_power_off_turns_strip_off_and_stops() {
    let mut cfg = saved_cfg();
    cfg.app.exit_action = ExitAction::Off;
    let link = FakeLink::with_strip();
    let (w, notices) = spawn(cfg, link.clone(), None);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    assert!(w.send(Command::Exit));
    assert!(w.join(Duration::from_secs(3)), "worker did not stop");
    assert!(link.calls().contains(&Call::ExitOff(IP)));
    assert!(notices.lock().unwrap().contains(&Notice::Stopped));
}

#[test]
fn worker_restarts_after_a_panic() {
    let link = FakeLink::with_strip();
    link.0.lock().unwrap().panic_on_first_discover = true;
    let (w, _) = spawn(saved_cfg(), link.clone(), None);
    let s = wait_for(&w, "ready after restart", |s| s.connection == Connection::Ready);
    assert!(s.errors.iter().any(|e| e.contains("restarted")), "{:?}", s.errors);
    assert_eq!(link.calls().iter().filter(|c| **c == Call::Discover).count(), 2);
}

#[test]
fn notice_round_trips_through_usize() {
    for n in [Notice::Status, Notice::OpenSettings, Notice::Stopped] {
        assert_eq!(Notice::from_usize(n as usize), Some(n));
    }
    assert_eq!(Notice::from_usize(0), None);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test worker_tests`
Expected: compile error — `obsid_ambilight::app::worker` not found.

- [ ] **Step 3: Implement `src/app/worker.rs`**

```rust
//! The device worker thread: owns the link to the strip, runs the
//! [`Controller`], throttles light changes, saves the config and survives
//! panics (up to [`MAX_RESTARTS`] in a row).

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{error, info};

use super::commands::{Command, coalesce};
use super::controller::{Action, Controller, Event, LightChange, RETRY_BASE, RETRY_MAX, Resolved, Target};
use super::state::{DeviceInfo, Snapshot};
use crate::config::{self, Config};
use crate::reconnect::Backoff;

pub const MAX_RESTARTS: u32 = 3;

/// Access to the strip; [`UdpLink`](super::link::UdpLink) in the app, a fake in tests.
pub trait DeviceLink: Send {
    /// Chameleon2 strips that answered a LAN search.
    fn discover(&mut self) -> Result<Vec<DeviceInfo>, String>;
    /// Opens a fresh UDP session; returns how long it took.
    fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String>;
    fn apply(&mut self, ip: Ipv4Addr, change: LightChange) -> Result<(), String>;
    /// Best-effort power off with short timeouts (app exit).
    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String>;
    fn session_open(&self) -> bool;
}

/// What the worker tells the UI thread. Values are stable: they travel in WPARAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Status = 1,
    OpenSettings = 2,
    Stopped = 3,
}

impl Notice {
    pub fn from_usize(v: usize) -> Option<Self> {
        match v {
            1 => Some(Self::Status),
            2 => Some(Self::OpenSettings),
            3 => Some(Self::Stopped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerOptions {
    /// Delay before writing the config after a change (coalesces bursts).
    pub save_delay: Duration,
    /// Minimum time between two light applies (≈ 5 per second).
    pub min_apply_interval: Duration,
}

impl Default for WorkerOptions {
    fn default() -> Self {
        Self { save_delay: Duration::from_millis(500), min_apply_interval: Duration::from_millis(200) }
    }
}

/// Finds the strip for a target. Saved strips are matched by id via
/// discovery first; the last IP is only a fallback and is reported unverified.
pub fn resolve(link: &mut dyn DeviceLink, target: &Target) -> Result<Resolved, String> {
    match target {
        Target::Manual(ip) => {
            link.handshake(*ip).map_err(|e| format!("no reply from {ip}: {e}"))?;
            Ok(Resolved { device: DeviceInfo { id: String::new(), name: String::new(), ip: *ip, fw_ver: None }, verified: false })
        }
        Target::Saved { id, last_ip } => {
            let discovery_error = match link.discover() {
                Ok(list) => match list.into_iter().find(|d| d.id == *id) {
                    Some(device) => return Ok(Resolved { device, verified: true }),
                    None => None,
                },
                Err(e) => Some(e),
            };
            match last_ip {
                Some(ip) => {
                    link.handshake(*ip).map_err(|e| format!("strip {id} not found; last IP {ip}: {e}"))?;
                    Ok(Resolved {
                        device: DeviceInfo { id: id.clone(), name: String::new(), ip: *ip, fw_ver: None },
                        verified: false,
                    })
                }
                None => Err(match discovery_error {
                    Some(e) => format!("strip {id} not found: {e}"),
                    None => format!("strip {id} not found on the network"),
                }),
            }
        }
    }
}

type Notify = Arc<dyn Fn(Notice) + Send + Sync>;

/// Cloneable access for other threads (IPC server, tray).
#[derive(Clone)]
pub struct WorkerHandle {
    tx: Sender<Command>,
    shared: Arc<Mutex<Snapshot>>,
}

impl WorkerHandle {
    /// Queues a command; false if the worker has stopped.
    pub fn send(&self, cmd: Command) -> bool {
        self.tx.send(cmd).is_ok()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

pub struct Worker {
    handle: WorkerHandle,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(
        config: Config,
        warnings: Vec<String>,
        config_path: Option<PathBuf>,
        mut link: Box<dyn DeviceLink>,
        notify: Notify,
        options: WorkerOptions,
    ) -> Worker {
        let (tx, rx) = mpsc::channel();
        let first = new_controller(config, warnings);
        let shared = Arc::new(Mutex::new(first.snapshot(false)));
        let handle = WorkerHandle { tx, shared: Arc::clone(&shared) };
        let thread = thread::Builder::new()
            .name("device-worker".into())
            .spawn(move || {
                let ctx = Ctx { shared, notify, config_path, options };
                let mut ctrl = first;
                let mut restarts = 0;
                loop {
                    let result = catch_unwind(AssertUnwindSafe(|| run(&mut ctrl, link.as_mut(), &rx, &ctx)));
                    match result {
                        Ok(()) => return,
                        Err(_) if restarts < MAX_RESTARTS => {
                            restarts += 1;
                            error!(restarts, "device worker panicked, restarting");
                            let snap = ctx.snapshot();
                            ctrl = new_controller(snap.config, snap.config_warnings);
                            for e in snap.errors {
                                ctrl.record_error(e);
                            }
                            ctrl.record_error(format!("device worker restarted after an internal error ({restarts}/{MAX_RESTARTS})"));
                        }
                        Err(_) => {
                            error!("device worker failed repeatedly, giving up");
                            let mut snap = ctx.snapshot();
                            snap.errors.push("device worker stopped after repeated internal errors".into());
                            ctx.publish_snapshot(snap);
                            return;
                        }
                    }
                }
            })
            .expect("spawn device worker");
        Worker { handle, thread: Some(thread) }
    }

    pub fn send(&self, cmd: Command) -> bool {
        self.handle.send(cmd)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.handle.snapshot()
    }

    pub fn handle(&self) -> WorkerHandle {
        self.handle.clone()
    }

    /// Waits up to `timeout` for the thread to end; true if it ended.
    pub fn join(mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while let Some(t) = &self.thread {
            if t.is_finished() {
                let _ = self.thread.take().map(JoinHandle::join);
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        true
    }
}

fn new_controller(config: Config, warnings: Vec<String>) -> Controller {
    Controller::new(config, warnings, Backoff::new(RETRY_BASE, RETRY_MAX, None))
}

struct Ctx {
    shared: Arc<Mutex<Snapshot>>,
    notify: Notify,
    config_path: Option<PathBuf>,
    options: WorkerOptions,
}

impl Ctx {
    fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn publish_snapshot(&self, snap: Snapshot) {
        let changed = {
            let mut cur = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
            let changed = *cur != snap;
            *cur = snap;
            changed
        };
        if changed {
            (self.notify)(Notice::Status);
        }
    }
}

/// One worker life. Returns normally only on exit or when all senders are gone.
fn run(ctrl: &mut Controller, link: &mut dyn DeviceLink, rx: &Receiver<Command>, ctx: &Ctx) {
    let mut state = RunState { save_at: None, last_apply: None, exit: false };
    process(ctrl, link, ctx, &mut state, Event::Startup);

    while !state.exit {
        let now = Instant::now();
        let deadline = [ctrl.next_deadline(), state.save_at].into_iter().flatten().min();
        let wait = deadline.map_or(Duration::from_secs(3600), |d| d.saturating_duration_since(now));
        match rx.recv_timeout(wait) {
            Ok(first) => {
                let mut batch = vec![first];
                batch.extend(rx.try_iter());
                // Throttle: wait out the minimum interval, collecting newer commands.
                if batch.iter().any(Command::is_light_change) {
                    if let Some(last) = state.last_apply {
                        let ready_at = last + ctx.options.min_apply_interval;
                        while let Some(rest) = ready_at.checked_duration_since(Instant::now()) {
                            match rx.recv_timeout(rest) {
                                Ok(cmd) => batch.push(cmd),
                                Err(RecvTimeoutError::Timeout) => break,
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        batch.extend(rx.try_iter());
                    }
                }
                for cmd in coalesce(batch) {
                    process(ctrl, link, ctx, &mut state, Event::Command(cmd));
                    if state.exit {
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => process(ctrl, link, ctx, &mut state, Event::Tick),
            Err(RecvTimeoutError::Disconnected) => {
                save_now(ctrl, ctx, &mut state);
                return;
            }
        }
        if state.save_at.is_some_and(|t| Instant::now() >= t) {
            save_now(ctrl, ctx, &mut state);
        }
    }
}

struct RunState {
    save_at: Option<Instant>,
    last_apply: Option<Instant>,
    exit: bool,
}

/// Feeds one event and executes the resulting actions, following up with the
/// events they produce until nothing is left.
fn process(ctrl: &mut Controller, link: &mut dyn DeviceLink, ctx: &Ctx, st: &mut RunState, event: Event) {
    let mut queue = VecDeque::from([event]);
    while let Some(ev) = queue.pop_front() {
        let actions = ctrl.handle(ev, Instant::now());
        ctx.publish_snapshot(ctrl.snapshot(link.session_open()));
        for action in actions {
            match action {
                Action::Resolve(target) => queue.push_back(Event::Resolved(resolve(link, &target))),
                Action::Discover => queue.push_back(Event::Discovered(link.discover())),
                Action::Apply { ip, change } => {
                    st.last_apply = Some(Instant::now());
                    queue.push_back(Event::Applied(link.apply(ip, change)));
                }
                Action::Test { ip } => queue.push_back(Event::Tested(link.handshake(ip))),
                Action::SaveConfig => {
                    st.save_at.get_or_insert(Instant::now() + ctx.options.save_delay);
                }
                Action::OpenSettings => (ctx.notify)(Notice::OpenSettings),
                Action::Shutdown { power_off_ip } => {
                    if let Some(ip) = power_off_ip {
                        if let Err(e) = link.power_off_for_exit(ip) {
                            info!(%e, "power off on exit failed");
                        }
                    }
                    save_now(ctrl, ctx, st);
                    st.exit = true;
                    (ctx.notify)(Notice::Stopped);
                    return;
                }
            }
        }
        ctx.publish_snapshot(ctrl.snapshot(link.session_open()));
    }
}

fn save_now(ctrl: &mut Controller, ctx: &Ctx, st: &mut RunState) {
    if st.save_at.take().is_none() {
        return;
    }
    if let Some(path) = &ctx.config_path {
        if let Err(e) = config::save(path, ctrl.config()) {
            ctrl.record_error(format!("could not save settings: {e}"));
        }
    }
}
```

Note: `save_now` only writes when a save is pending; a forced exit with no pending change does not rewrite the file.

- [ ] **Step 4: Implement `src/app/link.rs`**

```rust
//! [`DeviceLink`] over the real UDP protocol. Sessions are opened on demand
//! and reused only for [`SESSION_IDLE`]; there is no background keep-alive.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use super::controller::LightChange;
use super::state::DeviceInfo;
use super::worker::DeviceLink;
use crate::discovery::{self, DiscoveryOptions};
use crate::protocol::device::Chameleon2;
use crate::protocol::packets::MAX_LEDS;
use crate::protocol::session::SessionConfig;
use crate::protocol::transport::UdpTransport;
use crate::protocol::{self, ProtocolError};

/// A session unused for this long is assumed expired on the device
/// (it announces a 10 s keep-alive interval) and is replaced.
pub const SESSION_IDLE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct UdpLink {
    current: Option<(Ipv4Addr, Chameleon2<UdpTransport>, Instant)>,
}

impl UdpLink {
    pub fn new() -> Self {
        Self::default()
    }

    fn connect(ip: Ipv4Addr, cfg: SessionConfig) -> protocol::Result<Chameleon2<UdpTransport>> {
        // LED count is irrelevant for static commands.
        Chameleon2::connect(ip, MAX_LEDS, cfg)
    }

    fn device(&mut self, ip: Ipv4Addr) -> protocol::Result<&mut Chameleon2<UdpTransport>> {
        let reusable = matches!(&self.current, Some((cur, dev, used)) if *cur == ip && dev.session().is_open() && used.elapsed() < SESSION_IDLE);
        if !reusable {
            self.current = None;
            let dev = Self::connect(ip, SessionConfig::default())?;
            self.current = Some((ip, dev, Instant::now()));
        }
        let (_, dev, used) = self.current.as_mut().expect("session just ensured");
        *used = Instant::now();
        Ok(dev)
    }
}

fn send(dev: &mut Chameleon2<UdpTransport>, change: LightChange) -> protocol::Result<()> {
    match change.power {
        Some(false) => return dev.set_power(false),
        Some(true) => dev.set_power(true)?,
        None => {}
    }
    if let Some(c) = change.color {
        dev.set_color(c)?;
    }
    if let Some(b) = change.brightness {
        dev.set_brightness(b)?;
    }
    Ok(())
}

fn text(e: ProtocolError) -> String {
    e.to_string()
}

impl DeviceLink for UdpLink {
    fn discover(&mut self) -> Result<Vec<DeviceInfo>, String> {
        let found = discovery::discover(&DiscoveryOptions::default()).map_err(|e| e.to_string())?;
        Ok(found.iter().filter(|d| d.is_chameleon2() && d.supports_rgbic_udp()).map(DeviceInfo::from).collect())
    }

    fn handshake(&mut self, ip: Ipv4Addr) -> Result<Duration, String> {
        self.current = None;
        let start = Instant::now();
        let dev = Self::connect(ip, SessionConfig::default()).map_err(text)?;
        let took = start.elapsed();
        self.current = Some((ip, dev, Instant::now()));
        Ok(took)
    }

    fn apply(&mut self, ip: Ipv4Addr, change: LightChange) -> Result<(), String> {
        let first = self.device(ip).and_then(|dev| send(dev, change));
        if first.is_ok() {
            return Ok(());
        }
        // One retry on a fresh session before reporting the strip unreachable.
        self.current = None;
        self.device(ip).and_then(|dev| send(dev, change)).map_err(text)
    }

    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String> {
        let quick = SessionConfig {
            reply_timeout: Duration::from_millis(400),
            handshake_attempts: 1,
            ..SessionConfig::default()
        };
        let reusable = matches!(&self.current, Some((cur, dev, used)) if *cur == ip && dev.session().is_open() && used.elapsed() < SESSION_IDLE);
        if !reusable {
            self.current = Some((ip, Self::connect(ip, quick).map_err(text)?, Instant::now()));
        }
        let (_, dev, _) = self.current.as_mut().expect("session just ensured");
        dev.set_power(false).map_err(text)
    }

    fn session_open(&self) -> bool {
        matches!(&self.current, Some((_, dev, used)) if dev.session().is_open() && used.elapsed() < SESSION_IDLE)
    }
}
```

Update `src/app/mod.rs`:

```rust
//! Application core: the device controller, its worker thread and the UDP link.

pub mod commands;
pub mod controller;
pub mod link;
pub mod state;
pub mod worker;
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test worker_tests`
Expected: 9 passed.

- [ ] **Step 6: Full checks** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 7: Commit**

```bash
git add src/app tests/worker_tests.rs
git commit -m "Add device worker thread with throttling, saving and UDP link"
```

---

### Task 5: Platform helpers and diagnostics

**Files:**
- Modify: `Cargo.toml` (windows-sys features), `src/lib.rs`
- Create: `src/platform/mod.rs`, `src/platform/paths.rs`, `src/platform/instance.rs`, `src/diagnostics.rs`, `tests/platform_tests.rs`

**Interfaces:**
- Consumes: `tracing_subscriber`, `windows-sys`.
- Produces:
  - `platform::{wide(&str) -> Vec<u16>, ui_language() -> u16, message_box(&str, &str), open_folder(&Path)}`.
  - `platform::paths::{app_dir() -> io::Result<PathBuf>, config_path() -> io::Result<PathBuf>, logs_dir() -> io::Result<PathBuf>}`.
  - `platform::instance::{InstanceGuard, INSTANCE_MUTEX}`; `InstanceGuard::acquire(&str) -> io::Result<Option<InstanceGuard>>`.
  - `diagnostics::{MAX_LOG_BYTES, rotate(&Path, u64) -> io::Result<()>, init_logging(&Path, tracing::Level) -> io::Result<()>}`.

- [ ] **Step 1: Extend windows-sys features**

Edit the `[target."cfg(windows)".dependencies]` line in `Cargo.toml` to:

```toml
windows-sys = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Globalization",
    "Win32_Graphics_Gdi",
    "Win32_NetworkManagement_IpHelper",
    "Win32_NetworkManagement_Ndis",
    "Win32_Networking_WinSock",
    "Win32_Security",
    "Win32_Security_Authorization",
    "Win32_Storage_FileSystem",
    "Win32_System_IO",
    "Win32_System_LibraryLoader",
    "Win32_System_Pipes",
    "Win32_System_RemoteDesktop",
    "Win32_System_Threading",
    "Win32_UI_HiDpi",
    "Win32_UI_Input_KeyboardAndMouse",
    "Win32_UI_Shell",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 2: Write the failing tests**

`tests/platform_tests.rs`:

```rust
mod common;

use std::fs;

use obsid_ambilight::diagnostics;
use obsid_ambilight::platform::{self, instance::InstanceGuard, paths};

#[test]
fn mutex_allows_one_holder() {
    let name = format!("Local\\ObsidAmbilight.Test.{}", std::process::id());
    let first = InstanceGuard::acquire(&name).unwrap();
    assert!(first.is_some());
    assert!(InstanceGuard::acquire(&name).unwrap().is_none(), "second holder refused");
    drop(first);
    assert!(InstanceGuard::acquire(&name).unwrap().is_some(), "free again after drop");
}

#[test]
fn paths_live_under_appdata() {
    let dir = paths::app_dir().unwrap();
    assert!(dir.ends_with("ObsidAmbilight"));
    assert_eq!(paths::config_path().unwrap(), dir.join("config.toml"));
    assert_eq!(paths::logs_dir().unwrap(), dir.join("logs"));
}

#[test]
fn wide_strings_are_nul_terminated() {
    assert_eq!(platform::wide("ab"), vec![b'a' as u16, b'b' as u16, 0]);
}

#[test]
fn rotate_keeps_small_logs() {
    let dir = common::temp_dir("rotate-small");
    let log = dir.join("app.log");
    fs::write(&log, "short").unwrap();
    diagnostics::rotate(&log, 100).unwrap();
    assert!(log.exists());
    assert!(!dir.join("app.log.1").exists());
}

#[test]
fn rotate_moves_big_log_and_replaces_old_copy() {
    let dir = common::temp_dir("rotate-big");
    let log = dir.join("app.log");
    fs::write(dir.join("app.log.1"), "older").unwrap();
    fs::write(&log, vec![b'x'; 200]).unwrap();
    diagnostics::rotate(&log, 100).unwrap();
    assert!(!log.exists());
    assert_eq!(fs::read(dir.join("app.log.1")).unwrap().len(), 200);
}

#[test]
fn rotate_ignores_missing_file() {
    let dir = common::temp_dir("rotate-missing");
    diagnostics::rotate(&dir.join("none.log"), 100).unwrap();
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --test platform_tests`
Expected: compile error — `obsid_ambilight::platform` not found.

- [ ] **Step 4: Implement `src/platform/mod.rs`**

```rust
//! Thin Windows helpers shared by the tray and the settings window.

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
```

- [ ] **Step 5: Implement `src/platform/paths.rs`**

```rust
//! Per-user locations under `%APPDATA%\ObsidAmbilight`.

use std::io;
use std::path::PathBuf;

pub fn app_dir() -> io::Result<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("ObsidAmbilight"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set"))
}

pub fn config_path() -> io::Result<PathBuf> {
    Ok(app_dir()?.join("config.toml"))
}

pub fn logs_dir() -> io::Result<PathBuf> {
    Ok(app_dir()?.join("logs"))
}
```

- [ ] **Step 6: Implement `src/platform/instance.rs`**

```rust
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
```

- [ ] **Step 7: Implement `src/diagnostics.rs`**

```rust
//! Log file setup: one file per process under `logs\`, rotated at start-up
//! when it exceeds [`MAX_LOG_BYTES`], plus a panic hook that logs panics.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Mutex;

pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Moves `path` to `<path>.1` (replacing an older copy) when it is larger than `max`.
pub fn rotate(path: &Path, max: u64) -> io::Result<()> {
    match fs::metadata(path) {
        Ok(m) if m.len() > max => {
            let mut old = path.as_os_str().to_owned();
            old.push(".1");
            let _ = fs::remove_file(&old);
            fs::rename(path, old)
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Sends `tracing` output to `path` (appending) and installs the panic hook.
pub fn init_logging(path: &Path, level: tracing::Level) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    rotate(path, MAX_LOG_BYTES)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .try_init()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        previous(info);
    }));
    Ok(())
}
```

Add to `src/lib.rs`: `pub mod diagnostics;` and `pub mod platform;`.

- [ ] **Step 8: Run tests**

Run: `cargo test --test platform_tests`
Expected: 6 passed.

- [ ] **Step 9: Full checks** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock src/lib.rs src/platform src/diagnostics.rs tests/platform_tests.rs
git commit -m "Add platform helpers, single-instance mutex and log rotation"
```

---

### Task 6: IPC over a user-only named pipe

**Files:**
- Modify: `src/lib.rs`
- Create: `src/ipc/mod.rs`, `src/ipc/pipe.rs`, `tests/ipc_tests.rs`

**Interfaces:**
- Consumes: `app::commands::Command`, `app::state::{Snapshot, Tab}`, `platform::wide`.
- Produces:
  - `ipc::{Request { GetState, OpenSettings { tab: Option<Tab> }, Command { command: Command } }, Response { Ok, State { snapshot: Box<Snapshot> }, Error { message: String } }, MAX_MESSAGE, encode, decode}`.
  - `ipc::pipe::{pipe_name() -> io::Result<String>, serve(String, Handler) -> io::Result<()>, Client, Handler}` where `type Handler = Arc<dyn Fn(Request) -> Response + Send + Sync>`; `Client::connect(&str, Duration) -> io::Result<Client>`, `Client::request(&mut self, &Request) -> io::Result<Response>`; `pipe::request_with_timeout(&str, Request, Duration) -> io::Result<Response>`.

- [ ] **Step 1: Write the failing tests**

`tests/ipc_tests.rs`:

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{Snapshot, Tab};
use obsid_ambilight::config::Config;
use obsid_ambilight::ipc::pipe::{self, Client};
use obsid_ambilight::ipc::{Request, Response, decode, encode};
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;

static N: AtomicU32 = AtomicU32::new(0);

fn unique_pipe() -> String {
    format!(r"\\.\pipe\obsid-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
}

fn snapshot() -> Snapshot {
    Controller::new(Config::default(), vec![], Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1))
        .snapshot(false)
}

#[test]
fn messages_round_trip_as_single_json_lines() {
    let req = Request::Command { command: Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: None } };
    let line = encode(&req);
    assert!(line.ends_with('\n') && !line.trim_end().contains('\n'));
    assert_eq!(decode::<Request>(&line).unwrap(), req);

    let resp = Response::State { snapshot: Box::new(snapshot()) };
    assert_eq!(decode::<Response>(&encode(&resp)).unwrap(), resp);
    assert_eq!(decode::<Request>(&encode(&Request::OpenSettings { tab: Some(Tab::Lighting) })).unwrap(), Request::OpenSettings { tab: Some(Tab::Lighting) });
}

#[test]
fn garbage_is_rejected() {
    assert!(decode::<Request>("not json").is_err());
    assert!(decode::<Request>("{\"req\":\"nope\"}").is_err());
}

#[test]
fn pipe_name_is_per_user_and_session() {
    let name = pipe::pipe_name().unwrap();
    assert!(name.starts_with(r"\\.\pipe\ObsidAmbilight-"), "{name}");
    assert!(name.contains("S-1-"), "{name}");
}

#[test]
fn request_response_over_a_real_pipe() {
    let name = unique_pipe();
    let snap = snapshot();
    let s2 = snap.clone();
    pipe::serve(
        name.clone(),
        Arc::new(move |req| match req {
            Request::GetState => Response::State { snapshot: Box::new(s2.clone()) },
            Request::Command { .. } => Response::Ok,
            Request::OpenSettings { .. } => Response::Error { message: "no".into() },
        }),
    )
    .unwrap();

    let mut a = Client::connect(&name, Duration::from_secs(2)).unwrap();
    let mut b = Client::connect(&name, Duration::from_secs(2)).unwrap();
    assert_eq!(a.request(&Request::GetState).unwrap(), Response::State { snapshot: Box::new(snap) });
    assert_eq!(b.request(&Request::Command { command: Command::Discover }).unwrap(), Response::Ok);
    assert_eq!(a.request(&Request::OpenSettings { tab: None }).unwrap(), Response::Error { message: "no".into() });
}

#[test]
fn connecting_to_a_missing_pipe_fails_fast() {
    let start = std::time::Instant::now();
    assert!(Client::connect(&unique_pipe(), Duration::from_millis(300)).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn request_with_timeout_gives_up_on_a_silent_server() {
    let name = unique_pipe();
    pipe::serve(
        name.clone(),
        Arc::new(|_| {
            std::thread::sleep(Duration::from_secs(5));
            Response::Ok
        }),
    )
    .unwrap();
    let start = std::time::Instant::now();
    let r = pipe::request_with_timeout(&name, Request::GetState, Duration::from_millis(500));
    assert!(r.is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test ipc_tests`
Expected: compile error — `obsid_ambilight::ipc` not found.

- [ ] **Step 3: Implement `src/ipc/mod.rs`**

```rust
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
```

- [ ] **Step 4: Implement `src/ipc/pipe.rs`**

```rust
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
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX};
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
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut())
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
    let first = create_instance(&w, &sd, true)?;
    thread::Builder::new().name("ipc-server".into()).spawn(move || {
        let mut next = Some(first);
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
                CreateFileW(w.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
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
```

Add `pub mod ipc;` to `src/lib.rs`.

- [ ] **Step 5: Run tests**

Run: `cargo test --test ipc_tests`
Expected: 6 passed.

- [ ] **Step 6: Full checks** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 7: Commit**

```bash
git add src/lib.rs src/ipc tests/ipc_tests.rs
git commit -m "Add named-pipe IPC restricted to the current user"
```

---

### Task 7: Icon rendering

**Files:**
- Modify: `src/lib.rs`
- Create: `src/ui/mod.rs`, `src/ui/icons.rs`, `tests/icons_tests.rs`

**Interfaces:**
- Consumes: `protocol::Rgb`, `app::state::{Snapshot, Connection}`.
- Produces: `ui::icons::{IconKind { Lit(Rgb), Off, Connecting, Offline }, render(IconKind, u32) -> Vec<u8>, kind_for(&Snapshot) -> IconKind}` — RGBA, unpremultiplied, rows top to bottom.

- [ ] **Step 1: Write the failing tests**

`tests/icons_tests.rs`:

```rust
use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::Connection;
use obsid_ambilight::config::Config;
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::icons::{IconKind, kind_for, render};

fn px(img: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * size + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

#[test]
fn lit_icon_is_a_filled_circle_of_the_colour() {
    let c = Rgb::new(200, 100, 50);
    let img = render(IconKind::Lit(c), 32);
    assert_eq!(img.len(), 32 * 32 * 4);
    assert_eq!(px(&img, 32, 16, 16), [200, 100, 50, 255]);
    assert_eq!(px(&img, 32, 0, 0)[3], 0, "corners are transparent");
}

#[test]
fn off_icon_is_a_ring() {
    let img = render(IconKind::Off, 32);
    assert_eq!(px(&img, 32, 16, 16)[3], 0, "hollow centre");
    let ring = (0..32).map(|x| px(&img, 32, x, 16)[3]).max().unwrap();
    assert_eq!(ring, 255);
}

#[test]
fn status_dots_differ_in_colour() {
    let a = render(IconKind::Connecting, 16);
    let b = render(IconKind::Offline, 16);
    assert_ne!(px(&a, 16, 8, 8), px(&b, 16, 8, 8));
    assert_eq!(px(&a, 16, 8, 8)[3], 255);
}

#[test]
fn icon_kind_follows_connection_and_power() {
    let mut s = Controller::new(Config::default(), vec![], Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1)).snapshot(false);
    s.connection = Connection::Ready;
    assert_eq!(kind_for(&s), IconKind::Lit(s.config.light.color));
    s.config.light.power = false;
    assert_eq!(kind_for(&s), IconKind::Off);
    s.connection = Connection::Connecting;
    assert_eq!(kind_for(&s), IconKind::Connecting);
    s.connection = Connection::NoDevice;
    s.searching = true;
    assert_eq!(kind_for(&s), IconKind::Connecting);
    s.searching = false;
    assert_eq!(kind_for(&s), IconKind::Offline);
    s.connection = Connection::Offline;
    assert_eq!(kind_for(&s), IconKind::Offline);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --test icons_tests` — Expected: compile error, `obsid_ambilight::ui` not found.

- [ ] **Step 3: Implement**

`src/ui/mod.rs` (Tasks 8–9 add `settings` and `tray`):

```rust
//! User interface: tray (background process) and settings window (`--settings`).

pub mod icons;
```

`src/ui/icons.rs`:

```rust
//! Status icons drawn in code (no image assets): anti-aliased circles.

use crate::app::state::{Connection, Snapshot};
use crate::protocol::Rgb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    /// Static mode: a disc in the current colour.
    Lit(Rgb),
    /// Lighting switched off: a grey ring.
    Off,
    /// Looking for / connecting to the strip: an amber dot.
    Connecting,
    /// Strip unavailable or not chosen: a red dot.
    Offline,
}

const GREY: Rgb = Rgb::new(0x9A, 0xA0, 0xA6);
const AMBER: Rgb = Rgb::new(0xF2, 0xB3, 0x3D);
const RED: Rgb = Rgb::new(0xE5, 0x48, 0x4D);

pub fn kind_for(s: &Snapshot) -> IconKind {
    match s.connection {
        Connection::Ready if s.config.light.power => IconKind::Lit(s.config.light.color),
        Connection::Ready => IconKind::Off,
        Connection::Connecting => IconKind::Connecting,
        Connection::NoDevice if s.searching => IconKind::Connecting,
        Connection::NoDevice | Connection::Offline => IconKind::Offline,
    }
}

/// RGBA pixels (unpremultiplied), `size`×`size`, rows top to bottom.
pub fn render(kind: IconKind, size: u32) -> Vec<u8> {
    let s = size as f32;
    let centre = s / 2.0;
    let (color, outer, inner) = match kind {
        IconKind::Lit(c) => (c, s * 0.44, 0.0),
        IconKind::Off => (GREY, s * 0.42, s * 0.42 - (s * 0.12).max(1.5)),
        IconKind::Connecting => (AMBER, s * 0.30, 0.0),
        IconKind::Offline => (RED, s * 0.30, 0.0),
    };
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 + 0.5 - centre).powi(2) + (y as f32 + 0.5 - centre).powi(2)).sqrt();
            let cover_outer = (outer - d + 0.5).clamp(0.0, 1.0);
            let cover_inner = if inner > 0.0 { (inner - d + 0.5).clamp(0.0, 1.0) } else { 0.0 };
            let alpha = ((cover_outer - cover_inner).max(0.0) * 255.0).round() as u8;
            out.extend_from_slice(&[color.r, color.g, color.b, alpha]);
        }
    }
    out
}
```

Add `pub mod ui;` to `src/lib.rs`.

- [ ] **Step 4: Run tests** — `cargo test --test icons_tests` — Expected: 4 passed.

- [ ] **Step 5: Full checks** — `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/ui tests/icons_tests.rs
git commit -m "Add code-drawn status icons"
```

---

### Task 8: Tray application and `main.rs`

**Files:**
- Modify: `src/ui/mod.rs`
- Create: `src/ui/tray.rs`, `src/main.rs`

**Interfaces:**
- Consumes: everything above — `Worker`, `WorkerHandle`, `WorkerOptions`, `Notice`, `UdpLink`, `Command`, `Snapshot`, `Tab`, `config::{load, Loaded}`, `diagnostics::init_logging`, `platform::{paths, instance, wide, ui_language, message_box}`, `ipc::{Request, Response, pipe}`, `i18n`, `ui::icons`.
- Produces: `ui::tray::run() -> ExitCode`; binary `obsid-ambilight` (`src/main.rs`) dispatching to `ui::tray::run()` or `ui::settings::run(Tab)` (Task 9 provides `settings::run`; until then `main.rs` calls a stub — see Step 3).

This task is Win32 glue; it is verified by building and by the manual checks in Step 6.

- [ ] **Step 1: Implement `src/ui/tray.rs`**

```rust
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
    let loaded = config::load(&cfg_path);
    let _ = diagnostics::init_logging(&log_path, loaded.config.advanced.log_level.as_tracing());
    info!(version = env!("CARGO_PKG_VERSION"), "starting");
    let lang = i18n::resolve(loaded.config.app.language, platform::ui_language());

    let pipe_name = match pipe::pipe_name() {
        Ok(n) => n,
        Err(e) => {
            error!(%e, "cannot build pipe name");
            return ExitCode::FAILURE;
        }
    };
    let _guard = match InstanceGuard::acquire(INSTANCE_MUTEX) {
        Ok(Some(g)) => g,
        Ok(None) => {
            info!("already running; asking it to open settings");
            let r = pipe::request_with_timeout(&pipe_name, Request::OpenSettings { tab: None }, Duration::from_secs(2));
            if !matches!(r, Ok(Response::Ok)) {
                platform::message_box("Obsid Ambilight", i18n::strings(lang).already_running);
            }
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            error!(%e, "single-instance mutex failed");
            return ExitCode::FAILURE;
        }
    };

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
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, self.hwnd, std::ptr::null());
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
        if let Some(child) = &mut self.settings {
            if matches!(child.try_wait(), Ok(None)) {
                bring_to_front(child.id());
                return;
            }
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
```

Update `src/ui/mod.rs`:

```rust
//! User interface: tray (background process) and settings window (`--settings`).

pub mod icons;
pub mod settings;
pub mod tray;
```

- [ ] **Step 2: Temporary settings stub**

Create `src/ui/settings/mod.rs` (replaced fully in Task 9):

```rust
//! Settings window (`--settings`); implemented in the next task.

use std::process::ExitCode;

use crate::app::state::Tab;

pub fn run(_tab: Tab) -> ExitCode {
    ExitCode::SUCCESS
}
```

- [ ] **Step 3: Implement `src/main.rs`**

```rust
// Release builds have no console window; debug builds keep one for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use obsid_ambilight::app::state::Tab;
use obsid_ambilight::ui;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--settings") {
        let tab = args
            .iter()
            .position(|a| a == "--tab")
            .and_then(|i| args.get(i + 1))
            .and_then(|t| Tab::from_arg(t))
            .unwrap_or_default();
        return ui::settings::run(tab);
    }
    ui::tray::run()
}
```

- [ ] **Step 4: Build and check**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release`
Expected: green; `target\release\obsid-ambilight.exe` exists.

- [ ] **Step 5: Smoke test (manual, with the strip)**

1. `target\release\obsid-ambilight.exe` → within ~4 s a coloured disc appears in the tray; tooltip shows `Static #… · …% · 192.168.1.85`.
2. Right-click → presets and brightness change the strip; «Выключить подсветку» turns it off (grey ring).
3. Run the exe again → no second icon (settings stub does nothing yet).
4. `taskkill /im explorer.exe /f & start explorer` → icon comes back.
5. Exit from the menu → icon disappears, process ends (Task Manager).
6. Check `%APPDATA%\ObsidAmbilight\config.toml` and `logs\obsid-ambilight.log`.
7. Check Apple Home still controls the strip.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/ui
git commit -m "Add tray application with menu, icons and settings launcher"
```

---

### Task 9: Settings window (egui)

**Files:**
- Modify: `Cargo.toml`, `src/ui/settings/mod.rs` (replace stub)
- Create: `src/ui/settings/device.rs`, `src/ui/settings/lighting.rs`, `src/ui/settings/general.rs`, `src/ui/settings/diagnostics.rs`

**Interfaces:**
- Consumes: `ipc::{Request, Response, pipe::{pipe_name, Client}}`, `app::commands::Command`, `app::state::{Snapshot, Tab, Connection, TestState, DeviceInfo}`, `config::{Language, ExitAction}`, `i18n`, `platform::{ui_language, open_folder, paths}`, `ui::icons`, `diagnostics::init_logging`.
- Produces: `ui::settings::run(Tab) -> ExitCode`; `ui::settings::diagnostics::summary(&Snapshot) -> String`.

- [ ] **Step 1: Add eframe**

Run: `cargo add eframe@0.36.2 --no-default-features --features default_fonts,glow,accesskit`

- [ ] **Step 2: Write the failing test for the pure summary**

Create `tests/settings_tests.rs`:

```rust
use std::net::Ipv4Addr;
use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{Connection, DeviceInfo};
use obsid_ambilight::config::Config;
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::settings::diagnostics::summary;

#[test]
fn summary_lists_key_facts_without_secrets() {
    let mut s = Controller::new(Config::default(), vec!["w".into()], Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1)).snapshot(true);
    s.connection = Connection::Ready;
    s.device = Some(DeviceInfo { id: "dcda0cd778bc".into(), name: String::new(), ip: Ipv4Addr::new(192, 168, 1, 85), fw_ver: Some("36".into()) });
    s.errors = vec!["timeout".into()];
    let text = summary(&s);
    for needle in ["Obsid Ambilight", env!("CARGO_PKG_VERSION"), "192.168.1.85", "dcda0cd778bc", "fw 36", "Ready", "timeout", "w"] {
        assert!(text.contains(needle), "missing {needle}: {text}");
    }
    assert!(!text.to_lowercase().contains("token"));
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --test settings_tests` — Expected: compile error, `diagnostics` module not found.

- [ ] **Step 4: Implement `src/ui/settings/mod.rs`**

```rust
//! Settings window: a separate process (`--settings`) that talks to the
//! background process over the pipe and exits when the window closes.

pub mod device;
pub mod diagnostics;
pub mod general;
pub mod lighting;

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use tracing::info;

use crate::app::commands::Command;
use crate::app::state::{Snapshot, Tab};
use crate::i18n::{self, Strings};
use crate::ipc::pipe::{self, Client};
use crate::ipc::{Request, Response};
use crate::platform::{self, paths};
use crate::ui::icons;

const POLL: Duration = Duration::from_millis(250);

pub fn run(tab: Tab) -> ExitCode {
    if let Ok(logs) = paths::logs_dir() {
        let _ = crate::diagnostics::init_logging(&logs.join("settings.log"), tracing::Level::INFO);
    }
    let icon = egui::IconData { rgba: icons::render(icons::IconKind::Lit(crate::protocol::Rgb::new(0xF5, 0xD4, 0xA1)), 64), width: 64, height: 64 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Obsid Ambilight")
            .with_inner_size([680.0, 480.0])
            .with_min_inner_size([560.0, 400.0])
            .with_icon(icon),
        ..Default::default()
    };
    let result = eframe::run_native("Obsid Ambilight", options, Box::new(move |cc| Ok(Box::new(SettingsApp::new(cc, tab)))));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(%e, "settings window failed");
            ExitCode::FAILURE
        }
    }
}

/// Connection to the background process, run on its own thread.
pub struct Backend {
    snapshot: Arc<Mutex<Option<Snapshot>>>,
    connected: Arc<AtomicBool>,
    tx: Sender<Command>,
}

impl Backend {
    fn spawn(ctx: egui::Context) -> Self {
        let snapshot = Arc::new(Mutex::new(None));
        let connected = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (s, c) = (Arc::clone(&snapshot), Arc::clone(&connected));
        thread::Builder::new()
            .name("settings-ipc".into())
            .spawn(move || link_loop(&ctx, &s, &c, &rx))
            .expect("spawn settings IPC thread");
        Self { snapshot, connected, tx }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    fn snapshot(&self) -> Option<Snapshot> {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

fn link_loop(ctx: &egui::Context, snap: &Mutex<Option<Snapshot>>, connected: &AtomicBool, rx: &Receiver<Command>) {
    let Ok(name) = pipe::pipe_name() else { return };
    loop {
        let mut client = match Client::connect(&name, Duration::from_millis(500)) {
            Ok(c) => c,
            Err(_) => {
                connected.store(false, Ordering::Relaxed);
                *snap.lock().unwrap_or_else(PoisonError::into_inner) = None;
                ctx.request_repaint();
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        connected.store(true, Ordering::Relaxed);
        loop {
            // Commands first so the next state already reflects them.
            let mut failed = false;
            if let Ok(cmd) = rx.recv_timeout(POLL) {
                for cmd in std::iter::once(cmd).chain(rx.try_iter()) {
                    if client.request(&Request::Command { command: cmd }).is_err() {
                        failed = true;
                        break;
                    }
                }
            }
            match (failed, client.request(&Request::GetState)) {
                (false, Ok(Response::State { snapshot })) => {
                    *snap.lock().unwrap_or_else(PoisonError::into_inner) = Some(*snapshot);
                    ctx.request_repaint();
                }
                _ => break,
            }
        }
    }
}

pub struct SettingsApp {
    tab: Tab,
    backend: Backend,
    ui_lang: u16,
    title: String,
    pub(crate) lighting: lighting::Edit,
    pub(crate) device: device::Edit,
    pub(crate) copied_at: Option<Instant>,
    started_backend: bool,
}

impl SettingsApp {
    fn new(cc: &eframe::CreationContext<'_>, tab: Tab) -> Self {
        info!(?tab, "settings window opened");
        Self {
            tab,
            backend: Backend::spawn(cc.egui_ctx.clone()),
            ui_lang: platform::ui_language(),
            title: String::new(),
            lighting: lighting::Edit::default(),
            device: device::Edit::default(),
            copied_at: None,
            started_backend: false,
        }
    }

    fn tab_label(s: &Strings, tab: Tab) -> &'static str {
        match tab {
            Tab::Device => s.tab_device,
            Tab::Lighting => s.tab_lighting,
            Tab::General => s.tab_general,
            Tab::Diagnostics => s.tab_diagnostics,
        }
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let snap = self.backend.snapshot();
        let language = snap.as_ref().map(|s| s.config.app.language).unwrap_or_default();
        let s = i18n::strings(i18n::resolve(language, self.ui_lang));
        if self.title != s.window_title {
            self.title = s.window_title.to_owned();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(self.title.clone()));
        }

        egui::Panel::left("tabs").exact_size(170.0).resizable(false).show(ui, |ui| {
            ui.add_space(8.0);
            for tab in Tab::ALL {
                ui.selectable_value(&mut self.tab, tab, Self::tab_label(s, tab));
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(snap) = snap else {
                ui.add_space(24.0);
                ui.label(s.backend_down);
                if ui.add_enabled(!self.started_backend, egui::Button::new(s.start_backend)).clicked() {
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                        self.started_backend = true;
                    }
                }
                return;
            };
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Device => device::show(ui, s, &snap, &self.backend, &mut self.device),
                Tab::Lighting => lighting::show(ui, s, &snap, &self.backend, &mut self.lighting),
                Tab::General => general::show(ui, s, &snap, &self.backend),
                Tab::Diagnostics => diagnostics::show(ui, s, &snap, &mut self.copied_at),
            });
        });
    }
}
```

- [ ] **Step 5: Implement `src/ui/settings/device.rs`**

```rust
//! Device tab: current strip, discovery list, manual IP, connection test, LED count.

use std::net::Ipv4Addr;

use eframe::egui;

use super::Backend;
use crate::app::commands::Command;
use crate::app::state::{Connection, Snapshot, TestState};
use crate::i18n::{self, Strings};
use crate::protocol::packets::MAX_LEDS;

#[derive(Default)]
pub struct Edit {
    manual_ip: Option<String>,
    ip_error: bool,
}

pub fn connection_text(s: &Strings, snap: &Snapshot) -> String {
    match snap.connection {
        Connection::Ready => {
            let ip = snap.device.as_ref().map(|d| d.ip.to_string()).unwrap_or_default();
            i18n::fill(s.status_connected, &[&ip])
        }
        Connection::Connecting => s.status_connecting.to_owned(),
        Connection::NoDevice => s.status_no_device.to_owned(),
        Connection::Offline => match snap.retry_in_secs {
            Some(secs) => format!("{} · {}", s.status_offline, i18n::fill(s.retry_in, &[&secs.to_string()])),
            None => s.status_offline.to_owned(),
        },
    }
}

pub fn show(ui: &mut egui::Ui, s: &Strings, snap: &Snapshot, backend: &Backend, edit: &mut Edit) {
    ui.heading(s.current_device);
    match &snap.device {
        Some(d) => {
            egui::Grid::new("current").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.label(s.field_name);
                ui.label(d.display_name());
                ui.end_row();
                ui.label(s.field_id);
                ui.label(if d.id.is_empty() { snap.config.device.id.as_str() } else { d.id.as_str() });
                ui.end_row();
                ui.label(s.field_ip);
                ui.label(d.ip.to_string());
                ui.end_row();
                ui.label(s.field_fw);
                ui.label(d.fw_ver.as_deref().unwrap_or("—"));
                ui.end_row();
                ui.label(s.field_status);
                ui.label(connection_text(s, snap));
                ui.end_row();
            });
            if !snap.verified {
                ui.weak(s.unverified_id);
            }
        }
        None => {
            ui.label(s.no_device);
            ui.label(connection_text(s, snap));
        }
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(s.test_btn).clicked() {
            backend.send(Command::TestConnection);
        }
        match &snap.test {
            TestState::Idle => {}
            TestState::Running => {
                ui.spinner();
                ui.label(s.test_running);
            }
            TestState::Ok { millis } => {
                ui.label(i18n::fill(s.test_ok, &[&millis.to_string()]));
            }
            TestState::Failed { message } => {
                ui.colored_label(ui.visuals().error_fg_color, i18n::fill(s.test_failed, &[message]));
            }
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.heading(s.found_devices);
        if ui.add_enabled(!snap.searching, egui::Button::new(s.search_btn)).clicked() {
            backend.send(Command::Discover);
        }
        if snap.searching {
            ui.spinner();
            ui.label(s.searching);
        }
    });
    if snap.discovered.is_empty() && !snap.searching {
        ui.weak(s.none_found);
    }
    for d in &snap.discovered {
        ui.horizontal(|ui| {
            ui.label(format!("{} · {} · {}", d.display_name(), d.ip, d.id));
            if d.id == snap.config.device.id && snap.config.device.manual_ip.is_none() {
                ui.weak(s.selected);
            } else if ui.button(s.select_btn).clicked() {
                backend.send(Command::SelectDevice { id: d.id.clone() });
            }
        });
    }

    ui.separator();
    ui.heading(s.manual_ip);
    ui.weak(s.manual_ip_hint);
    let text = edit.manual_ip.get_or_insert_with(|| snap.config.device.manual_ip.map(|ip| ip.to_string()).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(text).desired_width(160.0).hint_text("192.168.1.85"));
        if ui.button(s.connect_btn).clicked() {
            match text.trim().parse::<Ipv4Addr>() {
                Ok(ip) => {
                    edit.ip_error = false;
                    backend.send(Command::SetManualIp { ip: Some(ip) });
                }
                Err(_) => edit.ip_error = true,
            }
        }
        if snap.config.device.manual_ip.is_some() && ui.button(s.clear_btn).clicked() {
            text.clear();
            edit.ip_error = false;
            backend.send(Command::SetManualIp { ip: None });
        }
    });
    if edit.ip_error {
        ui.colored_label(ui.visuals().error_fg_color, s.invalid_ip);
    }

    ui.separator();
    let mut leds = snap.config.device.led_count;
    ui.horizontal(|ui| {
        ui.label(s.led_count);
        if ui.add(egui::DragValue::new(&mut leds).range(1..=MAX_LEDS)).changed() {
            backend.send(Command::SetLedCount { count: leds });
        }
    });
    ui.weak(s.led_count_hint);
}
```

- [ ] **Step 6: Implement `src/ui/settings/lighting.rs`**

```rust
//! Lighting tab: power, colour (HSV picker + HEX), brightness, presets.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32};

use super::Backend;
use crate::app::commands::Command;
use crate::app::state::Snapshot;
use crate::config::MAX_PRESETS;
use crate::i18n::Strings;
use crate::protocol::Rgb;

/// Local values win over the snapshot for this long after the user touched them,
/// so a slow round trip does not snap the controls back.
const HOLD: Duration = Duration::from_millis(800);

#[derive(Default)]
pub struct Edit {
    color: Option<Color32>,
    brightness: Option<u8>,
    hex: String,
    touched: Option<Instant>,
    black_warning: bool,
}

fn to_color32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

fn to_rgb(c: Color32) -> Rgb {
    Rgb::new(c.r(), c.g(), c.b())
}

pub fn show(ui: &mut egui::Ui, s: &Strings, snap: &Snapshot, backend: &Backend, edit: &mut Edit) {
    let light = &snap.config.light;
    let held = edit.touched.is_some_and(|t| t.elapsed() < HOLD);
    if !held {
        edit.color = Some(to_color32(light.color));
        edit.brightness = Some(light.brightness);
        edit.hex = light.color.to_string();
    }

    let mut power = light.power;
    if ui.checkbox(&mut power, s.lighting_power).changed() {
        backend.send(Command::SetPower { on: power });
    }

    ui.add_space(8.0);
    ui.label(s.color_label);
    let mut color = edit.color.unwrap_or(Color32::WHITE);
    if egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque) {
        edit.touched = Some(Instant::now());
        edit.color = Some(color);
        edit.hex = to_rgb(color).to_string();
        edit.black_warning = color == Color32::BLACK;
        if !edit.black_warning {
            backend.send(Command::SetStatic { color: Some(to_rgb(color)), brightness: None });
        }
    }
    ui.horizontal(|ui| {
        ui.label(s.hex_label);
        let r = ui.add(egui::TextEdit::singleline(&mut edit.hex).desired_width(90.0));
        if r.changed() {
            edit.touched = Some(Instant::now());
        }
        if r.lost_focus() {
            if let Some(c) = Rgb::from_hex(edit.hex.trim()) {
                edit.black_warning = c == Rgb::BLACK;
                if !edit.black_warning {
                    edit.color = Some(to_color32(c));
                    backend.send(Command::SetStatic { color: Some(c), brightness: None });
                }
            }
        }
    });
    if edit.black_warning {
        ui.colored_label(ui.visuals().warn_fg_color, s.black_not_allowed);
    }

    ui.add_space(8.0);
    let mut b = edit.brightness.unwrap_or(light.brightness);
    ui.horizontal(|ui| {
        ui.label(s.brightness_label);
        if ui.add(egui::Slider::new(&mut b, 1..=100).suffix(" %")).changed() {
            edit.touched = Some(Instant::now());
            edit.brightness = Some(b);
            backend.send(Command::SetStatic { color: None, brightness: Some(b) });
        }
    });

    ui.add_space(8.0);
    ui.label(s.presets_label);
    ui.weak(s.preset_hint);
    ui.horizontal_wrapped(|ui| {
        for (i, c) in light.presets.iter().enumerate() {
            let r = ui.add(egui::Button::new("").fill(to_color32(*c)).min_size(egui::vec2(28.0, 28.0))).on_hover_text(c.to_string());
            if r.clicked() {
                edit.touched = None;
                backend.send(Command::SetStatic { color: Some(*c), brightness: None });
            }
            if r.secondary_clicked() {
                let mut presets = light.presets.clone();
                presets.remove(i);
                backend.send(Command::SetPresets { presets });
            }
        }
        let current = to_rgb(edit.color.unwrap_or(to_color32(light.color)));
        let can_add = light.presets.len() < MAX_PRESETS && !light.presets.contains(&current) && current != Rgb::BLACK;
        if ui.add_enabled(can_add, egui::Button::new("+")).on_hover_text(s.add_preset).clicked() {
            let mut presets = light.presets.clone();
            presets.push(current);
            backend.send(Command::SetPresets { presets });
        }
    });
}
```

- [ ] **Step 7: Implement `src/ui/settings/general.rs`**

```rust
//! General tab: language, power-on at start, exit action.

use eframe::egui;

use super::Backend;
use crate::app::commands::Command;
use crate::app::state::Snapshot;
use crate::config::{ExitAction, Language};
use crate::i18n::Strings;

pub fn show(ui: &mut egui::Ui, s: &Strings, snap: &Snapshot, backend: &Backend) {
    let cfg = &snap.config;

    let mut language = cfg.app.language;
    let name = |l: Language| match l {
        Language::Auto => s.lang_auto,
        Language::Ru => "Русский",
        Language::En => "English",
    };
    ui.horizontal(|ui| {
        ui.label(s.language_label);
        egui::ComboBox::from_id_salt("language").selected_text(name(language)).show_ui(ui, |ui| {
            for l in [Language::Auto, Language::Ru, Language::En] {
                ui.selectable_value(&mut language, l, name(l));
            }
        });
    });
    if language != cfg.app.language {
        backend.send(Command::SetLanguage { language });
    }

    ui.add_space(8.0);
    let mut power_on = cfg.startup.power_on;
    if ui.checkbox(&mut power_on, s.power_on_start).changed() {
        backend.send(Command::SetPowerOnStart { enabled: power_on });
    }

    ui.add_space(8.0);
    ui.label(s.exit_action_label);
    let mut action = cfg.app.exit_action;
    ui.radio_value(&mut action, ExitAction::Keep, s.exit_keep);
    ui.radio_value(&mut action, ExitAction::Off, s.exit_off);
    if action != cfg.app.exit_action {
        backend.send(Command::SetExitAction { action });
    }
}
```

- [ ] **Step 8: Implement `src/ui/settings/diagnostics.rs`**

```rust
//! Diagnostics tab and the copyable plain-text summary (no secrets: the
//! snapshot never contains the session token).

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::state::Snapshot;
use crate::i18n::Strings;
use crate::platform::{self, paths};

pub fn summary(snap: &Snapshot) -> String {
    let mut t = String::new();
    let _ = writeln!(t, "Obsid Ambilight {}", snap.version);
    let _ = writeln!(t, "connection: {:?} (verified: {})", snap.connection, snap.verified);
    match &snap.device {
        Some(d) => {
            let _ = writeln!(t, "strip: id {} ip {} fw {}", d.id, d.ip, d.fw_ver.as_deref().unwrap_or("?"));
        }
        None => {
            let _ = writeln!(t, "strip: none (saved id {:?})", snap.config.device.id);
        }
    }
    let _ = writeln!(t, "session open: {}", snap.session_open);
    let _ = writeln!(t, "manual ip: {:?}", snap.config.device.manual_ip);
    let _ = writeln!(t, "errors:");
    for e in &snap.errors {
        let _ = writeln!(t, "  {e}");
    }
    let _ = writeln!(t, "config warnings:");
    for w in &snap.config_warnings {
        let _ = writeln!(t, "  {w}");
    }
    t
}

pub fn show(ui: &mut egui::Ui, s: &Strings, snap: &Snapshot, copied_at: &mut Option<Instant>) {
    egui::Grid::new("diag").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
        ui.label(s.version_label);
        ui.label(&snap.version);
        ui.end_row();
        ui.label(s.connection_label);
        ui.label(super::device::connection_text(s, snap));
        ui.end_row();
        ui.label(s.field_ip);
        ui.label(snap.device.as_ref().map(|d| d.ip.to_string()).unwrap_or_else(|| "—".into()));
        ui.end_row();
        ui.label(s.session_label);
        ui.label(if snap.session_open { s.session_open } else { s.session_closed });
        ui.end_row();
    });

    ui.add_space(8.0);
    ui.label(s.errors_label);
    if snap.errors.is_empty() {
        ui.weak(s.no_errors);
    }
    for e in snap.errors.iter().rev() {
        ui.monospace(e);
    }
    if !snap.config_warnings.is_empty() {
        ui.add_space(8.0);
        ui.label(s.config_warnings);
        for w in &snap.config_warnings {
            ui.monospace(w);
        }
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button(s.open_logs).clicked() {
            if let Ok(dir) = paths::logs_dir() {
                platform::open_folder(&dir);
            }
        }
        if ui.button(s.copy_summary).clicked() {
            ui.ctx().copy_text(summary(snap));
            *copied_at = Some(Instant::now());
        }
        if copied_at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            ui.weak(s.copied);
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    });
}
```

- [ ] **Step 9: Run tests and checks**

Run: `cargo test --test settings_tests` — Expected: 1 passed.
Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release`
Expected: green. If an egui 0.36 API name differs from this plan, read the local source under `%USERPROFILE%\.cargo\registry\src\*\egui-0.36.2\src\` and adapt; do not change behaviour.

- [ ] **Step 10: Manual check with the strip**

1. Start `target\release\obsid-ambilight.exe`, double-click the tray icon → window opens on «Устройство», current strip shown, Cyrillic renders (if boxes appear instead of letters, embed a Cyrillic-capable font — report before doing so).
2. «Подсветка»: drag the colour picker and brightness slider → the strip follows smoothly; the controls do not jump back.
3. Presets: click applies, right-click removes, «+» adds.
4. «Общие»: switch language → window title, tabs and tray menu switch immediately.
5. «Устройство» → «Проверить подключение» shows the reply time; «Искать» lists the strip.
6. Close the window → its process ends (Task Manager); double-click again → a new window; while open, «Настройки…» in the tray focuses the existing window.
7. Exit the background app while the window is open → the window shows «Фоновое приложение … не запущено» with «Запустить».
8. Apple Home still controls the strip.

- [ ] **Step 11: Commit**

```bash
git add Cargo.toml Cargo.lock src/ui/settings tests/settings_tests.rs
git commit -m "Add egui settings window"
```

---

### Task 10: Documentation, measurements, final verification

**Files:**
- Modify: `docs/testing.md`, `README.md`

- [ ] **Step 1: Stage 1 section in `docs/testing.md`**

Append:

```markdown
## Stage 1 on hardware

Run `target\release\obsid-ambilight.exe`. After each row, check that Apple Home
still controls the strip.

| # | Scenario | Expected | Result |
|---|---|---|---|
| S1 | First start without `config.toml` | Strip found and lit with the static colour; disc icon | ⏳ |
| S2 | Restart the app | Connects without searching UI, applies static colour | ⏳ |
| S3 | Tray: preset, brightness, power off/on | Strip follows; icon/tooltip update | ⏳ |
| S4 | Settings: colour picker drag, brightness slider | Smooth, ≤ 5 updates/s, final value applied | ⏳ |
| S5 | Unplug the strip, change colour, wait, plug back | Red icon «Устройство недоступно», retries grow; after power returns the last colour is applied | ⏳ |
| S6 | Launch the exe a second time | Settings open in the running app, no second icon | ⏳ |
| S7 | Restart Explorer | Icon returns | ⏳ |
| S8 | Exit with «Оставить цвет» / «Выключить ленту» | Strip keeps colour / turns off; process ends | ⏳ |
| S9 | Switch language RU ↔ EN | Tray menu and window switch at once | ⏳ |
| S10 | Corrupt `config.toml`, start | App starts with defaults, `config.bad-*.toml` kept, warning in Diagnostics | ⏳ |
| S11 | Memory/CPU in Static (background process) | ≤ 35 MB, ≤ 1 % CPU on average | ⏳ |
| S12 | Memory of the settings process while open | recorded | ⏳ |
```

- [ ] **Step 2: Measure**

With the app idle in Static for 5 minutes:

```powershell
Get-Process obsid-ambilight | Select-Object Id, @{n='WS_MB';e={[math]::Round($_.WorkingSet64/1MB,1)}}, @{n='Private_MB';e={[math]::Round($_.PrivateMemorySize64/1MB,1)}}, CPU
```

Run twice 60 s apart; CPU % = ΔCPU seconds / 60 / logical cores × 100. Record both processes (background and settings) in S11/S12.

- [ ] **Step 3: README**

Update the status table: Stage 0 rows stay; add «Stage 1 — tray, settings window, static control, reconnect, single instance: ✅ implemented, ⏳ hardware checklist». Add a «Running the app» section: build with `cargo build --release`, run `target\release\obsid-ambilight.exe`, settings via tray double-click, config and logs under `%APPDATA%\ObsidAmbilight`. Update the layout block with `app/`, `config.rs`, `i18n.rs`, `ipc/`, `platform/`, `ui/`, `diagnostics.rs`, `main.rs`.

- [ ] **Step 4: Final verification**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release`
Review `git diff HEAD~10 --stat` and skim every changed file once more.

- [ ] **Step 5: Commit**

```bash
git add docs/testing.md README.md
git commit -m "Document Stage 1 usage and hardware checklist"
```
