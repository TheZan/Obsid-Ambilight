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

impl<'a> Reader<'a> {
    fn get(&self, section: &str, key: &str) -> Option<&'a Value> {
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
