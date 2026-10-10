mod common;

use std::fs;
use std::net::Ipv4Addr;

use obsid_ambilight::config::{
    self, AmbilightConfig, Config, ExitAction, Language, LogLevel, MAX_PRESETS, Mode, Segment,
};
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

#[test]
fn stage1_file_loads_with_static_mode_and_default_layout() {
    let l = config::parse(SPEC_EXAMPLE).unwrap();
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    assert_eq!(l.config.app.mode, Mode::Static);
    assert_eq!(l.config.ambilight, AmbilightConfig::defaults(60));
}

#[test]
fn default_layout_follows_the_parsed_led_count() {
    let l = config::parse("[device]\nled_count = 30\n").unwrap();
    assert_eq!(l.config.ambilight.segments, obsid_ambilight::ambilight::layout::default_layout(30));
}

#[test]
fn ambilight_section_round_trips() {
    let dir = common::temp_dir("ambilight-round-trip");
    let path = dir.join("config.toml");
    let mut c = Config::default();
    c.app.mode = Mode::Ambilight;
    c.ambilight.monitor = r"\\.\DISPLAY2".into();
    c.ambilight.fps = 30;
    c.ambilight.saturation = 1.5;
    c.ambilight.segments = vec![
        Segment { from: 0, to: 9, start: [0.1, 0.9], end: [0.1, 0.1] },
        Segment { from: 30, to: 20, start: [0.25, 0.3], end: [0.75, 0.3] },
    ];
    config::save(&path, &c).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("mode = \"ambilight\""), "{text}");
    assert!(text.contains("[[ambilight.segments]]"), "{text}");
    let l = config::load(&path);
    assert!(l.warnings.is_empty(), "{:?}", l.warnings);
    assert_eq!(l.config, c);
}

#[test]
fn bad_ambilight_fields_fall_back_alone() {
    let l = config::parse(
        "[app]\nmode = \"rainbow\"\n[ambilight]\nfps = 99\ndepth = 1.0\nsaturation = \"x\"\nsmoothing = 200\nbrightness = 50\n",
    )
    .unwrap();
    let d = AmbilightConfig::defaults(60);
    assert_eq!(l.config.app.mode, Mode::Static);
    assert_eq!(l.config.ambilight.fps, d.fps);
    assert_eq!(l.config.ambilight.depth, d.depth);
    assert_eq!(l.config.ambilight.saturation, d.saturation);
    assert_eq!(l.config.ambilight.smoothing, d.smoothing);
    assert_eq!(l.config.ambilight.brightness, 50, "valid neighbours are kept");
    assert_eq!(l.warnings.len(), 5, "{:?}", l.warnings);
}

#[test]
fn invalid_segments_are_skipped_with_warnings() {
    let text = r#"
[[ambilight.segments]]
from = 0
to = 9
start = [0.0, 1.0]
end = [0.0, 0.0]

[[ambilight.segments]]
from = 5
to = 12
start = [0.0, 0.0]
end = [1.0, 0.0]

[[ambilight.segments]]
from = 20
to = 25
start = [1.5, 0.0]
end = [1.0, 0.0]

[[ambilight.segments]]
from = 70
to = 71
start = [0.0, 0.0]
end = [1.0, 0.0]

[[ambilight.segments]]
from = 40
start = [0.0, 0.0]
end = [1.0, 0.0]
"#;
    let l = config::parse(text).unwrap();
    assert_eq!(l.config.ambilight.segments, vec![Segment { from: 0, to: 9, start: [0.0, 1.0], end: [0.0, 0.0] }]);
    assert_eq!(l.warnings.len(), 4, "{:?}", l.warnings);
}

#[test]
fn ambilight_check_reports_out_of_range_settings() {
    let ok = AmbilightConfig::defaults(60);
    assert_eq!(ok.check(), Ok(()));
    for bad in [
        AmbilightConfig { fps: 9, ..ok.clone() },
        AmbilightConfig { depth: 0.3, ..ok.clone() },
        AmbilightConfig { brightness: 0, ..ok.clone() },
        AmbilightConfig { saturation: 2.5, ..ok.clone() },
        AmbilightConfig { smoothing: 101, ..ok.clone() },
        AmbilightConfig { black_threshold: 21, ..ok.clone() },
    ] {
        assert!(bad.check().is_err(), "{bad:?}");
    }
}
