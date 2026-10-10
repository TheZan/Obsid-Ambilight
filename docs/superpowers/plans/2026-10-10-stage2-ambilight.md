# Stage 2 — Ambilight Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A third strip mode, Ambilight: the edges of the chosen monitor are captured with DXGI Desktop Duplication, turned into per-LED colours through a free-form segment layout, and streamed to the Yeelight Obsid strip in direct mode, with a visual layout editor, HDR support, letterbox removal and automatic resume.

**Architecture:** Pure colour code (`src/ambilight/`) turns a small frame into LED colours. A capture thread (`src/capture/`) owns D3D11/DXGI, downsizes frames on the GPU with mipmaps and leaves the newest LED colours in a slot. The existing device worker stays the only owner of the strip connection: the pure `Controller` decides when to enter/leave direct mode, the worker paces frames (only changed frames, plus one per second) and keep-alives, and starts/stops the capture thread to match `Controller::capture_wanted()`.

**Tech Stack:** Rust 2024 (1.95), `windows` 0.62 (D3D11/DXGI COM), `windows-sys` 0.61 (+ `Win32_Devices_Display`), `eframe`/`egui` 0.36, `serde`, `toml`, `tracing` — plus the Stage 0–1 modules.

**Spec:** `docs/superpowers/specs/2026-10-10-stage2-ambilight-design.md`

## Global Constraints

- Platform: Windows 10/11 x64; no admin rights; no TCP/HTTP ports.
- One new dependency: `windows` 0.62 with features `Win32_Foundation`, `Win32_Graphics_Direct3D`, `Win32_Graphics_Direct3D11`, `Win32_Graphics_Dxgi`, `Win32_Graphics_Dxgi_Common`, `Win32_Graphics_Gdi`; plus the `windows-sys` feature `Win32_Devices_Display`. Nothing else.
- Config (`%APPDATA%\ObsidAmbilight\config.toml`): `[app] mode = "static" | "ambilight"`; `[ambilight]` `monitor` (empty = primary), `fps` 10–30 (default 25), `depth` 0.02–0.25 (0.08), `brightness` 1–100 (100), `saturation` 0.5–2.0 (1.2), `smoothing` 0–100 (25), `black_threshold` 0–20 (3), `letterbox` (true), `[[ambilight.segments]]` `from`, `to`, `start = [x, y]`, `end = [x, y]`. Stage 1 files load unchanged with `mode = static`.
- Segments: ≤ 16; indices < `led_count`; coordinates in [0, 1]; no shared LEDs; LEDs outside every segment are black.
- Direct mode: enter = new session → `set_power on` → `set_bright 100` → `activate_fx_mode direct`; leave = `deactivate_fx_mode direct` → static colour/brightness or `set_power off`. Ambilight brightness is applied in software.
- Frames: `update_leds` only when the colours changed or 1 s after the last send; keep-alive via `Chameleon2::tick` (3 s); 3 missed keep-alives = stream lost → Offline → resume Ambilight after reconnect.
- Capture runs only while streaming in Ambilight mode; a failed or lost grabber is reopened every 1 s; the strip keeps the last colours meanwhile.
- Layout highlight lasts 3 s after the last `Highlight`; the settings window repeats it every 1 s.
- Exit from a stream: `keep` → leave direct mode and apply the static colour; `off` → leave direct mode and power off; ≤ ~1 s.
- Logs and controller error strings in English; every UI string is a `Strings` field in RU and EN.
- Commits: one per task, subject line only, no body, no `Co-Authored-By`.
- Every task ends with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green.

## Review Focus

1. **The monitor disappears or changes mode during Ambilight** (cable unplugged, resolution/HDR switch, UAC): capture must reopen every second, never stall the device worker, and the strip keeps its last colours. Pinned in Task 7 (`reopens_after_a_lost_grabber`, `factory_failure_is_reported_and_retried`).
2. **The LED count is changed while streaming**: frames must follow the new length, the link must adapt instead of failing every frame with "frame has N colours". Pinned in Task 8 (`led_count_change_while_streaming_resizes_frames`).
3. **The settings window is closed (or crashes) during a highlight**: the strip must return to the user's mode by itself 3 s later. Pinned in Task 5 (`highlight_from_static_starts_and_ends_the_stream`) and Task 8 (`highlight_from_static_sends_markers_then_returns_to_static`).
4. **The strip loses power during Ambilight**: keep-alive failure must go Offline, stop capture, retry with backoff and resume streaming after reconnect. Pinned in Task 5 (`stream_failure_resumes_ambilight_after_reconnect`) and Task 8 (`stream_failure_goes_offline_and_resumes`).
5. **A hand-edited config with broken segments or out-of-range numbers**: startup must not fail; each bad segment/field is skipped with a warning. Pinned in Task 1 (`invalid_segments_are_skipped_with_warnings`, `bad_ambilight_fields_fall_back_alone`).

## Notes on the spec

- `Desired` is `Light(LightChange) | Ambilight` (`LightChange` already expresses Off/Static), so `Action::Apply { ip, change }` from Stage 1 stays.
- `Event::StreamStopped` carries the IP, so a failure to leave direct mode on a strip the user just switched away from does not mark the new strip offline.
- `Command::SetAmbilight` carries an `AmbilightConfig`; its `segments` are ignored (segments change only through `SetLayout`).
- The capture thread sleeps to the next frame time and then calls `AcquireNextFrame` with a 0 ms timeout, so a 240 Hz desktop costs at most `fps` GPU copies per second.
- Rotated (portrait) outputs are captured unrotated; not needed for this setup.

---

## File Structure

```text
Cargo.toml                         + windows 0.62 (Task 6), windows-sys Win32_Devices_Display (Task 6)
src/lib.rs                         + pub mod ambilight (Task 1), capture (Task 6)
src/config.rs                      Mode, Segment, AmbilightConfig, parsing/validation (Task 1)
src/ambilight/mod.rs               NEW module root (Task 1; modules added in Tasks 2–4, 7)
src/ambilight/layout.rs            NEW segments: points, checks, perimeter, trim, highlight frames (Task 1)
src/ambilight/image.rs             NEW small frame, area mean, sample square (Task 2)
src/ambilight/color.rs             NEW HDR→SDR, f16, adjustment, sRGB (Task 2)
src/ambilight/letterbox.rs         NEW black bars with hysteresis (Task 3)
src/ambilight/smooth.rs            NEW exponential smoothing + scene cut (Task 4)
src/ambilight/pipeline.rs          NEW frame → LED colours (Task 4)
src/ambilight/rate.rs              NEW frames-per-second meter (Task 7)
src/app/state.rs                   + CaptureStatus, StreamStatus, Snapshot.stream (Task 5), Tab::Ambilight (Task 10)
src/app/commands.rs                + SetMode, SetAmbilight, SetLayout, Highlight, StopHighlight (Task 5)
src/app/controller.rs              modes, streaming, highlight, exit (Task 5)
src/capture/mod.rs                 NEW Grabber trait, GrabberInfo, GrabberFactory (Task 6)
src/capture/outputs.rs             NEW DXGI output enumeration, MonitorInfo (Task 6)
src/capture/display.rs             NEW SDR white level (DisplayConfig) (Task 6)
src/capture/dxgi.rs                NEW DxgiGrabber (Task 6)
src/capture/thread.rs              NEW capture thread, CaptureSettings, Capture (Task 7)
src/protocol/device.rs             + set_led_count, leave_direct (Task 8)
src/app/worker.rs                  DeviceLink stream methods, frame pacing, capture lifecycle (Task 8)
src/app/link.rs                    UdpLink stream methods (Task 8)
src/ui/icons.rs                    + IconKind::Ambilight (Task 9)
src/ui/tray.rs                     mode items, ambilight brightness, tooltip, icon cache, grabber factory (Task 9)
src/i18n.rs                        new strings (Tasks 9, 10, 11)
src/ui/settings/mod.rs             Ambilight tab, 900×600 (Task 10), highlight stop on tab change (Task 11)
src/ui/settings/ambilight.rs       NEW tab: on/off, monitor, fps, picture (Task 10)
src/ui/settings/layout_editor.rs   NEW schematic, segment list, perimeter, highlight (Task 11)
src/ui/settings/diagnostics.rs     capture/stream rows and summary (Task 10)
src/ui/settings/device.rs          LED count hint text (Task 10)
tests/layout_tests.rs              NEW (Task 1)
tests/color_tests.rs               NEW (Task 2)
tests/letterbox_tests.rs           NEW (Task 3)
tests/pipeline_tests.rs            NEW (Task 4)
tests/capture_tests.rs             NEW (Task 6, real GPU tests #[ignore])
tests/capture_thread_tests.rs      NEW (Task 7)
tests/tray_tests.rs                NEW (Task 9)
tests/config_tests.rs, controller_tests.rs, worker_tests.rs, protocol_tests.rs, icons_tests.rs, settings_tests.rs   extended
README.md, docs/testing.md, docs/protocol.md   Stage 2 (Task 12)
```

---

### Task 1: Config model — mode, Ambilight settings and segment layout

**Files:**
- Create: `src/ambilight/mod.rs`, `src/ambilight/layout.rs`, `tests/layout_tests.rs`
- Modify: `src/config.rs`, `src/lib.rs`, `tests/config_tests.rs`

**Interfaces:**
- Consumes: `crate::protocol::Rgb`, `crate::protocol::packets::MAX_LEDS`.
- Produces:
  - `config::Mode { Static, Ambilight }` (serde lowercase), `AppConfig.mode: Mode`.
  - `config::Segment { from: usize, to: usize, start: [f64; 2], end: [f64; 2] }`.
  - `config::AmbilightConfig { monitor: String, fps: u32, depth: f64, brightness: u8, saturation: f64, smoothing: u8, black_threshold: u8, letterbox: bool, segments: Vec<Segment> }`, `AmbilightConfig::defaults(led_count) -> Self`, `AmbilightConfig::check(&self) -> Result<(), String>` (scalar fields only); `Config.ambilight: AmbilightConfig`.
  - Constants `config::{FPS_RANGE, DEPTH_RANGE, SATURATION_RANGE, SMOOTHING_MAX, BLACK_THRESHOLD_MAX}`.
  - `ambilight::layout::{MAX_SEGMENTS, EDGE_INSET, SegmentError, HighlightTarget, check, errors, sanitize, led_points, perimeter, default_counts, default_layout, trim, highlight_frame}` and `Segment::{lo, hi, count, leds, overlaps, point, reversed}`.

- [ ] **Step 1: Write the failing layout tests**

Create `tests/layout_tests.rs`:

```rust
use obsid_ambilight::ambilight::layout::{
    self, EDGE_INSET, HighlightTarget, MAX_SEGMENTS, SegmentError, default_counts, highlight_frame, led_points,
    perimeter, sanitize, trim,
};
use obsid_ambilight::config::Segment;
use obsid_ambilight::protocol::Rgb;

fn seg(from: usize, to: usize, start: [f64; 2], end: [f64; 2]) -> Segment {
    Segment { from, to, start, end }
}

fn close(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
}

#[test]
fn leds_are_spread_evenly_along_the_line() {
    let s = seg(0, 3, [0.0, 0.0], [1.0, 0.0]);
    assert_eq!(s.count(), 4);
    assert_eq!(s.leds(), vec![0, 1, 2, 3]);
    assert!(close(s.point(0), [0.125, 0.0]));
    assert!(close(s.point(3), [0.875, 0.0]));
}

#[test]
fn reversed_indices_run_backwards() {
    let s = seg(9, 6, [0.0, 0.5], [0.0, 0.1]);
    assert_eq!(s.leds(), vec![9, 8, 7, 6]);
    let points = led_points(&[s], 10);
    assert!(close(points[9].unwrap(), [0.0, 0.45]), "LED `from` sits at the start");
    assert!(close(points[6].unwrap(), [0.0, 0.15]));
    assert!(points[0].is_none(), "LEDs outside every segment have no point");
}

#[test]
fn single_led_segment_sits_in_the_middle() {
    let s = seg(5, 5, [0.2, 0.2], [0.4, 0.6]);
    assert!(close(s.point(0), [0.3, 0.4]));
}

#[test]
fn reversing_swaps_the_order_but_keeps_the_line() {
    let s = seg(0, 9, [0.1, 0.2], [0.3, 0.4]);
    assert_eq!(s.reversed(), seg(9, 0, [0.1, 0.2], [0.3, 0.4]));
}

#[test]
fn check_flags_indices_and_points() {
    assert_eq!(layout::check(&seg(0, 59, [0.0, 0.0], [1.0, 1.0]), 60), Ok(()));
    assert_eq!(layout::check(&seg(0, 60, [0.0, 0.0], [1.0, 1.0]), 60), Err(SegmentError::OutOfRange { led_count: 60 }));
    assert_eq!(layout::check(&seg(0, 5, [1.5, 0.0], [1.0, 1.0]), 60), Err(SegmentError::BadPoint));
    assert_eq!(layout::check(&seg(0, 5, [f64::NAN, 0.0], [1.0, 1.0]), 60), Err(SegmentError::BadPoint));
}

#[test]
fn errors_point_at_the_earlier_overlapping_segment() {
    let segs = [seg(0, 9, [0.0, 0.0], [1.0, 0.0]), seg(20, 25, [0.0, 0.0], [1.0, 0.0]), seg(12, 5, [0.0, 0.0], [1.0, 0.0])];
    assert_eq!(layout::errors(&segs, 60), vec![None, None, Some(SegmentError::Overlap(0))]);
}

#[test]
fn sanitize_drops_bad_overlapping_and_extra_segments() {
    let mut input = vec![
        seg(0, 9, [0.0, 0.0], [1.0, 0.0]),
        seg(5, 12, [0.0, 0.0], [1.0, 0.0]),
        seg(130, 131, [0.0, 0.0], [1.0, 0.0]),
    ];
    for i in 0..20 {
        input.push(seg(20 + i * 2, 21 + i * 2, [0.0, 0.0], [1.0, 0.0]));
    }
    let (kept, warnings) = sanitize(input, 120);
    assert_eq!(kept.len(), MAX_SEGMENTS);
    assert_eq!(kept[0], seg(0, 9, [0.0, 0.0], [1.0, 0.0]));
    assert!(warnings.iter().any(|w| w.contains("segments[1]")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("segments[2]")), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("only the first 16")), "{warnings:?}");
}

#[test]
fn perimeter_runs_clockwise_from_the_bottom_left() {
    let p = perimeter([11, 19, 11, 19]);
    let (a, b) = (EDGE_INSET, 1.0 - EDGE_INSET);
    assert_eq!(
        p,
        vec![
            seg(0, 10, [a, b], [a, a]),
            seg(11, 29, [a, a], [b, a]),
            seg(30, 40, [b, a], [b, b]),
            seg(41, 59, [b, b], [a, b]),
        ]
    );
    assert_eq!(perimeter([0, 5, 0, 0]), vec![seg(0, 4, [a, a], [b, a])], "empty sides are skipped");
}

#[test]
fn default_counts_follow_16_by_9_and_sum_up() {
    assert_eq!(default_counts(60), [11, 19, 11, 19]);
    for n in 1..=120 {
        assert_eq!(default_counts(n).iter().sum::<usize>(), n, "n = {n}");
    }
    assert_eq!(layout::default_layout(60), perimeter([11, 19, 11, 19]));
}

#[test]
fn trim_keeps_led_positions() {
    let fwd = seg(0, 9, [0.0, 0.0], [1.0, 0.0]);
    let rev = seg(19, 10, [0.0, 1.0], [1.0, 1.0]);
    let beyond = seg(30, 35, [0.0, 0.5], [1.0, 0.5]);
    let before = led_points(&[fwd.clone(), rev.clone()], 40);
    let t = trim(&[fwd, rev, beyond], 15);
    assert_eq!(t.len(), 2, "segments starting beyond the count are dropped");
    assert_eq!((t[0].from, t[0].to), (0, 9));
    assert_eq!((t[1].from, t[1].to), (14, 10));
    let after = led_points(&t, 15);
    for i in 0..15 {
        match (before[i], after[i]) {
            (Some(x), Some(y)) => assert!(close(x, y), "LED {i}: {x:?} vs {y:?}"),
            (x, y) => assert_eq!(x.is_some(), y.is_some(), "LED {i}"),
        }
    }
}

#[test]
fn led_highlight_marks_direction() {
    let f = highlight_frame(HighlightTarget::Led(12), &[], 30);
    assert_eq!(f.len(), 30);
    assert_eq!(f[12], Rgb::new(255, 255, 255));
    assert_eq!(f[11], Rgb::new(48, 0, 0), "before: dim red");
    assert_eq!(f[2], Rgb::new(48, 0, 0));
    assert_eq!(f[1], Rgb::BLACK);
    assert_eq!(f[13], Rgb::new(0, 48, 0), "after: dim green");
    assert_eq!(f[22], Rgb::new(0, 48, 0));
    assert_eq!(f[23], Rgb::BLACK);
}

#[test]
fn segment_highlight_is_a_green_to_blue_gradient() {
    let segs = [seg(0, 4, [0.0, 0.0], [1.0, 0.0]), seg(9, 5, [0.0, 1.0], [1.0, 1.0])];
    let f = highlight_frame(HighlightTarget::Segment(1), &segs, 12);
    assert_eq!(f[9], Rgb::new(0, 255, 0), "`from` is green");
    assert_eq!(f[5], Rgb::new(0, 0, 255), "`to` is blue");
    assert_eq!(f[0], Rgb::BLACK, "other segments are dark");
    assert_eq!(highlight_frame(HighlightTarget::Segment(7), &segs, 12), vec![Rgb::BLACK; 12]);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test layout_tests`
Expected: compile error — `obsid_ambilight::ambilight` and `config::Segment` do not exist.

- [ ] **Step 3: Add the config types**

In `src/config.rs`:

1. Change the imports at the top:

```rust
use std::fs;
use std::io::{self, Write};
use std::net::Ipv4Addr;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::ambilight::layout;
use crate::protocol::Rgb;
use crate::protocol::packets::MAX_LEDS;
```

2. After `pub const MAX_PRESETS: usize = 16;` add:

```rust
pub const FPS_RANGE: RangeInclusive<u32> = 10..=30;
pub const DEPTH_RANGE: RangeInclusive<f64> = 0.02..=0.25;
pub const SATURATION_RANGE: RangeInclusive<f64> = 0.5..=2.0;
pub const SMOOTHING_MAX: u8 = 100;
pub const BLACK_THRESHOLD_MAX: u8 = 20;
```

3. After the `LogLevel` impl add:

```rust
/// What the strip shows while it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Static,
    Ambilight,
}

/// LEDs `from..=to` (or backwards when `from > to`) laid evenly along the line
/// `start → end`; points are screen fractions 0..1, front view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub from: usize,
    pub to: usize,
    pub start: [f64; 2],
    pub end: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmbilightConfig {
    /// DXGI output name such as `\\.\DISPLAY1`; empty = the primary monitor.
    pub monitor: String,
    pub fps: u32,
    /// Side of the sampled square as a share of the shorter screen side.
    pub depth: f64,
    /// Software brightness, percent.
    pub brightness: u8,
    pub saturation: f64,
    /// 0..=100, mapped to a 0–500 ms time constant.
    pub smoothing: u8,
    /// Percent; darker LEDs are switched off.
    pub black_threshold: u8,
    /// Leave letterbox/pillarbox bars out of the sampled area.
    pub letterbox: bool,
    pub segments: Vec<Segment>,
}

impl AmbilightConfig {
    pub fn defaults(led_count: usize) -> Self {
        Self {
            monitor: String::new(),
            fps: 25,
            depth: 0.08,
            brightness: 100,
            saturation: 1.2,
            smoothing: 25,
            black_threshold: 3,
            letterbox: true,
            segments: layout::default_layout(led_count),
        }
    }

    /// Checks every setting except the segments; returns the first problem.
    pub fn check(&self) -> Result<(), String> {
        if !FPS_RANGE.contains(&self.fps) {
            return Err(format!("frame rate {} is outside {}..={}", self.fps, FPS_RANGE.start(), FPS_RANGE.end()));
        }
        if !DEPTH_RANGE.contains(&self.depth) {
            return Err(format!("sample depth {} is outside {:?}", self.depth, DEPTH_RANGE));
        }
        if !(1..=100).contains(&self.brightness) {
            return Err(format!("brightness {} is outside 1..=100", self.brightness));
        }
        if !SATURATION_RANGE.contains(&self.saturation) {
            return Err(format!("saturation {} is outside {:?}", self.saturation, SATURATION_RANGE));
        }
        if self.smoothing > SMOOTHING_MAX {
            return Err(format!("smoothing {} is above {SMOOTHING_MAX}", self.smoothing));
        }
        if self.black_threshold > BLACK_THRESHOLD_MAX {
            return Err(format!("black threshold {} is above {BLACK_THRESHOLD_MAX}", self.black_threshold));
        }
        Ok(())
    }
}
```

4. Add `pub ambilight: AmbilightConfig,` as the **last** field of `Config` (after `advanced`; TOML needs the segment tables last) and `pub mode: Mode,` as the last field of `AppConfig`.

5. In `impl Default for Config`: `app: AppConfig { language: Language::Auto, exit_action: ExitAction::Keep, mode: Mode::Static },` and after `advanced`: `ambilight: AmbilightConfig::defaults(60),`.

6. Replace `parse` with:

```rust
/// Parses config text. Errors only when the text is not TOML at all.
pub fn parse(text: &str) -> Result<Loaded, String> {
    let table: Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut r = Reader { table: &table, warnings: Vec::new() };
    let d = Config::default();
    let device = DeviceConfig {
        id: r.field("device", "id", d.device.id, as_string),
        name: r.field("device", "name", d.device.name, as_string),
        model: r.field("device", "model", d.device.model, as_string),
        last_ip: r.field("device", "last_ip", d.device.last_ip, as_opt_ip),
        manual_ip: r.field("device", "manual_ip", d.device.manual_ip, as_opt_ip),
        led_count: r.field("device", "led_count", d.device.led_count, |v| {
            v.as_integer().and_then(|n| usize::try_from(n).ok()).filter(|n| (1..=MAX_LEDS).contains(n))
        }),
    };
    let ambilight = r.ambilight(device.led_count);
    let config = Config {
        version: CONFIG_VERSION,
        device,
        light: LightConfig {
            power: r.field("static", "power", d.light.power, |v| match v.as_str()? {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            }),
            color: r.field("static", "color", d.light.color, |v| {
                v.as_str().and_then(Rgb::from_hex).filter(|c| *c != Rgb::BLACK)
            }),
            brightness: r.field("static", "brightness", d.light.brightness, |v| as_u8(v).filter(|b| (1..=100).contains(b))),
            presets: r.presets(d.light.presets),
        },
        startup: StartupConfig { power_on: r.field("startup", "power_on", d.startup.power_on, Value::as_bool) },
        app: AppConfig {
            language: r.field("app", "language", d.app.language, choice),
            exit_action: r.field("app", "exit_action", d.app.exit_action, choice),
            mode: r.field("app", "mode", d.app.mode, choice),
        },
        advanced: AdvancedConfig { log_level: r.field("advanced", "log_level", d.advanced.log_level, choice) },
        ambilight,
    };
    Ok(Loaded { config, warnings: r.warnings })
}
```

7. Add to `impl<'a> Reader<'a>`:

```rust
    fn ambilight(&mut self, led_count: usize) -> AmbilightConfig {
        let d = AmbilightConfig::defaults(led_count);
        let s = "ambilight";
        AmbilightConfig {
            monitor: self.field(s, "monitor", d.monitor, as_string),
            fps: self.field(s, "fps", d.fps, |v| {
                v.as_integer().and_then(|n| u32::try_from(n).ok()).filter(|n| FPS_RANGE.contains(n))
            }),
            depth: self.field(s, "depth", d.depth, |v| as_f64(v).filter(|x| DEPTH_RANGE.contains(x))),
            brightness: self.field(s, "brightness", d.brightness, |v| as_u8(v).filter(|b| (1..=100).contains(b))),
            saturation: self.field(s, "saturation", d.saturation, |v| as_f64(v).filter(|x| SATURATION_RANGE.contains(x))),
            smoothing: self.field(s, "smoothing", d.smoothing, |v| as_u8(v).filter(|x| *x <= SMOOTHING_MAX)),
            black_threshold: self.field(s, "black_threshold", d.black_threshold, |v| {
                as_u8(v).filter(|x| *x <= BLACK_THRESHOLD_MAX)
            }),
            letterbox: self.field(s, "letterbox", d.letterbox, Value::as_bool),
            segments: self.segments(led_count, d.segments),
        }
    }

    fn segments(&mut self, led_count: usize, default: Vec<Segment>) -> Vec<Segment> {
        let Some(v) = self.get("ambilight", "segments") else { return default };
        let Some(items) = v.as_array() else {
            self.warnings.push(format!("ambilight.segments: invalid value {v}, using the default layout"));
            return default;
        };
        let mut parsed = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match parse_segment(item) {
                Some(seg) => parsed.push(seg),
                None => self.warnings.push(format!("ambilight.segments[{i}]: invalid entry {item}, skipped")),
            }
        }
        let (clean, warnings) = layout::sanitize(parsed, led_count);
        self.warnings.extend(warnings);
        clean
    }
```

8. Next to `as_string` add:

```rust
fn as_u8(v: &Value) -> Option<u8> {
    v.as_integer().and_then(|n| u8::try_from(n).ok())
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|n| n as f64))
}

fn parse_segment(v: &Value) -> Option<Segment> {
    let t = v.as_table()?;
    let index = |k: &str| -> Option<usize> { t.get(k)?.as_integer().and_then(|n| usize::try_from(n).ok()) };
    let point = |k: &str| -> Option<[f64; 2]> {
        match t.get(k)?.as_array()?.as_slice() {
            [x, y] => Some([as_f64(x)?, as_f64(y)?]),
            _ => None,
        }
    };
    Some(Segment { from: index("from")?, to: index("to")?, start: point("start")?, end: point("end")? })
}
```

9. Add the `Choice` impl:

```rust
impl Choice for Mode {
    const NAMES: &'static [(&'static str, Self)] = &[("static", Self::Static), ("ambilight", Self::Ambilight)];
}
```

- [ ] **Step 4: Add the layout module**

Create `src/ambilight/mod.rs`:

```rust
//! Ambilight: screen-edge colours for the strip. Everything here is plain
//! Rust without Win32; screen capture lives in `crate::capture`.

pub mod layout;
```

In `src/lib.rs` add `pub mod ambilight;` (alphabetical, before `pub mod app;`).

Create `src/ambilight/layout.rs`:

```rust
//! Strip layout: segments of LEDs laid along lines on the screen (front view).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::config::Segment;
use crate::protocol::Rgb;

pub const MAX_SEGMENTS: usize = 16;
/// Distance of the perimeter template from the screen edge.
pub const EDGE_INSET: f64 = 0.02;
/// LEDs marked on each side of the one being looked for.
const NEIGHBOURS: usize = 10;
const BEFORE: Rgb = Rgb::new(48, 0, 0);
const AFTER: Rgb = Rgb::new(0, 48, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentError {
    /// An index is not below the LED count.
    OutOfRange { led_count: usize },
    /// A coordinate is outside 0..=1 or not finite.
    BadPoint,
    /// Shares LEDs with the segment at this list position.
    Overlap(usize),
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { led_count } => write!(f, "LED index outside 0..{led_count}"),
            Self::BadPoint => f.write_str("point outside the screen"),
            Self::Overlap(i) => write!(f, "shares LEDs with segment {}", i + 1),
        }
    }
}

/// What the layout editor lights on the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "index", rename_all = "snake_case")]
pub enum HighlightTarget {
    Led(usize),
    Segment(usize),
}

impl Segment {
    pub fn lo(&self) -> usize {
        self.from.min(self.to)
    }

    pub fn hi(&self) -> usize {
        self.from.max(self.to)
    }

    /// Number of LEDs (never zero).
    pub fn count(&self) -> usize {
        self.hi() - self.lo() + 1
    }

    /// LED indices from `from` to `to`, in segment order.
    pub fn leds(&self) -> Vec<usize> {
        if self.from <= self.to { (self.from..=self.to).collect() } else { (self.to..=self.from).rev().collect() }
    }

    pub fn overlaps(&self, other: &Segment) -> bool {
        self.lo() <= other.hi() && other.lo() <= self.hi()
    }

    /// Screen point of the `k`-th LED of the segment (0 = `from`).
    pub fn point(&self, k: usize) -> [f64; 2] {
        let t = (k as f64 + 0.5) / self.count() as f64;
        [self.start[0] + (self.end[0] - self.start[0]) * t, self.start[1] + (self.end[1] - self.start[1]) * t]
    }

    /// Same line, LEDs in the opposite order.
    pub fn reversed(&self) -> Segment {
        Segment { from: self.to, to: self.from, start: self.start, end: self.end }
    }
}

pub fn check(seg: &Segment, led_count: usize) -> Result<(), SegmentError> {
    if seg.hi() >= led_count {
        return Err(SegmentError::OutOfRange { led_count });
    }
    let inside = |p: [f64; 2]| p.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c));
    if !inside(seg.start) || !inside(seg.end) {
        return Err(SegmentError::BadPoint);
    }
    Ok(())
}

/// The problem of each segment, in list order; an overlap names the first
/// earlier segment it collides with.
pub fn errors(segments: &[Segment], led_count: usize) -> Vec<Option<SegmentError>> {
    segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            check(s, led_count)
                .err()
                .or_else(|| segments[..i].iter().position(|e| e.overlaps(s)).map(SegmentError::Overlap))
        })
        .collect()
}

/// Keeps valid segments that do not overlap an already kept one, at most
/// [`MAX_SEGMENTS`]; warns about everything dropped.
pub fn sanitize(segments: Vec<Segment>, led_count: usize) -> (Vec<Segment>, Vec<String>) {
    let mut kept: Vec<Segment> = Vec::new();
    let mut warnings = Vec::new();
    for (i, s) in segments.into_iter().enumerate() {
        let problem =
            check(&s, led_count).err().or_else(|| kept.iter().position(|k| k.overlaps(&s)).map(SegmentError::Overlap));
        match problem {
            Some(e) => warnings.push(format!("ambilight.segments[{i}]: {e}, skipped")),
            None if kept.len() == MAX_SEGMENTS => {
                warnings.push(format!("ambilight.segments: only the first {MAX_SEGMENTS} segments are kept"));
                break;
            }
            None => kept.push(s),
        }
    }
    (kept, warnings)
}

/// Screen point of every LED; `None` for LEDs outside all segments.
pub fn led_points(segments: &[Segment], led_count: usize) -> Vec<Option<[f64; 2]>> {
    let mut out = vec![None; led_count];
    for s in segments {
        for (k, led) in s.leds().into_iter().enumerate() {
            if let Some(slot) = out.get_mut(led) {
                *slot = Some(s.point(k));
            }
        }
    }
    out
}

/// Four sides from LED 0, clockwise seen from the front: left (bottom → top),
/// top (left → right), right (top → bottom), bottom (right → left).
pub fn perimeter(counts: [usize; 4]) -> Vec<Segment> {
    let (a, b) = (EDGE_INSET, 1.0 - EDGE_INSET);
    let lines = [([a, b], [a, a]), ([a, a], [b, a]), ([b, a], [b, b]), ([b, b], [a, b])];
    let mut next = 0;
    let mut out = Vec::new();
    for (n, (start, end)) in counts.into_iter().zip(lines) {
        if n == 0 {
            continue;
        }
        out.push(Segment { from: next, to: next + n - 1, start, end });
        next += n;
    }
    out
}

/// `[left, top, right, bottom]` for `led_count` LEDs around a 16:9 screen.
pub fn default_counts(led_count: usize) -> [usize; 4] {
    let top = led_count * 16 / 50;
    let rest = led_count - 2 * top;
    let left = rest / 2;
    [left, top, rest - left, top]
}

pub fn default_layout(led_count: usize) -> Vec<Segment> {
    perimeter(default_counts(led_count))
}

/// Fits segments to a smaller LED count: drops segments that start beyond it
/// and shortens the rest so the remaining LEDs keep their screen points.
pub fn trim(segments: &[Segment], led_count: usize) -> Vec<Segment> {
    segments
        .iter()
        .filter(|s| s.lo() < led_count)
        .map(|s| {
            if s.hi() < led_count {
                return s.clone();
            }
            let keep = (led_count - s.lo()) as f64 / s.count() as f64;
            let toward = |a: [f64; 2], b: [f64; 2]| [a[0] + (b[0] - a[0]) * keep, a[1] + (b[1] - a[1]) * keep];
            if s.from <= s.to {
                Segment { from: s.from, to: led_count - 1, start: s.start, end: toward(s.start, s.end) }
            } else {
                Segment { from: led_count - 1, to: s.to, start: toward(s.end, s.start), end: s.end }
            }
        })
        .collect()
}

/// Frame shown by the layout editor: one LED white with dim red before and
/// dim green after it, or one segment as a green (`from`) → blue (`to`) gradient.
pub fn highlight_frame(target: HighlightTarget, segments: &[Segment], led_count: usize) -> Vec<Rgb> {
    let mut frame = vec![Rgb::BLACK; led_count];
    match target {
        HighlightTarget::Led(i) => {
            for (j, c) in frame.iter_mut().enumerate() {
                *c = if j == i {
                    Rgb::new(255, 255, 255)
                } else if j < i && i - j <= NEIGHBOURS {
                    BEFORE
                } else if j > i && j - i <= NEIGHBOURS {
                    AFTER
                } else {
                    Rgb::BLACK
                };
            }
        }
        HighlightTarget::Segment(k) => {
            if let Some(s) = segments.get(k) {
                let leds = s.leds();
                let n = leds.len();
                for (pos, led) in leds.into_iter().enumerate() {
                    let t = if n > 1 { pos as f64 / (n - 1) as f64 } else { 0.0 };
                    if let Some(c) = frame.get_mut(led) {
                        *c = Rgb::new(0, (255.0 * (1.0 - t)).round() as u8, (255.0 * t).round() as u8);
                    }
                }
            }
        }
    }
    frame
}
```

- [ ] **Step 5: Run the layout tests**

Run: `cargo test --test layout_tests`
Expected: PASS (12 tests).

- [ ] **Step 6: Write the failing config tests**

Append to `tests/config_tests.rs` (add `AmbilightConfig, Mode, Segment` to the `config::{…}` import):

```rust
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
```

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: PASS. Existing tests that build `AppConfig`/`Config` literally (if any fail to compile) get `mode: Mode::Static` / `..Config::default()`.

- [ ] **Step 8: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/ambilight src/config.rs src/lib.rs tests/layout_tests.rs tests/config_tests.rs
git commit -m "Add Ambilight settings and segment layout model"
```

---

### Task 2: Small frame and colour maths

**Files:**
- Create: `src/ambilight/image.rs`, `src/ambilight/color.rs`, `tests/color_tests.rs`
- Modify: `src/ambilight/mod.rs`

**Interfaces:**
- Consumes: `config::AmbilightConfig`, `protocol::Rgb`.
- Produces:
  - `image::Rect { x0, y0, x1, y1 }` (exclusive ends) with `width()`, `height()`.
  - `image::Image::{new(width, height, Vec<[f32; 3]>), filled(width, height, c), width(), height(), full() -> Rect, pixel(x, y), set(x, y, c), mean(Rect) -> [f32; 3], sample_rect(area: Rect, p: [f64; 2], depth: f64) -> Rect}`; pixels are sRGB-encoded 0..1.
  - `color::Color = [f32; 3]`, `srgb_encode`, `hdr_to_srgb(Color, sdr_white_nits: f32) -> Color`, `f16_to_f32(u16) -> f32`, `luma(Color)`, `to_rgb(Color) -> Rgb`, `Adjust { saturation, brightness, black_threshold }` with `Adjust::from_config(&AmbilightConfig)`, `adjust(Color, &Adjust) -> Color`.

- [ ] **Step 1: Write the failing tests**

Create `tests/color_tests.rs`:

```rust
use obsid_ambilight::ambilight::color::{Adjust, adjust, f16_to_f32, hdr_to_srgb, luma, srgb_encode, to_rgb};
use obsid_ambilight::ambilight::image::{Image, Rect};
use obsid_ambilight::config::AmbilightConfig;
use obsid_ambilight::protocol::Rgb;

const PLAIN: Adjust = Adjust { saturation: 1.0, brightness: 1.0, black_threshold: 0.0 };

fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
}

#[test]
fn sdr_white_in_hdr_becomes_full_white() {
    // 200-nit SDR white is 2.5 in scRGB (1.0 = 80 nits).
    assert_eq!(to_rgb(hdr_to_srgb([2.5; 3], 200.0)), Rgb::new(255, 255, 255));
    assert_eq!(to_rgb(hdr_to_srgb([1.25; 3], 200.0)), Rgb::new(188, 188, 188), "half linear = sRGB 188");
}

#[test]
fn hdr_highlights_keep_their_hue() {
    assert_eq!(to_rgb(hdr_to_srgb([10.0, 5.0, 0.0], 80.0)), Rgb::new(255, 188, 0));
}

#[test]
fn hdr_out_of_gamut_and_garbage_values_are_dropped() {
    assert_eq!(to_rgb(hdr_to_srgb([-0.3, 1.0, f32::NAN], 80.0)), Rgb::new(0, 255, 0));
    assert_eq!(to_rgb(hdr_to_srgb([f32::INFINITY, 0.0, 0.0], 80.0)), Rgb::BLACK);
    assert_eq!(to_rgb(hdr_to_srgb([1.0; 3], 0.0)), Rgb::new(255, 255, 255), "unknown white = 80 nits");
}

#[test]
fn srgb_encoding_matches_reference_points() {
    assert_eq!(srgb_encode(0.0), 0.0);
    assert!((srgb_encode(1.0) - 1.0).abs() < 1e-6);
    assert!((srgb_encode(0.002) - 0.02584).abs() < 1e-5, "linear toe");
}

#[test]
fn half_floats_decode() {
    assert_eq!(f16_to_f32(0x3C00), 1.0);
    assert_eq!(f16_to_f32(0x4000), 2.0);
    assert_eq!(f16_to_f32(0xC000), -2.0);
    assert_eq!(f16_to_f32(0x3800), 0.5);
    assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
    assert_eq!(f16_to_f32(0x7C00), f32::INFINITY);
    assert!(f16_to_f32(0x7E00).is_nan());
}

#[test]
fn black_threshold_switches_dark_leds_off() {
    let a = Adjust { black_threshold: 0.03, ..PLAIN };
    assert_eq!(adjust([0.02, 0.01, 0.0], &a), [0.0; 3]);
    assert!(approx(adjust([0.04, 0.01, 0.0], &a), [0.04, 0.01, 0.0]));
}

#[test]
fn saturation_moves_away_from_grey() {
    let c = [0.6, 0.4, 0.2];
    assert!(approx(adjust(c, &PLAIN), c));
    let more = adjust(c, &Adjust { saturation: 2.0, ..PLAIN });
    assert!(more[0] > c[0] && more[2] < c[2], "{more:?}");
    let grey = [0.5; 3];
    assert!(approx(adjust(grey, &Adjust { saturation: 2.0, ..PLAIN }), grey));
    let l = luma(c);
    let less = adjust(c, &Adjust { saturation: 0.5, ..PLAIN });
    assert!((less[0] - (l + (c[0] - l) * 0.5)).abs() < 1e-6);
}

#[test]
fn brightness_scales_the_result() {
    assert!(approx(adjust([1.0, 0.5, 0.0], &Adjust { brightness: 0.5, ..PLAIN }), [0.5, 0.25, 0.0]));
}

#[test]
fn adjust_reads_percentages_from_config() {
    let a = Adjust::from_config(&AmbilightConfig { brightness: 40, black_threshold: 5, ..AmbilightConfig::defaults(60) });
    assert_eq!(a, Adjust { saturation: 1.2, brightness: 0.4, black_threshold: 0.05 });
}

#[test]
fn to_rgb_clamps_and_rounds() {
    assert_eq!(to_rgb([1.5, -0.2, 0.5]), Rgb::new(255, 0, 128));
}

#[test]
fn mean_averages_the_rectangle() {
    let mut img = Image::filled(4, 2, [0.0; 3]);
    img.set(0, 0, [1.0, 0.0, 0.0]);
    img.set(1, 0, [0.0, 1.0, 0.0]);
    assert_eq!(img.mean(Rect { x0: 0, y0: 0, x1: 2, y1: 1 }), [0.5, 0.5, 0.0]);
    assert_eq!(img.mean(Rect { x0: 3, y0: 1, x1: 9, y1: 9 }), [0.0; 3], "clamped to the image");
    assert_eq!(img.mean(Rect { x0: 2, y0: 0, x1: 2, y1: 1 }), [0.0; 3], "empty = black");
}

#[test]
fn sample_square_stays_inside_the_area() {
    let area = Rect { x0: 0, y0: 10, x1: 160, y1: 80 };
    // depth 0.1 of the shorter side (70) = 7 px.
    let r = Image::sample_rect(area, [0.0, 0.0], 0.1);
    assert_eq!(r, Rect { x0: 0, y0: 10, x1: 7, y1: 17 });
    let r = Image::sample_rect(area, [1.0, 1.0], 0.1);
    assert_eq!(r, Rect { x0: 153, y0: 73, x1: 160, y1: 80 });
    let r = Image::sample_rect(area, [0.5, 0.5], 0.0);
    assert_eq!((r.width(), r.height()), (1, 1), "never smaller than one pixel");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test color_tests`
Expected: compile error — modules `color` and `image` do not exist.

- [ ] **Step 3: Implement `image.rs`**

Create `src/ambilight/image.rs`:

```rust
//! Small frame of sRGB-encoded colours (0..1) and averages over areas of it.

use super::color::Color;

/// Pixel rectangle with exclusive right/bottom edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Rect {
    pub fn width(&self) -> usize {
        self.x1.saturating_sub(self.x0)
    }

    pub fn height(&self) -> usize {
        self.y1.saturating_sub(self.y0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    width: usize,
    height: usize,
    pixels: Vec<Color>,
}

impl Image {
    /// Panics if a side is zero or `pixels` does not hold `width * height` colours.
    pub fn new(width: usize, height: usize, pixels: Vec<Color>) -> Self {
        assert!(width > 0 && height > 0, "empty image");
        assert_eq!(pixels.len(), width * height, "pixel count");
        Self { width, height, pixels }
    }

    pub fn filled(width: usize, height: usize, c: Color) -> Self {
        Self::new(width, height, vec![c; width * height])
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn full(&self) -> Rect {
        Rect { x0: 0, y0: 0, x1: self.width, y1: self.height }
    }

    pub fn pixel(&self, x: usize, y: usize) -> Color {
        self.pixels[y * self.width + x]
    }

    pub fn set(&mut self, x: usize, y: usize, c: Color) {
        self.pixels[y * self.width + x] = c;
    }

    /// Mean colour of `r` clipped to the image; black for an empty area.
    pub fn mean(&self, r: Rect) -> Color {
        let (x1, y1) = (r.x1.min(self.width), r.y1.min(self.height));
        let mut sum = [0.0f32; 3];
        let mut n = 0u32;
        for y in r.y0..y1 {
            for x in r.x0..x1 {
                for (acc, v) in sum.iter_mut().zip(self.pixel(x, y)) {
                    *acc += v;
                }
                n += 1;
            }
        }
        if n == 0 { [0.0; 3] } else { sum.map(|s| s / n as f32) }
    }

    /// Square of side `depth × shorter side of area` (at least 1 px) centred
    /// on `p` (fractions of `area`) and moved inside `area`.
    pub fn sample_rect(area: Rect, p: [f64; 2], depth: f64) -> Rect {
        let (w, h) = (area.width(), area.height());
        if w == 0 || h == 0 {
            return area;
        }
        let short = w.min(h);
        let side = ((depth * short as f64).round() as usize).clamp(1, short);
        let place = |origin: usize, len: usize, frac: f64| {
            let centre = origin as f64 + frac.clamp(0.0, 1.0) * len as f64;
            let start = (centre - side as f64 / 2.0).round().max(origin as f64) as usize;
            start.min(origin + len - side)
        };
        let (x0, y0) = (place(area.x0, w, p[0]), place(area.y0, h, p[1]));
        Rect { x0, y0, x1: x0 + side, y1: y0 + side }
    }
}
```

- [ ] **Step 4: Implement `color.rs`**

Create `src/ambilight/color.rs`:

```rust
//! Colour maths: HDR (scRGB) to SDR, half floats, LED adjustments.

use crate::config::AmbilightConfig;
use crate::protocol::Rgb;

/// RGB, normally sRGB-encoded 0..1.
pub type Color = [f32; 3];

/// Brightness of SDR white in scRGB units (1.0 = 80 nits).
const SCRGB_NITS: f32 = 80.0;

pub fn srgb_encode(linear: f32) -> f32 {
    if linear <= 0.003_130_8 { 12.92 * linear } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 }
}

/// Linear scRGB → sRGB: SDR white maps to 1.0, out-of-gamut negatives and
/// non-finite values to 0, and anything brighter than SDR white is scaled
/// down by its largest channel so the hue survives.
pub fn hdr_to_srgb(c: Color, sdr_white_nits: f32) -> Color {
    let white = if sdr_white_nits > 0.0 { sdr_white_nits } else { SCRGB_NITS };
    let scale = SCRGB_NITS / white;
    let mut v = c.map(|x| if x.is_finite() { (x * scale).max(0.0) } else { 0.0 });
    let m = v[0].max(v[1]).max(v[2]);
    if m > 1.0 {
        v = v.map(|x| x / m);
    }
    v.map(srgb_encode)
}

/// IEEE 754 half precision → `f32` (DXGI FP16 frames).
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1F);
    let frac = f32::from(h & 0x3FF);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 if frac == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// Rec. 709 luma.
pub fn luma(c: Color) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn to_rgb(c: Color) -> Rgb {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb::new(q(c[0]), q(c[1]), q(c[2]))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Adjust {
    pub saturation: f32,
    /// 0..1
    pub brightness: f32,
    /// 0..1; colours whose largest channel is below it become black.
    pub black_threshold: f32,
}

impl Adjust {
    pub fn from_config(a: &AmbilightConfig) -> Self {
        Self {
            saturation: a.saturation as f32,
            brightness: f32::from(a.brightness) / 100.0,
            black_threshold: f32::from(a.black_threshold) / 100.0,
        }
    }
}

/// Threshold, then saturation around the luma, then brightness.
pub fn adjust(c: Color, a: &Adjust) -> Color {
    if c[0].max(c[1]).max(c[2]) < a.black_threshold {
        return [0.0; 3];
    }
    let l = luma(c);
    c.map(|x| (l + (x - l) * a.saturation).clamp(0.0, 1.0) * a.brightness)
}
```

Add to `src/ambilight/mod.rs`: `pub mod color;` and `pub mod image;` (alphabetical order with `layout`).

- [ ] **Step 5: Run the tests**

Run: `cargo test --test color_tests`
Expected: PASS (12 tests).

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/ambilight tests/color_tests.rs
git commit -m "Add small-frame sampling and HDR colour maths"
```

---

### Task 3: Letterbox detection

**Files:**
- Create: `src/ambilight/letterbox.rs`, `tests/letterbox_tests.rs`
- Modify: `src/ambilight/mod.rs`

**Interfaces:**
- Consumes: `image::{Image, Rect}`.
- Produces: `letterbox::{BLACK_LEVEL, MAX_BAR, HOLD, Bars { vertical, horizontal }, Bars::content(&Image) -> Rect, detect(&Image) -> Option<Bars>, Letterbox::{new, current, reset, update(&Image, Instant) -> Bars}}`.

- [ ] **Step 1: Write the failing tests**

Create `tests/letterbox_tests.rs`:

```rust
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::{Image, Rect};
use obsid_ambilight::ambilight::letterbox::{Bars, HOLD, Letterbox, detect};

const W: usize = 160;
const H: usize = 90;

/// Black frame with a coloured box inside the given margins.
fn framed(top: usize, side: usize) -> Image {
    let mut img = Image::filled(W, H, [0.0; 3]);
    for y in top..H - top {
        for x in side..W - side {
            img.set(x, y, [0.5, 0.3, 0.2]);
        }
    }
    img
}

#[test]
fn cinema_bars_are_found() {
    assert_eq!(detect(&framed(11, 0)), Some(Bars { vertical: 11, horizontal: 0 }));
}

#[test]
fn pillarbox_bars_are_found() {
    assert_eq!(detect(&framed(0, 20)), Some(Bars { vertical: 0, horizontal: 20 }));
}

#[test]
fn a_dark_sky_is_not_a_bar() {
    let mut img = Image::filled(W, H, [0.5; 3]);
    for y in 0..30 {
        for x in 0..W {
            img.set(x, y, [0.0; 3]);
        }
    }
    assert_eq!(detect(&img), Some(Bars::default()), "only one side is dark");
}

#[test]
fn bars_wider_than_30_percent_are_ignored() {
    assert_eq!(detect(&framed(30, 0)), Some(Bars::default()));
}

#[test]
fn dim_content_is_not_black() {
    let img = Image::filled(W, H, [0.1, 0.1, 0.1]);
    assert_eq!(detect(&img), Some(Bars::default()));
}

#[test]
fn a_black_frame_says_nothing() {
    assert_eq!(detect(&Image::filled(W, H, [0.0; 3])), None);
}

#[test]
fn new_bars_apply_only_after_holding() {
    let t0 = Instant::now();
    let mut lb = Letterbox::new();
    let img = framed(11, 0);
    assert_eq!(lb.update(&img, t0), Bars::default());
    assert_eq!(lb.update(&img, t0 + HOLD / 2), Bars::default());
    assert_eq!(lb.update(&img, t0 + HOLD), Bars { vertical: 11, horizontal: 0 });
    // A black frame (fade) keeps the bars.
    assert_eq!(lb.update(&Image::filled(W, H, [0.0; 3]), t0 + HOLD * 2), Bars { vertical: 11, horizontal: 0 });
}

#[test]
fn a_flickering_candidate_restarts_the_timer() {
    let t0 = Instant::now();
    let mut lb = Letterbox::new();
    lb.update(&framed(11, 0), t0);
    lb.update(&framed(12, 0), t0 + Duration::from_millis(600));
    assert_eq!(lb.update(&framed(12, 0), t0 + Duration::from_millis(1200)), Bars::default());
    assert_eq!(lb.update(&framed(12, 0), t0 + Duration::from_millis(1600)), Bars { vertical: 12, horizontal: 0 });
}

#[test]
fn content_rect_excludes_the_bars() {
    let img = framed(11, 0);
    assert_eq!(Bars { vertical: 11, horizontal: 4 }.content(&img), Rect { x0: 4, y0: 11, x1: 156, y1: 79 });
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test letterbox_tests`
Expected: compile error — module `letterbox` does not exist.

- [ ] **Step 3: Implement**

Create `src/ambilight/letterbox.rs`:

```rust
//! Letterbox/pillarbox bars in the small frame, applied with hysteresis so a
//! dark scene does not make the sampled area jump around.

use std::time::{Duration, Instant};

use super::color::Color;
use super::image::{Image, Rect};

/// A row/column is black when every pixel's largest channel is below this (sRGB).
pub const BLACK_LEVEL: f32 = 0.06;
/// Bars thicker than this share of the side are not bars.
pub const MAX_BAR: f32 = 0.30;
/// How long new bars must persist before they are used.
pub const HOLD: Duration = Duration::from_secs(1);

/// Bar thickness in small-frame pixels, the same on opposite sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bars {
    /// Top and bottom.
    pub vertical: usize,
    /// Left and right.
    pub horizontal: usize,
}

impl Bars {
    pub fn content(&self, img: &Image) -> Rect {
        Rect {
            x0: self.horizontal,
            y0: self.vertical,
            x1: img.width() - self.horizontal,
            y1: img.height() - self.vertical,
        }
    }
}

fn dark(c: Color) -> bool {
    c[0].max(c[1]).max(c[2]) < BLACK_LEVEL
}

/// Bars of this frame alone; `None` when the whole frame is black.
pub fn detect(img: &Image) -> Option<Bars> {
    let (w, h) = (img.width(), img.height());
    let row_dark = |y: usize| (0..w).all(|x| dark(img.pixel(x, y)));
    let col_dark = |x: usize| (0..h).all(|y| dark(img.pixel(x, y)));
    let top = (0..h).take_while(|&y| row_dark(y)).count();
    if top == h {
        return None;
    }
    let bottom = (0..h).rev().take_while(|&y| row_dark(y)).count();
    let left = (0..w).take_while(|&x| col_dark(x)).count();
    let right = (0..w).rev().take_while(|&x| col_dark(x)).count();
    let limit = |n: usize, side: usize| if n as f32 > side as f32 * MAX_BAR { 0 } else { n };
    Some(Bars { vertical: limit(top.min(bottom), h), horizontal: limit(left.min(right), w) })
}

#[derive(Debug, Default)]
pub struct Letterbox {
    current: Bars,
    candidate: Option<(Bars, Instant)>,
}

impl Letterbox {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> Bars {
        self.current
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn update(&mut self, img: &Image, now: Instant) -> Bars {
        let Some(seen) = detect(img) else { return self.current };
        if seen == self.current {
            self.candidate = None;
            return self.current;
        }
        match self.candidate {
            Some((bars, since)) if bars == seen => {
                if now.duration_since(since) >= HOLD {
                    self.current = seen;
                    self.candidate = None;
                }
            }
            _ => self.candidate = Some((seen, now)),
        }
        self.current
    }
}
```

Add `pub mod letterbox;` to `src/ambilight/mod.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --test letterbox_tests`
Expected: PASS (9 tests).

- [ ] **Step 5: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/ambilight tests/letterbox_tests.rs
git commit -m "Detect letterbox bars with hysteresis"
```

---

### Task 4: Smoothing and the frame pipeline

**Files:**
- Create: `src/ambilight/smooth.rs`, `src/ambilight/pipeline.rs`, `tests/pipeline_tests.rs`
- Modify: `src/ambilight/mod.rs`

**Interfaces:**
- Consumes: Tasks 1–3 (`layout::led_points`, `Image`, `Adjust`, `adjust`, `to_rgb`, `luma`, `Letterbox`, `Bars`), `config::{Config, Segment}`.
- Produces:
  - `smooth::{SCENE_CUT, MAX_TAU, tau(u8) -> Duration, Smoother::{new, reset, step(&[Color], Duration, Instant) -> &[Color]}}`.
  - `pipeline::Settings { led_count, segments, depth, adjust, smoothing, letterbox }` with `Settings::from_config(&Config)`; `pipeline::Pipeline::{new, bars() -> Bars, process(&Image, &Settings, Instant) -> Vec<Rgb>}`.

- [ ] **Step 1: Write the failing tests**

Create `tests/pipeline_tests.rs`:

```rust
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::Image;
use obsid_ambilight::ambilight::letterbox::{Bars, HOLD};
use obsid_ambilight::ambilight::pipeline::{Pipeline, Settings};
use obsid_ambilight::ambilight::smooth::{Smoother, tau};
use obsid_ambilight::config::{Config, Segment};
use obsid_ambilight::protocol::Rgb;

fn settings() -> Settings {
    let mut s = Settings::from_config(&Config::default());
    s.smoothing = 0;
    s
}

#[test]
fn settings_come_from_the_config() {
    let c = Config::default();
    let s = Settings::from_config(&c);
    assert_eq!(s.led_count, 60);
    assert_eq!(s.segments, c.ambilight.segments);
    assert_eq!(s.smoothing, 25);
    assert!(s.letterbox);
}

#[test]
fn a_red_screen_makes_every_led_red() {
    let mut p = Pipeline::new();
    let out = p.process(&Image::filled(160, 90, [1.0, 0.0, 0.0]), &settings(), Instant::now());
    assert_eq!(out, vec![Rgb::new(255, 0, 0); 60]);
}

#[test]
fn each_side_takes_its_own_colour() {
    let mut img = Image::filled(160, 90, [0.0, 0.0, 1.0]);
    for y in 0..90 {
        for x in 80..160 {
            img.set(x, y, [1.0, 0.0, 0.0]);
        }
    }
    let mut s = settings();
    s.led_count = 4;
    s.segments = vec![
        Segment { from: 0, to: 1, start: [0.02, 0.2], end: [0.02, 0.8] },
        Segment { from: 2, to: 3, start: [0.98, 0.2], end: [0.98, 0.8] },
    ];
    let out = Pipeline::new().process(&img, &s, Instant::now());
    assert_eq!(out, vec![Rgb::new(0, 0, 255), Rgb::new(0, 0, 255), Rgb::new(255, 0, 0), Rgb::new(255, 0, 0)]);
}

#[test]
fn leds_outside_segments_stay_dark() {
    let mut s = settings();
    s.segments = vec![Segment { from: 0, to: 9, start: [0.1, 0.1], end: [0.9, 0.1] }];
    let out = Pipeline::new().process(&Image::filled(160, 90, [1.0; 3]), &s, Instant::now());
    assert!(out[..10].iter().all(|c| *c == Rgb::new(255, 255, 255)));
    assert!(out[10..].iter().all(|c| *c == Rgb::BLACK));
}

#[test]
fn letterbox_moves_sampling_inside_the_picture() {
    // 21:9 picture: black bars of 11 rows, red picture between them.
    let mut img = Image::filled(160, 90, [0.0; 3]);
    for y in 11..79 {
        for x in 0..160 {
            img.set(x, y, [1.0, 0.0, 0.0]);
        }
    }
    let mut s = settings();
    s.led_count = 1;
    s.segments = vec![Segment { from: 0, to: 0, start: [0.5, 0.0], end: [0.5, 0.0] }];
    let t0 = Instant::now();
    let mut p = Pipeline::new();
    assert_eq!(p.process(&img, &s, t0), vec![Rgb::BLACK], "bars are not trusted yet");
    assert_eq!(p.process(&img, &s, t0 + HOLD), vec![Rgb::new(255, 0, 0)]);
    assert_eq!(p.bars(), Bars { vertical: 11, horizontal: 0 });

    s.letterbox = false;
    assert_eq!(p.process(&img, &s, t0 + HOLD * 2), vec![Rgb::BLACK]);
    assert_eq!(p.bars(), Bars::default(), "disabled detection reports no bars");
}

#[test]
fn smoothing_eases_small_changes() {
    let t0 = Instant::now();
    let mut sm = Smoother::new();
    assert_eq!(sm.step(&[[0.5; 3]], tau(25), t0), &[[0.5; 3]], "first frame is taken as is");
    let next = sm.step(&[[0.6; 3]], tau(25), t0 + Duration::from_millis(40))[0][0];
    assert!(next > 0.5 && next < 0.6, "{next}");
    assert_eq!(sm.step(&[[0.7; 3]], tau(0), t0 + Duration::from_millis(80)), &[[0.7; 3]], "0 % = no smoothing");
}

#[test]
fn scene_cuts_skip_smoothing() {
    let t0 = Instant::now();
    let mut sm = Smoother::new();
    sm.step(&[[0.0; 3]], tau(100), t0);
    assert_eq!(sm.step(&[[1.0; 3]], tau(100), t0 + Duration::from_millis(40)), &[[1.0; 3]]);
}

#[test]
fn tau_spans_zero_to_half_a_second() {
    assert_eq!(tau(0), Duration::ZERO);
    assert_eq!(tau(100), Duration::from_millis(500));
    assert_eq!(tau(25), Duration::from_millis(125));
}

#[test]
fn a_new_frame_size_resets_letterbox_state() {
    let mut img = Image::filled(160, 90, [0.0; 3]);
    for y in 11..79 {
        for x in 0..160 {
            img.set(x, y, [1.0, 0.0, 0.0]);
        }
    }
    let s = settings();
    let t0 = Instant::now();
    let mut p = Pipeline::new();
    p.process(&img, &s, t0);
    p.process(&img, &s, t0 + HOLD);
    assert_eq!(p.bars().vertical, 11);
    p.process(&Image::filled(120, 68, [0.5; 3]), &s, t0 + HOLD * 2);
    assert_eq!(p.bars(), Bars::default());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test pipeline_tests`
Expected: compile error — modules `pipeline` and `smooth` do not exist.

- [ ] **Step 3: Implement `smooth.rs`**

Create `src/ambilight/smooth.rs`:

```rust
//! Exponential smoothing of LED colours, skipped on scene cuts.

use std::time::{Duration, Instant};

use super::color::{Color, luma};

/// A change of the mean LED luma larger than this is a scene cut.
pub const SCENE_CUT: f32 = 0.25;
/// Time constant at smoothing 100 %.
pub const MAX_TAU: Duration = Duration::from_millis(500);

/// Time constant for the "Smoothing" setting (0..=100).
pub fn tau(smoothing: u8) -> Duration {
    MAX_TAU.mul_f64(f64::from(smoothing.min(100)) / 100.0)
}

#[derive(Debug, Default)]
pub struct Smoother {
    state: Vec<Color>,
    last: Option<Instant>,
}

impl Smoother {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Moves the state toward `target` and returns it.
    pub fn step(&mut self, target: &[Color], tau: Duration, now: Instant) -> &[Color] {
        let jump = match self.last {
            Some(last) if self.state.len() == target.len() && !tau.is_zero() => {
                if (mean_luma(target) - mean_luma(&self.state)).abs() > SCENE_CUT {
                    None
                } else {
                    let dt = now.saturating_duration_since(last).as_secs_f32();
                    Some(1.0 - (-dt / tau.as_secs_f32()).exp())
                }
            }
            _ => None,
        };
        match jump {
            Some(a) => {
                for (s, t) in self.state.iter_mut().zip(target) {
                    for (sv, tv) in s.iter_mut().zip(t) {
                        *sv += (tv - *sv) * a;
                    }
                }
            }
            None => self.state = target.to_vec(),
        }
        self.last = Some(now);
        &self.state
    }
}

fn mean_luma(colors: &[Color]) -> f32 {
    if colors.is_empty() { 0.0 } else { colors.iter().map(|c| luma(*c)).sum::<f32>() / colors.len() as f32 }
}
```

- [ ] **Step 4: Implement `pipeline.rs`**

Create `src/ambilight/pipeline.rs`:

```rust
//! One small frame in, LED colours out: letterbox → sampling → adjustment →
//! smoothing.

use std::time::Instant;

use super::color::{self, Adjust, Color};
use super::image::Image;
use super::layout;
use super::letterbox::{Bars, Letterbox};
use super::smooth::{self, Smoother};
use crate::config::{Config, Segment};
use crate::protocol::Rgb;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub led_count: usize,
    pub segments: Vec<Segment>,
    pub depth: f64,
    pub adjust: Adjust,
    pub smoothing: u8,
    pub letterbox: bool,
}

impl Settings {
    pub fn from_config(cfg: &Config) -> Self {
        let a = &cfg.ambilight;
        Self {
            led_count: cfg.device.led_count,
            segments: a.segments.clone(),
            depth: a.depth,
            adjust: Adjust::from_config(a),
            smoothing: a.smoothing,
            letterbox: a.letterbox,
        }
    }
}

#[derive(Debug, Default)]
pub struct Pipeline {
    letterbox: Letterbox,
    smoother: Smoother,
    size: (usize, usize),
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bars currently left out of sampling.
    pub fn bars(&self) -> Bars {
        self.letterbox.current()
    }

    pub fn process(&mut self, img: &Image, s: &Settings, now: Instant) -> Vec<Rgb> {
        let size = (img.width(), img.height());
        if self.size != size {
            self.size = size;
            self.letterbox.reset();
        }
        let area = if s.letterbox {
            self.letterbox.update(img, now).content(img)
        } else {
            self.letterbox.reset();
            img.full()
        };
        let targets: Vec<Color> = layout::led_points(&s.segments, s.led_count)
            .into_iter()
            .map(|p| match p {
                Some(p) => color::adjust(img.mean(Image::sample_rect(area, p, s.depth)), &s.adjust),
                None => [0.0; 3],
            })
            .collect();
        self.smoother.step(&targets, smooth::tau(s.smoothing), now).iter().map(|c| color::to_rgb(*c)).collect()
    }
}
```

Add `pub mod pipeline;` and `pub mod smooth;` to `src/ambilight/mod.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --test pipeline_tests`
Expected: PASS (9 tests).

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/ambilight tests/pipeline_tests.rs
git commit -m "Add smoothing and the Ambilight frame pipeline"
```

---
### Task 5: Controller — modes, streaming, highlight, exit

**Files:**
- Modify: `src/app/state.rs`, `src/app/commands.rs`, `src/app/controller.rs`, `src/app/worker.rs` (temporary arms), `tests/controller_tests.rs`

**Interfaces:**
- Consumes: Task 1 (`Mode`, `AmbilightConfig`, `Segment`, `layout::{sanitize, trim, HighlightTarget}`).
- Produces:
  - `state::CaptureStatus { monitor, adapter, width: u32, height: u32, hdr, sdr_white_nits: f32, fps: f32, bars: [f32; 2], error: Option<String>, active: bool }` and `state::StreamStatus { streaming, highlight, send_fps: f32, capture: Option<CaptureStatus> }` (both `Default`, serde); `Snapshot.stream: StreamStatus`.
  - `Command::{SetMode { mode }, SetAmbilight { settings: AmbilightConfig }, SetLayout { segments: Vec<Segment> }, Highlight { target: HighlightTarget }, StopHighlight}`.
  - `controller::{HIGHLIGHT_TTL, Desired}`, `Event::{StreamStarted(Result<(), String>), StreamStopped(Ipv4Addr, Result<(), String>), StreamFailed(String)}`, `Action::{StartStream { ip }, StopStream { ip, then: LightChange }}`, `Action::Shutdown { power_off_ip: Option<Ipv4Addr>, stop_stream: Option<(Ipv4Addr, LightChange)> }`.
  - `Controller::{streaming() -> bool, highlight() -> Option<HighlightTarget>, capture_wanted() -> bool}`; `next_deadline()` also covers the highlight expiry.

- [ ] **Step 1: Write the failing tests**

In `tests/controller_tests.rs`:

1. Replace the imports with:

```rust
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::layout::HighlightTarget;
use obsid_ambilight::app::commands::{Command, coalesce};
use obsid_ambilight::app::controller::{
    Action, Controller, Event, HIGHLIGHT_TTL, LightChange, MAX_ERRORS, RETRY_BASE, Resolved, Target,
};
use obsid_ambilight::app::state::{Connection, DeviceInfo, TestState};
use obsid_ambilight::config::{AmbilightConfig, Config, ExitAction, Mode, Segment};
use obsid_ambilight::protocol::Rgb;
use obsid_ambilight::reconnect::Backoff;
```

2. In `exit_powers_off_only_when_asked_and_connected` every `Action::Shutdown { power_off_ip: X }` becomes `Action::Shutdown { power_off_ip: X, stop_stream: None }`.

3. Append:

```rust
// ---------- Ambilight ----------

fn ambilight_cfg() -> Config {
    let mut c = saved_cfg();
    c.app.mode = Mode::Ambilight;
    c
}

fn found(c: &mut Controller, at: Instant) -> Vec<Action> {
    c.handle(Event::Resolved(Ok(Resolved { device: dev("dcda0cd778bc", IP), verified: true })), at)
}

/// Started in Ambilight mode, connected, stream running.
fn streaming() -> (Controller, Instant) {
    let now = Instant::now();
    let mut c = ctrl(ambilight_cfg());
    c.handle(Event::Startup, now);
    assert_eq!(found(&mut c, now), vec![Action::StartStream { ip: IP }]);
    c.handle(Event::StreamStarted(Ok(())), now);
    (c, now)
}

#[test]
fn ambilight_mode_streams_after_connecting() {
    let (c, _) = streaming();
    assert!(c.streaming());
    assert!(c.capture_wanted());
    let s = c.snapshot(true);
    assert!(s.stream.streaming);
    assert!(!s.stream.highlight);
}

#[test]
fn switching_modes_enters_and_leaves_direct_mode() {
    let (mut c, now) = ready();
    let a = c.handle(Event::Command(Command::SetMode { mode: Mode::Ambilight }), now);
    assert_eq!(a, vec![Action::SaveConfig, Action::StartStream { ip: IP }]);
    c.handle(Event::StreamStarted(Ok(())), now);
    assert!(c.capture_wanted());

    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(1, 2, 3)), brightness: None }), now);
    assert_eq!(c.config().app.mode, Mode::Static, "a static colour leaves Ambilight");
    assert_eq!(a, vec![Action::SaveConfig, Action::StopStream { ip: IP, then: full_on(c.config()) }]);
    assert!(!c.capture_wanted());
    c.handle(Event::StreamStopped(IP, Ok(())), now);

    c.handle(Event::Command(Command::SetMode { mode: Mode::Ambilight }), now);
    c.handle(Event::StreamStarted(Ok(())), now);
    let a = c.handle(Event::Command(Command::SetPower { on: false }), now);
    assert_eq!(a, vec![Action::SaveConfig, Action::StopStream { ip: IP, then: LightChange::off() }]);
    let a = c.handle(Event::Command(Command::SetPower { on: true }), now);
    assert_eq!(a, vec![Action::SaveConfig, Action::StartStream { ip: IP }], "power on resumes the chosen mode");
}

#[test]
fn choosing_a_mode_turns_the_strip_on() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetPower { on: false }), now);
    c.handle(Event::Command(Command::SetMode { mode: Mode::Static }), now);
    assert!(c.config().light.power);
}

#[test]
fn ambilight_is_not_started_without_power_on_start() {
    let mut cfg = ambilight_cfg();
    cfg.startup.power_on = false;
    cfg.light.power = false;
    let now = Instant::now();
    let mut c = ctrl(cfg);
    c.handle(Event::Startup, now);
    assert!(found(&mut c, now).is_empty());
    assert!(!c.streaming());
}

#[test]
fn stream_failure_resumes_ambilight_after_reconnect() {
    let (mut c, now) = streaming();
    c.handle(Event::StreamFailed("keep-alive lost".into()), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    assert!(!c.streaming());
    assert!(!c.capture_wanted());
    let at = c.next_deadline().unwrap();
    assert!(matches!(c.handle(Event::Tick, at).as_slice(), [Action::Resolve(_)]));
    assert_eq!(found(&mut c, at), vec![Action::StartStream { ip: IP }]);
}

#[test]
fn a_failed_stream_start_goes_offline_and_retries() {
    let now = Instant::now();
    let mut c = ctrl(ambilight_cfg());
    c.handle(Event::Startup, now);
    found(&mut c, now);
    c.handle(Event::StreamStarted(Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    assert!(!c.streaming());
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    assert_eq!(found(&mut c, at), vec![Action::StartStream { ip: IP }]);
}

#[test]
fn a_failed_stream_stop_retries_the_static_state() {
    let (mut c, now) = streaming();
    c.handle(Event::Command(Command::SetMode { mode: Mode::Static }), now);
    c.handle(Event::StreamStopped(IP, Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Offline);
    let at = c.next_deadline().unwrap();
    c.handle(Event::Tick, at);
    assert_eq!(found(&mut c, at), vec![Action::Apply { ip: IP, change: full_on(c.config()) }]);
}

#[test]
fn highlight_from_static_starts_and_ends_the_stream() {
    let (mut c, now) = ready();
    let a = c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(3) }), now);
    assert_eq!(a, vec![Action::StartStream { ip: IP }]);
    c.handle(Event::StreamStarted(Ok(())), now);
    assert_eq!(c.highlight(), Some(HighlightTarget::Led(3)));
    assert!(!c.capture_wanted(), "static mode needs no capture");
    assert_eq!(c.next_deadline(), Some(now + HIGHLIGHT_TTL));
    assert!(c.snapshot(true).stream.highlight);

    let t1 = now + Duration::from_secs(2);
    let a = c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(4) }), t1);
    assert!(a.is_empty(), "already streaming: {a:?}");
    assert!(c.handle(Event::Tick, now + HIGHLIGHT_TTL).is_empty(), "renewed at t1");
    let a = c.handle(Event::Tick, t1 + HIGHLIGHT_TTL);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: full_on(c.config()) }]);
    assert_eq!(c.highlight(), None);
    assert!(!c.streaming());
}

#[test]
fn highlight_in_ambilight_keeps_the_stream() {
    let (mut c, now) = streaming();
    let a = c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Segment(0) }), now);
    assert!(a.is_empty(), "{a:?}");
    assert!(c.capture_wanted(), "capture keeps running under the overlay");
    assert!(c.handle(Event::Command(Command::StopHighlight), now).is_empty());
    assert!(c.streaming());
}

#[test]
fn changes_during_a_highlight_wait_for_its_end() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(0) }), now);
    c.handle(Event::StreamStarted(Ok(())), now);
    let a = c.handle(Event::Command(Command::SetStatic { color: Some(Rgb::new(7, 7, 7)), brightness: None }), now);
    assert_eq!(a, vec![Action::SaveConfig]);
    let a = c.handle(Event::Command(Command::StopHighlight), now);
    assert_eq!(a, vec![Action::StopStream { ip: IP, then: full_on(c.config()) }]);
    assert_eq!(c.config().light.color, Rgb::new(7, 7, 7));
}

#[test]
fn highlight_needs_a_connected_strip_and_a_real_target() {
    let mut c = ctrl(saved_cfg());
    assert!(c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(0) }), Instant::now()).is_empty());
    assert_eq!(c.snapshot(false).errors.len(), 1);

    let (mut c, now) = ready();
    assert!(c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Led(60) }), now).is_empty());
    assert!(c.handle(Event::Command(Command::Highlight { target: HighlightTarget::Segment(99) }), now).is_empty());
    assert_eq!(c.snapshot(false).errors.len(), 2);
    assert!(!c.streaming());
}

#[test]
fn exit_from_a_stream_leaves_direct_mode() {
    let (mut c, now) = streaming();
    let full = full_on(c.config());
    assert_eq!(
        c.handle(Event::Command(Command::Exit), now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: Some((IP, full)) }]
    );
    let (mut c, now) = streaming();
    c.handle(Event::Command(Command::SetExitAction { action: ExitAction::Off }), now);
    assert_eq!(
        c.handle(Event::Command(Command::Exit), now),
        vec![Action::Shutdown { power_off_ip: None, stop_stream: Some((IP, LightChange::off())) }]
    );
}

#[test]
fn ambilight_settings_are_checked_and_keep_the_segments() {
    let (mut c, now) = ready();
    let before = c.config().clone();
    let bad = AmbilightConfig { fps: 5, ..before.ambilight.clone() };
    assert!(c.handle(Event::Command(Command::SetAmbilight { settings: bad }), now).is_empty());
    assert_eq!(c.config(), &before);
    let good = AmbilightConfig { fps: 30, segments: Vec::new(), ..before.ambilight.clone() };
    assert_eq!(c.handle(Event::Command(Command::SetAmbilight { settings: good }), now), vec![Action::SaveConfig]);
    assert_eq!(c.config().ambilight.fps, 30);
    assert_eq!(c.config().ambilight.segments, before.ambilight.segments, "segments change only via SetLayout");
}

#[test]
fn layout_is_sanitized() {
    let (mut c, now) = ready();
    let segs = vec![
        Segment { from: 0, to: 9, start: [0.0, 0.0], end: [1.0, 0.0] },
        Segment { from: 5, to: 15, start: [0.0, 1.0], end: [1.0, 1.0] },
    ];
    assert_eq!(c.handle(Event::Command(Command::SetLayout { segments: segs.clone() }), now), vec![Action::SaveConfig]);
    assert_eq!(c.config().ambilight.segments, segs[..1]);
    assert_eq!(c.snapshot(false).errors.len(), 1);
}

#[test]
fn a_smaller_led_count_trims_the_layout() {
    let (mut c, now) = ready();
    c.handle(Event::Command(Command::SetLedCount { count: 30 }), now);
    assert!(!c.config().ambilight.segments.is_empty());
    assert!(c.config().ambilight.segments.iter().all(|s| s.hi() < 30));
}

#[test]
fn selecting_another_strip_stops_the_stream_on_the_old_one() {
    let (mut c, now) = streaming();
    let other = Ipv4Addr::new(192, 168, 1, 99);
    c.handle(Event::Command(Command::Discover), now);
    c.handle(Event::Discovered(Ok(vec![dev("dcda0cd778bc", IP), dev("other", other)])), now);
    let a = c.handle(Event::Command(Command::SelectDevice { id: "other".into() }), now);
    assert_eq!(a.first(), Some(&Action::StopStream { ip: IP, then: full_on(c.config()) }));
    assert!(a.contains(&Action::StartStream { ip: other }), "{a:?}");
    c.handle(Event::StreamStopped(IP, Err("timeout".into())), now);
    assert_eq!(c.snapshot(false).connection, Connection::Ready, "the old strip's failure does not matter");
}

#[test]
fn coalesce_keeps_the_last_of_ambilight_runs() {
    let a = AmbilightConfig::defaults(60);
    let b = AmbilightConfig { fps: 30, ..a.clone() };
    let seg = Segment { from: 0, to: 1, start: [0.0, 0.0], end: [1.0, 0.0] };
    let out = coalesce(vec![
        Command::SetAmbilight { settings: a },
        Command::SetAmbilight { settings: b.clone() },
        Command::SetLayout { segments: vec![] },
        Command::SetLayout { segments: vec![seg.clone()] },
        Command::Highlight { target: HighlightTarget::Led(1) },
        Command::Highlight { target: HighlightTarget::Led(2) },
    ]);
    assert_eq!(
        out,
        vec![
            Command::SetAmbilight { settings: b },
            Command::SetLayout { segments: vec![seg] },
            Command::Highlight { target: HighlightTarget::Led(2) },
        ]
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test controller_tests`
Expected: compile errors (`HIGHLIGHT_TTL`, `Command::SetMode`, `Event::StreamStarted`, …).

- [ ] **Step 3: Extend the shared state**

In `src/app/state.rs`, before `Snapshot` add:

```rust
/// Screen capture as seen by the background process.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CaptureStatus {
    /// Output being captured (differs from the setting after a fallback).
    pub monitor: String,
    pub adapter: String,
    pub width: u32,
    pub height: u32,
    pub hdr: bool,
    pub sdr_white_nits: f32,
    /// Frames processed per second.
    pub fps: f32,
    /// Black bars as shares of the height (top = bottom) and width (left = right).
    pub bars: [f32; 2],
    /// Last problem; with `active == false` capture is unavailable.
    pub error: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StreamStatus {
    /// The strip is in direct mode and receives frames.
    pub streaming: bool,
    /// The layout editor overlay is shown instead of the screen.
    pub highlight: bool,
    /// Frames actually sent per second.
    pub send_fps: f32,
    /// `None` while capture is not running.
    pub capture: Option<CaptureStatus>,
}
```

and add `pub stream: StreamStatus,` as the last field of `Snapshot`.

- [ ] **Step 4: Extend the commands**

In `src/app/commands.rs`:

1. Imports:

```rust
use crate::ambilight::layout::HighlightTarget;
use crate::config::{AmbilightConfig, ExitAction, Language, Mode, Segment};
use crate::protocol::Rgb;
```

2. Before `Exit` in `Command` add:

```rust
    /// Static or Ambilight; implies power on.
    SetMode {
        mode: Mode,
    },
    /// Every Ambilight setting except `segments`, which is ignored here (see `SetLayout`).
    SetAmbilight {
        settings: AmbilightConfig,
    },
    SetLayout {
        segments: Vec<Segment>,
    },
    /// Layout editor: light one LED or segment for a few seconds.
    Highlight {
        target: HighlightTarget,
    },
    StopHighlight,
```

3. In `coalesce`, extend the "keep the last one" arm:

```rust
            (Some(last @ Command::SetPower { .. }), cmd @ Command::SetPower { .. })
            | (Some(last @ Command::SetPresets { .. }), cmd @ Command::SetPresets { .. })
            | (Some(last @ Command::SetAmbilight { .. }), cmd @ Command::SetAmbilight { .. })
            | (Some(last @ Command::SetLayout { .. }), cmd @ Command::SetLayout { .. })
            | (Some(last @ Command::Highlight { .. }), cmd @ Command::Highlight { .. }) => *last = cmd,
```

and update its doc comment: "adjacent `SetPower`, `SetPresets`, `SetAmbilight`, `SetLayout` and `Highlight` keep the last one".

- [ ] **Step 5: Rewrite the controller**

Replace `src/app/controller.rs` with:

```rust
//! Pure state machine of the device worker. It never touches the network or
//! the clock: the worker feeds it events with the current time and executes
//! the actions it returns, feeding results back as events.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::commands::Command;
use super::state::{Connection, DeviceInfo, Snapshot, StreamStatus, TestState};
use crate::ambilight::layout::{self, HighlightTarget};
use crate::config::{self, AmbilightConfig, Config, ExitAction, LightConfig, Mode};
use crate::protocol::Rgb;
use crate::protocol::packets::MAX_LEDS;
use crate::reconnect::Backoff;

pub const MAX_ERRORS: usize = 20;
pub const RETRY_BASE: Duration = Duration::from_secs(2);
pub const RETRY_MAX: Duration = Duration::from_secs(300);
/// A layout highlight ends this long after the last `Highlight` command.
pub const HIGHLIGHT_TTL: Duration = Duration::from_secs(3);

/// Commands to send to the strip, in order: power, colour, brightness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightChange {
    pub power: Option<bool>,
    pub color: Option<Rgb>,
    pub brightness: Option<u8>,
}

impl LightChange {
    /// The full static state: on + colour + brightness, or just off.
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

/// What the strip should show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desired {
    /// Static colour or off, sent as plain commands.
    Light(LightChange),
    /// Screen colours streamed in direct mode.
    Ambilight,
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
    /// Result of `Action::StartStream`.
    StreamStarted(Result<(), String>),
    /// Result of `Action::StopStream` on the strip at this address.
    StreamStopped(Ipv4Addr, Result<(), String>),
    /// The running stream lost its session.
    StreamFailed(String),
    /// Time has passed; lets due retries and highlight expiry fire.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Resolve(Target),
    Discover,
    Apply { ip: Ipv4Addr, change: LightChange },
    /// Enter direct mode; the worker sends frames until `StopStream`.
    StartStream { ip: Ipv4Addr },
    /// Leave direct mode, then send `then`.
    StopStream { ip: Ipv4Addr, then: LightChange },
    Test { ip: Ipv4Addr },
    SaveConfig,
    OpenSettings,
    /// `stop_stream` leaves direct mode with short timeouts and applies its
    /// change; `power_off_ip` switches off a strip that is not streaming.
    Shutdown { power_off_ip: Option<Ipv4Addr>, stop_stream: Option<(Ipv4Addr, LightChange)> },
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
    /// State to reach once the strip is reachable.
    pending: Option<Desired>,
    retry_at: Option<Instant>,
    retry_in_secs: Option<u64>,
    backoff: Backoff,
    test: TestState,
    errors: VecDeque<String>,
    config_warnings: Vec<String>,
    /// Direct mode is on (or `StartStream` was just issued).
    streaming: bool,
    /// Layout editor overlay and when it expires.
    highlight: Option<(HighlightTarget, Instant)>,
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
            streaming: false,
            highlight: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// When the controller next needs a `Tick`.
    pub fn next_deadline(&self) -> Option<Instant> {
        [self.retry_at, self.highlight.map(|(_, until)| until)].into_iter().flatten().min()
    }

    pub fn streaming(&self) -> bool {
        self.streaming
    }

    /// The layout overlay to send instead of screen colours.
    pub fn highlight(&self) -> Option<HighlightTarget> {
        self.highlight.map(|(t, _)| t)
    }

    /// Screen capture is needed: streaming in Ambilight mode.
    pub fn capture_wanted(&self) -> bool {
        self.streaming && self.connection == Connection::Ready && self.desired() == Desired::Ambilight
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
            stream: StreamStatus {
                streaming: self.streaming,
                highlight: self.highlight.is_some(),
                ..StreamStatus::default()
            },
        }
    }

    pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match event {
            Event::Startup => self.startup(),
            Event::Command(cmd) => self.command(cmd, now),
            Event::Tick => self.tick(now),
            Event::Resolved(Ok(r)) => self.on_resolved(r),
            Event::Resolved(Err(e)) => {
                self.record_error(e);
                self.go_offline(now);
                Vec::new()
            }
            Event::Discovered(result) => self.on_discovered(result),
            Event::Applied(Ok(())) | Event::StreamStarted(Ok(())) | Event::StreamStopped(_, Ok(())) => {
                self.backoff.reset();
                Vec::new()
            }
            Event::Applied(Err(e)) => self.fail(e, now),
            Event::StreamStarted(Err(e)) | Event::StreamFailed(e) => {
                self.streaming = false;
                self.fail(e, now)
            }
            Event::StreamStopped(ip, Err(e)) => {
                if self.ready_ip() == Some(ip) {
                    self.fail(e, now)
                } else {
                    // The user has already switched to another strip.
                    self.record_error(e);
                    Vec::new()
                }
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

    /// A command to the strip failed: keep the user's state and go offline.
    fn fail(&mut self, e: String, now: Instant) -> Vec<Action> {
        self.record_error(e);
        self.pending = Some(self.desired());
        self.go_offline(now);
        Vec::new()
    }

    fn tick(&mut self, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.highlight.is_some_and(|(_, until)| now >= until) {
            actions.extend(self.end_highlight());
        }
        if self.connection == Connection::Offline && self.retry_at.is_some_and(|at| now >= at) {
            actions.extend(self.begin_resolve());
        }
        actions
    }

    fn startup(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.cfg.startup.power_on {
            if !self.cfg.light.power {
                self.cfg.light.power = true;
                actions.push(Action::SaveConfig);
            }
            self.pending = Some(self.desired());
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
        self.streaming = false;
        self.highlight = None;
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
        match self.pending.take() {
            // Being found is not enough when commands keep failing: the
            // backoff resets only after the pending state is reached.
            Some(desired) => actions.extend(self.drive(desired)),
            None => self.backoff.reset(),
        }
        actions
    }

    fn become_ready(&mut self, device: DeviceInfo, verified: bool) {
        self.connection = Connection::Ready;
        self.device = Some(device);
        self.verified = verified;
        self.retry_at = None;
        self.retry_in_secs = None;
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
        let mut actions = self.leave_stream();
        let d = &mut self.cfg.device;
        d.id = dev.id.clone();
        d.name = dev.name.clone();
        d.last_ip = Some(dev.ip);
        d.manual_ip = None;
        self.become_ready(dev, true);
        // A strip the user just picked starts with a fresh retry schedule.
        self.backoff.reset();
        actions.push(Action::SaveConfig);
        let desired = match self.pending.take() {
            Some(d) => Some(d),
            None => self.cfg.light.power.then(|| self.desired()),
        };
        if let Some(d) = desired {
            actions.extend(self.drive(d));
        }
        actions
    }

    /// The user's chosen state of the strip.
    fn desired(&self) -> Desired {
        if self.cfg.light.power && self.cfg.app.mode == Mode::Ambilight {
            Desired::Ambilight
        } else {
            Desired::Light(LightChange::full(&self.cfg.light))
        }
    }

    fn ready_ip(&self) -> Option<Ipv4Addr> {
        if self.connection == Connection::Ready { self.device.as_ref().map(|d| d.ip) } else { None }
    }

    /// Moves the strip to `desired` now when ready, otherwise keeps it for the
    /// next connection.
    fn drive(&mut self, desired: Desired) -> Vec<Action> {
        let Some(ip) = self.ready_ip() else {
            self.pending = Some(desired);
            return Vec::new();
        };
        // The editor overlay keeps the stream; the user's state follows when it ends.
        if self.highlight.is_some() {
            return Vec::new();
        }
        match (desired, self.streaming) {
            (Desired::Ambilight, true) => Vec::new(),
            (Desired::Ambilight, false) => {
                self.streaming = true;
                vec![Action::StartStream { ip }]
            }
            (Desired::Light(change), true) => {
                self.streaming = false;
                vec![Action::StopStream { ip, then: change }]
            }
            (Desired::Light(change), false) => vec![Action::Apply { ip, change }],
        }
    }

    /// Leaves direct mode on the current strip before switching to another
    /// one; the old strip keeps the static colour.
    fn leave_stream(&mut self) -> Vec<Action> {
        self.highlight = None;
        match (self.streaming, self.ready_ip()) {
            (true, Some(ip)) => {
                self.streaming = false;
                vec![Action::StopStream { ip, then: LightChange::full(&self.cfg.light) }]
            }
            _ => Vec::new(),
        }
    }

    fn save_and_drive(&mut self) -> Vec<Action> {
        let mut actions = vec![Action::SaveConfig];
        let desired = self.desired();
        actions.extend(self.drive(desired));
        actions
    }

    fn start_highlight(&mut self, target: HighlightTarget, now: Instant) -> Vec<Action> {
        let Some(ip) = self.ready_ip() else {
            self.record_error("cannot highlight LEDs: the strip is not connected".to_owned());
            return Vec::new();
        };
        let valid = match target {
            HighlightTarget::Led(i) => i < self.cfg.device.led_count,
            HighlightTarget::Segment(k) => k < self.cfg.ambilight.segments.len(),
        };
        if !valid {
            self.record_error(format!("nothing to highlight at {target:?}"));
            return Vec::new();
        }
        self.highlight = Some((target, now + HIGHLIGHT_TTL));
        if self.streaming {
            Vec::new()
        } else {
            self.streaming = true;
            vec![Action::StartStream { ip }]
        }
    }

    fn end_highlight(&mut self) -> Vec<Action> {
        if self.highlight.take().is_none() {
            return Vec::new();
        }
        let desired = self.desired();
        self.drive(desired)
    }

    fn command(&mut self, cmd: Command, now: Instant) -> Vec<Action> {
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
                self.cfg.app.mode = Mode::Static;
                self.save_and_drive()
            }
            Command::SetPower { on } => {
                self.cfg.light.power = on;
                self.save_and_drive()
            }
            Command::SetMode { mode } => {
                self.cfg.app.mode = mode;
                self.cfg.light.power = true;
                self.save_and_drive()
            }
            Command::SetAmbilight { settings } => {
                if let Err(e) = settings.check() {
                    self.record_error(e);
                    return Vec::new();
                }
                let segments = std::mem::take(&mut self.cfg.ambilight.segments);
                self.cfg.ambilight = AmbilightConfig { segments, ..settings };
                vec![Action::SaveConfig]
            }
            Command::SetLayout { segments } => {
                let (clean, warnings) = layout::sanitize(segments, self.cfg.device.led_count);
                for w in warnings {
                    self.record_error(w);
                }
                self.cfg.ambilight.segments = clean;
                vec![Action::SaveConfig]
            }
            Command::Highlight { target } => self.start_highlight(target, now),
            Command::StopHighlight => self.end_highlight(),
            Command::SelectDevice { id } => match self.discovered.iter().find(|d| d.id == id).cloned() {
                Some(dev) => self.select(dev),
                None => {
                    self.record_error(format!("strip {id} is not in the discovery list"));
                    Vec::new()
                }
            },
            Command::SetManualIp { ip } => {
                let mut actions = self.leave_stream();
                self.cfg.device.manual_ip = ip;
                self.device = None;
                if self.cfg.light.power {
                    self.pending = Some(self.desired());
                }
                actions.push(Action::SaveConfig);
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
                self.cfg.ambilight.segments = layout::trim(&self.cfg.ambilight.segments, count);
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
                let ip = self.ready_ip();
                let stop_stream = match (self.streaming, ip) {
                    (true, Some(ip)) => Some((
                        ip,
                        match self.cfg.app.exit_action {
                            // A live stream cannot outlive the app: keep the static colour.
                            ExitAction::Keep => LightChange::full(&self.cfg.light),
                            ExitAction::Off => LightChange::off(),
                        },
                    )),
                    _ => None,
                };
                let power_off_ip = match (stop_stream.is_none(), self.cfg.app.exit_action, ip) {
                    (true, ExitAction::Off, Some(ip)) => Some(ip),
                    _ => None,
                };
                vec![Action::Shutdown { power_off_ip, stop_stream }]
            }
        }
    }
}
```

- [ ] **Step 6: Keep the worker compiling**

`src/app/worker.rs` must compile with the new actions until Task 8 implements streaming. In `process`, change the `Shutdown` arm pattern to `Action::Shutdown { power_off_ip, stop_stream: _ }` and add temporary arms:

```rust
                Action::StartStream { .. } => {
                    queue.push_back(Event::StreamStarted(Err("streaming is not available yet".into())))
                }
                Action::StopStream { ip, .. } => queue.push_back(Event::StreamStopped(ip, Ok(()))),
```

These arms are replaced in Task 8.

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: PASS. Fix any other test or UI code that builds `Snapshot` or `Action::Shutdown` literally by adding the new fields.

- [ ] **Step 8: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/app tests/controller_tests.rs
git commit -m "Add Ambilight mode, streaming and highlight to the controller"
```

---

### Task 6: DXGI screen capture

**Files:**
- Create: `src/capture/mod.rs`, `src/capture/outputs.rs`, `src/capture/display.rs`, `src/capture/dxgi.rs`, `tests/capture_tests.rs`
- Modify: `Cargo.toml`, `src/lib.rs`

**Interfaces:**
- Consumes: `ambilight::image::Image`, `ambilight::color::{f16_to_f32, hdr_to_srgb}`.
- Produces:
  - `capture::GrabberInfo { monitor, adapter, width: u32, height: u32, hdr, sdr_white_nits: f32, fallback: bool }`.
  - `capture::Grabber` trait: `fn info(&self) -> &GrabberInfo; fn grab(&mut self) -> Result<Option<Image>, String>` (`Ok(None)` = unchanged screen; `Err` = reopen).
  - `capture::GrabberFactory = Arc<dyn Fn(&str) -> Result<Box<dyn Grabber>, String> + Send + Sync>`.
  - `capture::outputs::{MonitorInfo { name, adapter, width: u32, height: u32, primary }, list() -> Result<Vec<MonitorInfo>, String>}`.
  - `capture::display::sdr_white_nits(&str) -> Option<f32>`.
  - `capture::dxgi::{MAX_SMALL_WIDTH, mip_level(u32) -> u32, DxgiGrabber::open(&str) -> Result<DxgiGrabber, String>}`.

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, in `[target."cfg(windows)".dependencies]`, add `"Win32_Devices_Display",` to the `windows-sys` feature list (keep it sorted) and add:

```toml
windows = { version = "0.62", features = [
    "Win32_Foundation",
    "Win32_Graphics_Direct3D",
    "Win32_Graphics_Direct3D11",
    "Win32_Graphics_Dxgi",
    "Win32_Graphics_Dxgi_Common",
    "Win32_Graphics_Gdi",
] }
```

Run: `cargo build`
Expected: builds; `Cargo.lock` keeps `windows 0.62.2` (already present through `accesskit_windows`).

- [ ] **Step 2: Write the tests**

Create `tests/capture_tests.rs`:

```rust
use std::time::{Duration, Instant};

use obsid_ambilight::capture::Grabber;
use obsid_ambilight::capture::dxgi::{DxgiGrabber, MAX_SMALL_WIDTH, mip_level};
use obsid_ambilight::capture::outputs;

#[test]
fn mip_level_reads_back_at_most_256_pixels_wide() {
    assert_eq!(mip_level(256), 0);
    assert_eq!(mip_level(1920), 3);
    assert_eq!(mip_level(2560), 4);
    assert_eq!(mip_level(3840), 4);
    assert_eq!(mip_level(7680), 5);
}

#[test]
#[ignore = "needs a real desktop and GPU"]
fn lists_the_monitors() {
    let monitors = outputs::list().unwrap();
    assert!(!monitors.is_empty());
    assert_eq!(monitors.iter().filter(|m| m.primary).count(), 1, "{monitors:?}");
}

#[test]
#[ignore = "needs a real desktop and GPU"]
fn captures_the_primary_monitor() {
    let mut g = DxgiGrabber::open("").unwrap();
    assert!(g.info().width >= 640, "{:?}", g.info());
    assert!(!g.info().fallback);
    let deadline = Instant::now() + Duration::from_secs(3);
    let img = loop {
        if let Some(img) = g.grab().unwrap() {
            break img;
        }
        assert!(Instant::now() < deadline, "no frame within 3 s");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(img.width() as u32 <= MAX_SMALL_WIDTH && img.height() > 0);
}

#[test]
#[ignore = "needs a real desktop and GPU"]
fn an_unknown_monitor_falls_back_to_the_primary() {
    let g = DxgiGrabber::open(r"\\.\NO_SUCH_DISPLAY").unwrap();
    assert!(g.info().fallback);
}
```

Run: `cargo test --test capture_tests`
Expected: compile error — `obsid_ambilight::capture` does not exist.

- [ ] **Step 3: Module root**

Create `src/capture/mod.rs`:

```rust
//! Screen capture for Ambilight. `dxgi`, `outputs` and `display` talk to
//! Windows; consumers only see the [`Grabber`] trait.

pub mod display;
pub mod dxgi;
pub mod outputs;

use std::sync::Arc;

use crate::ambilight::image::Image;

/// What a grabber captures.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GrabberInfo {
    /// DXGI output name, e.g. `\\.\DISPLAY1`.
    pub monitor: String,
    pub adapter: String,
    pub width: u32,
    pub height: u32,
    pub hdr: bool,
    pub sdr_white_nits: f32,
    /// The requested monitor was not found; the primary one is captured.
    pub fallback: bool,
}

/// Source of small frames. Created and used on the capture thread only.
pub trait Grabber {
    fn info(&self) -> &GrabberInfo;
    /// The newest frame if the screen changed since the last call, `Ok(None)`
    /// otherwise. `Err` means the grabber is unusable and must be reopened.
    fn grab(&mut self) -> Result<Option<Image>, String>;
}

/// Opens a grabber for a monitor name (empty = primary).
pub type GrabberFactory = Arc<dyn Fn(&str) -> Result<Box<dyn Grabber>, String> + Send + Sync>;

/// UTF-16 buffer with a trailing NUL → `String`.
pub(crate) fn from_wide(w: &[u16]) -> String {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}
```

In `src/lib.rs` add `pub mod capture;` (after `pub mod app;`).

- [ ] **Step 4: Monitor enumeration**

Create `src/capture/outputs.rs`:

```rust
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
```

- [ ] **Step 5: SDR white level**

Create `src/capture/display.rs`:

```rust
//! SDR white level of a display (Windows "SDR content brightness"), used to
//! map HDR frames back to SDR.

use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS,
    QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;

use super::from_wide;

/// Nits of SDR white on the display with this GDI name; `None` when unknown.
pub fn sdr_white_nits(gdi_name: &str) -> Option<f32> {
    let (mut np, mut nm) = (0u32, 0u32);
    // SAFETY: buffers are sized by GetDisplayConfigBufferSizes and zeroed;
    // every request header carries its own size.
    unsafe {
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) != ERROR_SUCCESS {
            return None;
        }
        let mut paths = vec![std::mem::zeroed::<DISPLAYCONFIG_PATH_INFO>(); np as usize];
        let mut modes = vec![std::mem::zeroed::<DISPLAYCONFIG_MODE_INFO>(); nm as usize];
        let status = QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            std::ptr::null_mut(),
        );
        if status != ERROR_SUCCESS {
            return None;
        }
        paths.truncate(np as usize);
        for p in &paths {
            let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = std::mem::zeroed();
            source.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: p.sourceInfo.adapterId,
                id: p.sourceInfo.id,
            };
            if DisplayConfigGetDeviceInfo(&mut source.header) != 0 || from_wide(&source.viewGdiDeviceName) != gdi_name {
                continue;
            }
            let mut white: DISPLAYCONFIG_SDR_WHITE_LEVEL = std::mem::zeroed();
            white.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                size: size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                adapterId: p.targetInfo.adapterId,
                id: p.targetInfo.id,
            };
            if DisplayConfigGetDeviceInfo(&mut white.header) != 0 {
                return None;
            }
            // 1000 = 80 nits.
            return Some(white.SDRWhiteLevel as f32 / 1000.0 * 80.0);
        }
        None
    }
}
```

- [ ] **Step 6: The grabber**

Create `src/capture/dxgi.rs`:

```rust
//! DXGI Desktop Duplication on the monitor's own adapter. Frames are scaled
//! down on the GPU with mipmaps; only an image at most [`MAX_SMALL_WIDTH`]
//! pixels wide is read back.

use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_RESOURCE_MISC_GENERATE_MIPS, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication,
    IDXGIResource,
};
use windows::core::{Error, Interface};

use super::outputs::{self, FoundOutput};
use super::{Grabber, GrabberInfo, display};
use crate::ambilight::color;
use crate::ambilight::image::Image;

/// Widest image read back from the GPU.
pub const MAX_SMALL_WIDTH: u32 = 256;
/// SDR white when the display does not report it (scRGB 1.0).
const DEFAULT_WHITE_NITS: f32 = 80.0;

/// Mip level whose width first fits in [`MAX_SMALL_WIDTH`].
pub fn mip_level(width: u32) -> u32 {
    let mut level = 0;
    while (width >> level) > MAX_SMALL_WIDTH {
        level += 1;
    }
    level
}

pub struct DxgiGrabber {
    info: GrabberInfo,
    context: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    mips: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    staging: ID3D11Texture2D,
    level: u32,
    small: (u32, u32),
    _device: ID3D11Device,
}

fn texture(device: &ID3D11Device, desc: &D3D11_TEXTURE2D_DESC) -> windows::core::Result<ID3D11Texture2D> {
    let mut tex = None;
    // SAFETY: valid descriptor, no initial data.
    unsafe { device.CreateTexture2D(desc, None, Some(&mut tex))? };
    tex.ok_or_else(|| Error::from(E_FAIL))
}

fn desc(width: u32, height: u32, mips: u32, format: DXGI_FORMAT) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: mips,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: 0,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    }
}

impl DxgiGrabber {
    /// Opens the monitor named `monitor` (empty = primary); an unknown name
    /// falls back to the primary monitor with `info().fallback` set.
    pub fn open(monitor: &str) -> Result<Self, String> {
        let all = outputs::enumerate().map_err(|e| format!("cannot list monitors: {}", e.message()))?;
        let wanted = all.iter().position(|o| !monitor.is_empty() && o.name == monitor);
        let fallback = !monitor.is_empty() && wanted.is_none();
        let index = wanted
            .or_else(|| all.iter().position(FoundOutput::is_primary))
            .or(if all.is_empty() { None } else { Some(0) })
            .ok_or("no monitor is attached to the desktop")?;
        let found = &all[index];
        Self::create(found, fallback).map_err(|e| format!("cannot capture {}: {}", found.name, e.message()))
    }

    fn create(found: &FoundOutput, fallback: bool) -> windows::core::Result<Self> {
        let (mut device, mut context) = (None, None);
        // SAFETY: the adapter outlives the call; out-pointers are valid.
        unsafe {
            D3D11CreateDevice(
                &found.adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
        }
        let device: ID3D11Device = device.ok_or_else(|| Error::from(E_FAIL))?;
        let context: ID3D11DeviceContext = context.ok_or_else(|| Error::from(E_FAIL))?;
        // SAFETY: COM calls on interfaces owned here.
        let dup = unsafe {
            match found.output.cast::<IDXGIOutput5>() {
                Ok(o5) => o5.DuplicateOutput1(&device, 0, &[DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_B8G8R8A8_UNORM])?,
                // Windows 10 before 1703: SDR only.
                Err(_) => found.output.cast::<IDXGIOutput1>()?.DuplicateOutput(&device)?,
            }
        };
        // SAFETY: plain getter.
        let mode = unsafe { dup.GetDesc() }.ModeDesc;
        let (w, h, format) = (mode.Width, mode.Height, mode.Format);
        let hdr = format == DXGI_FORMAT_R16G16B16A16_FLOAT;
        let level = mip_level(w);
        let small = ((w >> level).max(1), (h >> level).max(1));

        let mips = texture(
            &device,
            &D3D11_TEXTURE2D_DESC {
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
                ..desc(w, h, level + 1, format)
            },
        )?;
        let mut srv = None;
        // SAFETY: `mips` is a shader resource of this device.
        unsafe { device.CreateShaderResourceView(&mips, None, Some(&mut srv))? };
        let srv = srv.ok_or_else(|| Error::from(E_FAIL))?;
        let staging = texture(
            &device,
            &D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                ..desc(small.0, small.1, 1, format)
            },
        )?;
        let sdr_white_nits =
            if hdr { display::sdr_white_nits(&found.name).unwrap_or(DEFAULT_WHITE_NITS) } else { DEFAULT_WHITE_NITS };
        let info = GrabberInfo {
            monitor: found.name.clone(),
            adapter: found.adapter_name.clone(),
            width: w,
            height: h,
            hdr,
            sdr_white_nits,
            fallback,
        };
        Ok(Self { info, context, dup, mips, srv, staging, level, small, _device: device })
    }

    /// Copies the acquired frame into the mip chain and reads the small level back.
    fn read(
        &self,
        frame: &DXGI_OUTDUPL_FRAME_INFO,
        resource: Option<IDXGIResource>,
    ) -> windows::core::Result<Option<Image>> {
        // Only the mouse pointer moved: the desktop image is unchanged.
        if frame.LastPresentTime == 0 {
            return Ok(None);
        }
        let Some(resource) = resource else { return Ok(None) };
        let desktop: ID3D11Texture2D = resource.cast()?;
        let (w, h) = (self.small.0 as usize, self.small.1 as usize);
        // SAFETY: all resources belong to this device; the mapped pointer is
        // read only within RowPitch × h bytes and unmapped before returning.
        unsafe {
            self.context.CopySubresourceRegion(&self.mips, 0, 0, 0, 0, &desktop, 0, None);
            self.context.GenerateMips(&self.srv);
            self.context.CopySubresourceRegion(&self.staging, 0, 0, 0, 0, &self.mips, self.level, None);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let base = mapped.pData as *const u8;
            let bytes = if self.info.hdr { 8 } else { 4 };
            let mut pixels = Vec::with_capacity(w * h);
            for y in 0..h {
                let row = std::slice::from_raw_parts(base.add(y * mapped.RowPitch as usize), w * bytes);
                if self.info.hdr {
                    pixels.extend(row.chunks_exact(8).map(|p| {
                        let ch = |i: usize| color::f16_to_f32(u16::from_le_bytes([p[i], p[i + 1]]));
                        color::hdr_to_srgb([ch(0), ch(2), ch(4)], self.info.sdr_white_nits)
                    }));
                } else {
                    pixels.extend(
                        row.chunks_exact(4)
                            .map(|p| [f32::from(p[2]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[0]) / 255.0]),
                    );
                }
            }
            self.context.Unmap(&self.staging, 0);
            Ok(Some(Image::new(w, h, pixels)))
        }
    }
}

impl Grabber for DxgiGrabber {
    fn info(&self) -> &GrabberInfo {
        &self.info
    }

    fn grab(&mut self) -> Result<Option<Image>, String> {
        let mut frame = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: out-pointers are valid for the call.
        match unsafe { self.dup.AcquireNextFrame(0, &mut frame, &mut resource) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            // ACCESS_LOST (mode/HDR change, full screen switch), ACCESS_DENIED
            // (secure desktop) and everything else: reopen.
            Err(e) => return Err(format!("capture lost: {}", e.message())),
        }
        let result = self.read(&frame, resource);
        // Every acquired frame must be released, whatever happened above.
        // SAFETY: a frame is currently acquired.
        let _ = unsafe { self.dup.ReleaseFrame() };
        result.map_err(|e| format!("capture failed: {}", e.message()))
    }
}
```

Note for the implementer: if a `windows` 0.62 signature differs from this code (for example `HMODULE::default()` vs `None`, or `Error::from(E_FAIL)` vs `E_FAIL.into()`), follow the compiler and the crate source in `~/.cargo/registry/src/*/windows-0.62.2/src/Windows/Win32/Graphics/`; keep the behaviour unchanged.

- [ ] **Step 7: Run the tests, including the GPU ones**

Run: `cargo test --test capture_tests`
Expected: PASS for `mip_level_reads_back_at_most_256_pixels_wide`, three ignored.

Run: `cargo test --test capture_tests -- --ignored`
Expected: PASS on the development PC (two monitors; the primary is 2560×1440 on a GTX 1050 Ti).

- [ ] **Step 8: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add Cargo.toml Cargo.lock src/capture src/lib.rs tests/capture_tests.rs
git commit -m "Capture the screen with DXGI Desktop Duplication"
```

---

### Task 7: Capture thread

**Files:**
- Create: `src/capture/thread.rs`, `src/ambilight/rate.rs`, `tests/capture_thread_tests.rs`
- Modify: `src/capture/mod.rs`, `src/ambilight/mod.rs`

**Interfaces:**
- Consumes: Tasks 4 and 6 (`Pipeline`, `pipeline::Settings`, `Grabber`, `GrabberFactory`, `GrabberInfo`), `state::CaptureStatus`, `config::Config`.
- Produces:
  - `ambilight::rate::RateMeter::{new, tick(Instant), rate(Instant) -> f32}` with `RateMeter::WINDOW` (5 s).
  - `capture::thread::{REOPEN_DELAY, MAX_PANICS, CaptureSettings { monitor, fps, pipeline }, CaptureSettings::from_config(&Config), Capture::{start(GrabberFactory, CaptureSettings) -> Capture, update(&self, CaptureSettings), take_frame(&self) -> Option<Vec<Rgb>>, status(&self) -> CaptureStatus}}`; dropping a `Capture` stops and joins its thread.

- [ ] **Step 1: Write the failing tests**

Create `tests/capture_thread_tests.rs`:

```rust
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::Image;
use obsid_ambilight::ambilight::rate::RateMeter;
use obsid_ambilight::app::state::CaptureStatus;
use obsid_ambilight::capture::thread::{Capture, CaptureSettings};
use obsid_ambilight::capture::{Grabber, GrabberFactory, GrabberInfo};
use obsid_ambilight::config::Config;
use obsid_ambilight::protocol::Rgb;

#[derive(Default)]
struct Probe {
    opened: Mutex<Vec<String>>,
    alive: AtomicUsize,
    fail_next_grab: AtomicBool,
    fail_opens: AtomicUsize,
    color: Mutex<[f32; 3]>,
}

struct FakeGrabber {
    info: GrabberInfo,
    probe: Arc<Probe>,
}

impl Grabber for FakeGrabber {
    fn info(&self) -> &GrabberInfo {
        &self.info
    }
    fn grab(&mut self) -> Result<Option<Image>, String> {
        if self.probe.fail_next_grab.swap(false, Ordering::SeqCst) {
            return Err("capture lost: test".into());
        }
        Ok(Some(Image::filled(32, 18, *self.probe.color.lock().unwrap())))
    }
}

impl Drop for FakeGrabber {
    fn drop(&mut self) {
        self.probe.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

fn factory(probe: &Arc<Probe>) -> GrabberFactory {
    let probe = Arc::clone(probe);
    Arc::new(move |monitor: &str| {
        probe.opened.lock().unwrap().push(monitor.to_owned());
        if probe.fail_opens.load(Ordering::SeqCst) > 0 {
            probe.fail_opens.fetch_sub(1, Ordering::SeqCst);
            return Err("no monitor".into());
        }
        probe.alive.fetch_add(1, Ordering::SeqCst);
        let info = GrabberInfo {
            monitor: if monitor.is_empty() { r"\\.\DISPLAY1".into() } else { monitor.to_owned() },
            adapter: "Fake GPU".into(),
            width: 2560,
            height: 1440,
            hdr: true,
            sdr_white_nits: 200.0,
            fallback: monitor == "missing",
        };
        Ok(Box::new(FakeGrabber { info, probe: Arc::clone(&probe) }) as Box<dyn Grabber>)
    })
}

fn settings() -> CaptureSettings {
    let mut s = CaptureSettings::from_config(&Config::default());
    s.fps = 30;
    s.pipeline.smoothing = 0;
    s
}

fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(x) = f() {
            return x;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn red_probe() -> Arc<Probe> {
    let p = Arc::new(Probe::default());
    *p.color.lock().unwrap() = [1.0, 0.0, 0.0];
    p
}

#[test]
fn produces_led_colours_from_grabbed_frames() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    let frame = wait("frame", || c.take_frame());
    assert_eq!(frame, vec![Rgb::new(255, 0, 0); 60]);
    let s: CaptureStatus = wait("status", || Some(c.status()).filter(|s| s.active && s.fps > 0.0));
    assert_eq!((s.monitor.as_str(), s.adapter.as_str(), s.width, s.height), (r"\\.\DISPLAY1", "Fake GPU", 2560, 1440));
    assert!(s.hdr);
    assert_eq!(s.sdr_white_nits, 200.0);
    assert_eq!(s.error, None);
}

#[test]
fn follows_settings_updates() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("first frame", || c.take_frame());
    let mut s = settings();
    s.pipeline.led_count = 10;
    s.pipeline.segments.retain(|seg| seg.hi() < 10);
    c.update(s);
    wait("10-LED frame", || c.take_frame().filter(|f| f.len() == 10));
}

#[test]
fn reopens_after_a_lost_grabber() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("first frame", || c.take_frame());
    probe.fail_next_grab.store(true, Ordering::SeqCst);
    wait("reopen", || (probe.opened.lock().unwrap().len() == 2).then_some(()));
    wait("frames again", || c.take_frame());
    assert_eq!(probe.alive.load(Ordering::SeqCst), 1, "the lost grabber was dropped");
}

#[test]
fn factory_failure_is_reported_and_retried() {
    let probe = red_probe();
    probe.fail_opens.store(1, Ordering::SeqCst);
    let c = Capture::start(factory(&probe), settings());
    let s = wait("error", || Some(c.status()).filter(|s| s.error.is_some()));
    assert!(!s.active);
    wait("recovery", || Some(c.status()).filter(|s| s.active && s.error.is_none()));
    wait("frame", || c.take_frame());
}

#[test]
fn changing_the_monitor_reopens_the_grabber() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("first frame", || c.take_frame());
    c.update(CaptureSettings { monitor: r"\\.\DISPLAY2".into(), ..settings() });
    wait("second open", || {
        (probe.opened.lock().unwrap().last().map(String::as_str) == Some(r"\\.\DISPLAY2")).then_some(())
    });
    wait("status", || Some(c.status()).filter(|s| s.monitor == r"\\.\DISPLAY2"));
}

#[test]
fn a_fallback_is_reported_as_a_warning() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), CaptureSettings { monitor: "missing".into(), ..settings() });
    let s = wait("status", || Some(c.status()).filter(|s| s.active));
    assert!(s.error.as_deref().is_some_and(|e| e.contains("missing")), "{s:?}");
}

#[test]
fn dropping_stops_the_thread_and_releases_the_grabber() {
    let probe = red_probe();
    let c = Capture::start(factory(&probe), settings());
    wait("frame", || c.take_frame());
    drop(c);
    assert_eq!(probe.alive.load(Ordering::SeqCst), 0);
}

#[test]
fn rate_meter_measures_recent_ticks() {
    let t0 = Instant::now();
    let mut m = RateMeter::new();
    assert_eq!(m.rate(t0), 0.0);
    for i in 0..26 {
        m.tick(t0 + Duration::from_millis(40 * i));
    }
    let r = m.rate(t0 + Duration::from_secs(1));
    assert!((r - 25.0).abs() < 0.5, "{r}");
    assert_eq!(m.rate(t0 + RateMeter::WINDOW * 3), 0.0, "old ticks do not count");
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --test capture_thread_tests`
Expected: compile error — `capture::thread` and `ambilight::rate` do not exist.

- [ ] **Step 3: Rate meter**

Create `src/ambilight/rate.rs`:

```rust
//! Events per second over a sliding window (capture and send FPS).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct RateMeter {
    ticks: VecDeque<Instant>,
}

impl RateMeter {
    pub const WINDOW: Duration = Duration::from_secs(5);

    pub fn new() -> Self {
        Self::default()
    }

    pub fn tick(&mut self, now: Instant) {
        self.ticks.push_back(now);
        while self.ticks.front().is_some_and(|t| now.duration_since(*t) > Self::WINDOW) {
            self.ticks.pop_front();
        }
    }

    /// Rate of the ticks within the window before `now`, rounded to 0.1.
    pub fn rate(&self, now: Instant) -> f32 {
        let recent: Vec<Instant> =
            self.ticks.iter().copied().filter(|t| now.saturating_duration_since(*t) <= Self::WINDOW).collect();
        match (recent.first(), recent.last()) {
            (Some(first), Some(last)) if recent.len() >= 2 && last > first => {
                let r = (recent.len() - 1) as f32 / last.duration_since(*first).as_secs_f32();
                (r * 10.0).round() / 10.0
            }
            _ => 0.0,
        }
    }
}
```

Add `pub mod rate;` to `src/ambilight/mod.rs`.

- [ ] **Step 4: Capture thread**

Create `src/capture/thread.rs`:

```rust
//! Capture thread: grabs frames at the Ambilight frame rate, runs the colour
//! pipeline and leaves the newest LED colours in a slot for the device worker.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{error, info, warn};

use super::{Grabber, GrabberFactory};
use crate::ambilight::image::Image;
use crate::ambilight::pipeline::{Pipeline, Settings};
use crate::ambilight::rate::RateMeter;
use crate::app::state::CaptureStatus;
use crate::config::Config;
use crate::protocol::Rgb;

/// Pause before reopening a grabber that failed or could not be opened.
pub const REOPEN_DELAY: Duration = Duration::from_secs(1);
/// Panics in a row after which the thread gives up.
pub const MAX_PANICS: u32 = 3;
/// Longest single sleep, so a stop request is noticed quickly.
const SLICE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq)]
pub struct CaptureSettings {
    pub monitor: String,
    pub fps: u32,
    pub pipeline: Settings,
}

impl CaptureSettings {
    pub fn from_config(cfg: &Config) -> Self {
        Self { monitor: cfg.ambilight.monitor.clone(), fps: cfg.ambilight.fps, pipeline: Settings::from_config(cfg) }
    }
}

struct Shared {
    settings: Mutex<CaptureSettings>,
    frame: Mutex<Option<Vec<Rgb>>>,
    status: Mutex<CaptureStatus>,
    stop: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A running capture thread; dropping it stops the thread.
pub struct Capture {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    pub fn start(factory: GrabberFactory, settings: CaptureSettings) -> Capture {
        let shared = Arc::new(Shared {
            settings: Mutex::new(settings),
            frame: Mutex::new(None),
            status: Mutex::new(CaptureStatus::default()),
            stop: AtomicBool::new(false),
        });
        let s = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("capture".into())
            .spawn(move || supervise(&s, &factory))
            .expect("spawn capture thread");
        Capture { shared, thread: Some(thread) }
    }

    pub fn update(&self, settings: CaptureSettings) {
        *lock(&self.shared.settings) = settings;
    }

    /// The newest LED colours since the last call.
    pub fn take_frame(&self) -> Option<Vec<Rgb>> {
        lock(&self.shared.frame).take()
    }

    pub fn status(&self) -> CaptureStatus {
        lock(&self.shared.status).clone()
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Sleeps up to `d`, returning early when asked to stop.
fn pause(shared: &Shared, d: Duration) {
    let until = Instant::now() + d;
    while !shared.stop.load(Ordering::SeqCst) {
        let Some(rest) = until.checked_duration_since(Instant::now()) else { return };
        thread::sleep(rest.min(SLICE));
    }
}

fn set_unavailable(shared: &Shared, e: String) {
    let mut st = lock(&shared.status);
    st.active = false;
    st.error = Some(e);
}

fn supervise(shared: &Shared, factory: &GrabberFactory) {
    let mut panics = 0;
    while !shared.stop.load(Ordering::SeqCst) {
        match catch_unwind(AssertUnwindSafe(|| run(shared, factory))) {
            Ok(()) => return,
            Err(_) => {
                panics += 1;
                error!(panics, "capture thread panicked");
                if panics > MAX_PANICS {
                    set_unavailable(shared, "screen capture stopped after repeated internal errors".into());
                    return;
                }
                set_unavailable(
                    shared,
                    format!("screen capture restarted after an internal error ({panics}/{MAX_PANICS})"),
                );
                pause(shared, REOPEN_DELAY);
            }
        }
    }
}

fn run(shared: &Shared, factory: &GrabberFactory) {
    // The grabber and the monitor setting it was opened for.
    let mut grabber: Option<(Box<dyn Grabber>, String)> = None;
    let mut pipeline = Pipeline::new();
    let mut last: Option<Image> = None;
    let mut rate = RateMeter::new();
    let mut next = Instant::now();
    while !shared.stop.load(Ordering::SeqCst) {
        let settings = lock(&shared.settings).clone();
        if grabber.as_ref().is_some_and(|(_, m)| *m != settings.monitor) {
            grabber = None;
            last = None;
        }
        if grabber.is_none() {
            match factory(&settings.monitor) {
                Ok(g) => {
                    let i = g.info().clone();
                    info!(monitor = %i.monitor, adapter = %i.adapter, hdr = i.hdr, "capture started");
                    *lock(&shared.status) = CaptureStatus {
                        monitor: i.monitor,
                        adapter: i.adapter,
                        width: i.width,
                        height: i.height,
                        hdr: i.hdr,
                        sdr_white_nits: i.sdr_white_nits,
                        active: true,
                        error: i
                            .fallback
                            .then(|| format!("monitor {} not found, capturing the primary one", settings.monitor)),
                        ..CaptureStatus::default()
                    };
                    grabber = Some((g, settings.monitor.clone()));
                }
                Err(e) => {
                    warn!(%e, "screen capture unavailable");
                    set_unavailable(shared, e);
                    pause(shared, REOPEN_DELAY);
                    continue;
                }
            }
        }
        let (g, _) = grabber.as_mut().expect("grabber just opened");
        match g.grab() {
            Ok(Some(img)) => last = Some(img),
            Ok(None) => {}
            Err(e) => {
                warn!(%e, "screen capture lost, reopening");
                set_unavailable(shared, e);
                grabber = None;
                pause(shared, REOPEN_DELAY);
                continue;
            }
        }
        let now = Instant::now();
        if let Some(img) = &last {
            let colors = pipeline.process(img, &settings.pipeline, now);
            *lock(&shared.frame) = Some(colors);
            rate.tick(now);
            let bars = pipeline.bars();
            let mut st = lock(&shared.status);
            st.fps = rate.rate(now);
            st.bars = [bars.vertical as f32 / img.height() as f32, bars.horizontal as f32 / img.width() as f32];
        }
        // Sleep to the next frame time; the next grab takes whatever the
        // desktop accumulated meanwhile, so a 240 Hz screen costs `fps` copies.
        next += Duration::from_secs(1) / settings.fps.max(1);
        let now = Instant::now();
        if next < now {
            next = now;
        }
        pause(shared, next - now);
    }
}
```

Add `pub mod thread;` to `src/capture/mod.rs`.

- [ ] **Step 5: Run the tests**

Run: `cargo test --test capture_thread_tests`
Expected: PASS (8 tests).

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
git add src/capture src/ambilight tests/capture_thread_tests.rs
git commit -m "Run screen capture and the colour pipeline on a capture thread"
```

---

### Task 8: Streaming in the worker and the UDP link

**Files:**
- Modify: `src/protocol/device.rs`, `src/app/worker.rs`, `src/app/link.rs`, `src/ui/tray.rs` (spawn call only), `tests/protocol_tests.rs`, `tests/worker_tests.rs`

**Interfaces:**
- Consumes: Task 5 (`Action::{StartStream, StopStream, Shutdown}`, `Event::{StreamStarted, StreamStopped, StreamFailed}`, `Controller::{streaming, highlight, capture_wanted}`), Task 7 (`Capture`, `CaptureSettings`, `RateMeter`), Task 1 (`layout::highlight_frame`), Task 6 (`GrabberFactory`, `DxgiGrabber`).
- Produces:
  - `Chameleon2::{set_led_count(usize) -> Result<()>, leave_direct() -> Result<()>}`.
  - `DeviceLink::{start_stream(&mut self, ip, led_count) -> Result<(), String>, send_frame(&mut self, &[Rgb]) -> Result<(), String>, stream_tick(&mut self) -> Result<(), String>, stop_stream(&mut self, ip, then: LightChange, quick: bool) -> Result<(), String>}`.
  - `Worker::spawn(config, warnings, config_path, link, grabbers: GrabberFactory, notify, options)`; `worker::RESEND_EVERY` (1 s).
  - Snapshots published by the worker carry `stream.capture` and `stream.send_fps`.

- [ ] **Step 1: Write the failing protocol test**

Append to `tests/protocol_tests.rs`:

```rust
#[test]
fn led_count_can_change_in_direct_mode_and_direct_mode_can_be_left() {
    let mut d = device(4);
    d.enter_direct().unwrap();
    assert!(d.set_led_count(0).is_err());
    assert!(d.set_led_count(MAX_LEDS + 1).is_err());
    d.set_led_count(6).unwrap();
    d.send_frame(&[Rgb::new(1, 1, 1); 6]).unwrap();
    d.leave_direct().unwrap();
    assert!(!d.is_direct());
    assert_eq!(sent_methods(d.session()).last().unwrap(), "deactivate_fx_mode");
}
```

Run: `cargo test --test protocol_tests led_count_can_change`
Expected: compile error — no method `set_led_count`.

- [ ] **Step 2: Implement the device methods**

In `src/protocol/device.rs`, inside `impl<T: Transport> Chameleon2<T>` after `led_count()`:

```rust
    /// Changes the frame length expected by `send_frame` (the LED count setting changed).
    pub fn set_led_count(&mut self, led_count: usize) -> Result<()> {
        if !(1..=MAX_LEDS).contains(&led_count) {
            return Err(ProtocolError::InvalidInput(format!("LED count {led_count} not in 1..={MAX_LEDS}")));
        }
        self.led_count = led_count;
        Ok(())
    }
```

and after `enter_direct`:

```rust
    /// `deactivate_fx_mode direct`; the static state is applied separately.
    pub fn leave_direct(&mut self) -> Result<()> {
        self.ensure_session()?;
        self.session.request(&Call::deactivate_fx_mode("direct"))?;
        self.direct = false;
        info!("direct mode left");
        Ok(())
    }
```

Run: `cargo test --test protocol_tests`
Expected: PASS.

- [ ] **Step 3: Write the failing worker tests**

In `tests/worker_tests.rs`:

1. Replace everything above `fn dev(` (imports, `Call`, `Script`, `FakeLink`) with:

```rust
mod common;

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use obsid_ambilight::ambilight::image::Image;
use obsid_ambilight::ambilight::layout::HighlightTarget;
use obsid_ambilight::app::commands::Command;
use obsid_ambilight::app::controller::{LightChange, Target};
use obsid_ambilight::app::state::{Connection, DeviceInfo, Snapshot};
use obsid_ambilight::app::worker::{DeviceLink, Notice, Worker, WorkerOptions, resolve};
use obsid_ambilight::capture::{Grabber, GrabberFactory, GrabberInfo};
use obsid_ambilight::config::{self, Config, ExitAction, Mode};
use obsid_ambilight::protocol::Rgb;

const IP: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 85);

#[derive(Debug, Clone, PartialEq)]
enum Call {
    Discover,
    Handshake(Ipv4Addr),
    Apply(Ipv4Addr, LightChange),
    ExitOff(Ipv4Addr),
    StartStream(Ipv4Addr, usize),
    Frame(Vec<Rgb>),
    StopStream(Ipv4Addr, LightChange, bool),
}

#[derive(Default)]
struct Script {
    calls: Vec<Call>,
    devices: Vec<DeviceInfo>,
    fail_handshake: bool,
    panic_on_first_discover: bool,
    /// The next `stream_tick` reports a lost session.
    fail_next_tick: bool,
    /// Session counts as open until this moment (set by `apply`).
    session_until: Option<Instant>,
    streaming: bool,
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
    fn frames(&self) -> Vec<Vec<Rgb>> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                Call::Frame(f) => Some(f),
                _ => None,
            })
            .collect()
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
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::Apply(ip, change));
        s.session_until = Some(Instant::now() + Duration::from_millis(300));
        Ok(())
    }
    fn power_off_for_exit(&mut self, ip: Ipv4Addr) -> Result<(), String> {
        self.0.lock().unwrap().calls.push(Call::ExitOff(ip));
        Ok(())
    }
    fn start_stream(&mut self, ip: Ipv4Addr, led_count: usize) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::StartStream(ip, led_count));
        s.streaming = true;
        Ok(())
    }
    fn send_frame(&mut self, colors: &[Rgb]) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        assert!(s.streaming, "frame sent outside a stream");
        s.calls.push(Call::Frame(colors.to_vec()));
        Ok(())
    }
    fn stream_tick(&mut self) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        if std::mem::take(&mut s.fail_next_tick) {
            s.streaming = false;
            return Err("keep-alive lost".into());
        }
        Ok(())
    }
    fn stop_stream(&mut self, ip: Ipv4Addr, then: LightChange, quick: bool) -> Result<(), String> {
        let mut s = self.0.lock().unwrap();
        s.calls.push(Call::StopStream(ip, then, quick));
        s.streaming = false;
        Ok(())
    }
    fn session_open(&self) -> bool {
        let s = self.0.lock().unwrap();
        s.streaming || s.session_until.is_some_and(|t| Instant::now() < t)
    }
}

/// Grabber that always shows one solid colour.
struct SolidGrabber {
    info: GrabberInfo,
    color: [f32; 3],
    alive: Arc<AtomicUsize>,
}

impl Grabber for SolidGrabber {
    fn info(&self) -> &GrabberInfo {
        &self.info
    }
    fn grab(&mut self) -> Result<Option<Image>, String> {
        Ok(Some(Image::filled(32, 18, self.color)))
    }
}

impl Drop for SolidGrabber {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Factory of red-screen grabbers; the counter tracks live grabbers.
fn red_screen() -> (GrabberFactory, Arc<AtomicUsize>) {
    let alive = Arc::new(AtomicUsize::new(0));
    let a = Arc::clone(&alive);
    let f: GrabberFactory = Arc::new(move |_: &str| {
        a.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(SolidGrabber { info: GrabberInfo::default(), color: [1.0, 0.0, 0.0], alive: Arc::clone(&a) })
            as Box<dyn Grabber>)
    });
    (f, alive)
}
```

2. Replace `spawn` with:

```rust
fn spawn(cfg: Config, link: FakeLink, path: Option<std::path::PathBuf>) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    spawn_with(cfg, link, path, red_screen().0)
}

fn spawn_with(
    cfg: Config,
    link: FakeLink,
    path: Option<std::path::PathBuf>,
    grabbers: GrabberFactory,
) -> (Worker, Arc<Mutex<Vec<Notice>>>) {
    let notices = Arc::new(Mutex::new(Vec::new()));
    let n = Arc::clone(&notices);
    let w = Worker::spawn(
        cfg,
        Vec::new(),
        path,
        Box::new(link),
        grabbers,
        Arc::new(move |x: Notice| n.lock().unwrap().push(x)),
        fast(),
    );
    (w, notices)
}

fn ambilight_cfg() -> Config {
    let mut c = saved_cfg();
    c.app.mode = Mode::Ambilight;
    c.ambilight.smoothing = 0;
    c
}

fn wait_calls(link: &FakeLink, what: &str, pred: impl Fn(&[Call]) -> bool) -> Vec<Call> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let calls = link.calls();
        if pred(&calls) {
            return calls;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {calls:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
```

3. Append the streaming tests:

```rust
// ---------- streaming ----------

#[test]
fn ambilight_streams_captured_frames() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "stream start", |c| c.contains(&Call::StartStream(IP, 60)));
    wait_calls(&link, "red frame", |c| c.contains(&Call::Frame(vec![Rgb::new(255, 0, 0); 60])));
    let s = wait_for(&w, "capture status", |s| s.stream.capture.as_ref().is_some_and(|c| c.active));
    assert!(s.stream.streaming);
}

#[test]
fn an_unchanged_screen_is_resent_once_per_second() {
    let link = FakeLink::with_strip();
    let (_w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    let before = link.frames().len();
    std::thread::sleep(Duration::from_millis(2200));
    let sent = link.frames().len() - before;
    assert!((1..=3).contains(&sent), "{sent} frames for a still screen in 2.2 s");
}

#[test]
fn switching_to_static_stops_capture_and_the_stream() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(ambilight_cfg(), link.clone(), None, grabbers);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert_eq!(alive.load(Ordering::SeqCst), 1);
    w.send(Command::SetMode { mode: Mode::Static });
    let full = LightChange::full(&Config::default().light);
    wait_calls(&link, "stop", |c| c.contains(&Call::StopStream(IP, full, false)));
    wait_for(&w, "capture stopped", |s| s.stream.capture.is_none() && !s.stream.streaming);
    assert_eq!(alive.load(Ordering::SeqCst), 0, "the grabber is released");
    let frames = link.frames().len();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(link.frames().len(), frames, "no frames after the stop");
}

#[test]
fn stream_failure_goes_offline_and_resumes() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    link.0.lock().unwrap().fail_next_tick = true;
    wait_for(&w, "offline", |s| s.connection == Connection::Offline && s.stream.capture.is_none());
    wait_calls(&link, "second start", |c| c.iter().filter(|x| matches!(x, Call::StartStream(..))).count() == 2);
    wait_for(&w, "streaming again", |s| s.connection == Connection::Ready && s.stream.streaming);
}

#[test]
fn highlight_from_static_sends_markers_then_returns_to_static() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(saved_cfg(), link.clone(), None, grabbers);
    wait_for(&w, "ready", |s| s.connection == Connection::Ready);
    w.send(Command::Highlight { target: HighlightTarget::Led(5) });
    let calls = wait_calls(&link, "marker frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert!(calls.contains(&Call::StartStream(IP, 60)));
    let marker = link.frames()[0].clone();
    assert_eq!(marker[5], Rgb::new(255, 255, 255));
    assert_eq!(alive.load(Ordering::SeqCst), 0, "no capture for a highlight in static mode");
    // Nobody renews the highlight (the window was closed): back to static after 3 s.
    let full = LightChange::full(&Config::default().light);
    let deadline = Instant::now() + Duration::from_secs(6);
    while !link.calls().contains(&Call::StopStream(IP, full, false)) {
        assert!(Instant::now() < deadline, "{:?}", link.calls());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn led_count_change_while_streaming_resizes_frames() {
    let link = FakeLink::with_strip();
    let (w, _) = spawn(ambilight_cfg(), link.clone(), None);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(f) if f.len() == 60)));
    w.send(Command::SetLedCount { count: 40 });
    wait_calls(&link, "40-LED frame", |c| c.iter().any(|x| matches!(x, Call::Frame(f) if f.len() == 40)));
}

#[test]
fn exit_during_ambilight_leaves_direct_mode_with_the_static_colour() {
    let link = FakeLink::with_strip();
    let (grabbers, alive) = red_screen();
    let (w, _) = spawn_with(ambilight_cfg(), link.clone(), None, grabbers);
    wait_calls(&link, "first frame", |c| c.iter().any(|x| matches!(x, Call::Frame(_))));
    assert!(w.send(Command::Exit));
    assert!(w.join(Duration::from_secs(3)), "worker did not stop");
    let full = LightChange::full(&Config::default().light);
    assert!(link.calls().contains(&Call::StopStream(IP, full, true)), "{:?}", link.calls());
    assert_eq!(alive.load(Ordering::SeqCst), 0);
}
```

Run: `cargo test --test worker_tests`
Expected: compile errors (`DeviceLink` has no `start_stream`, `Worker::spawn` takes six arguments).

- [ ] **Step 4: Extend `DeviceLink` and the worker**

In `src/app/worker.rs`:

1. Imports:

```rust
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
use crate::ambilight::layout;
use crate::ambilight::rate::RateMeter;
use crate::capture::GrabberFactory;
use crate::capture::thread::{Capture, CaptureSettings};
use crate::config::{self, Config};
use crate::protocol::Rgb;
use crate::reconnect::Backoff;
```

2. After `SESSION_RECHECK` add:

```rust
/// An unchanged frame is sent again after this long.
pub const RESEND_EVERY: Duration = Duration::from_secs(1);
/// How often stream statistics are published while streaming.
const STREAM_PUBLISH: Duration = Duration::from_secs(1);
```

3. Add to the `DeviceLink` trait (after `power_off_for_exit`):

```rust
    /// New session → power on → brightness 100 → `activate_fx_mode direct`.
    fn start_stream(&mut self, ip: Ipv4Addr, led_count: usize) -> Result<(), String>;
    /// One `update_leds` frame (no reply); the length may change between frames.
    fn send_frame(&mut self, colors: &[Rgb]) -> Result<(), String>;
    /// Keep-alive while streaming; `Err` once the session is lost.
    fn stream_tick(&mut self) -> Result<(), String>;
    /// `deactivate_fx_mode direct`, then `then`; `quick` uses short timeouts (exit).
    fn stop_stream(&mut self, ip: Ipv4Addr, then: LightChange, quick: bool) -> Result<(), String>;
```

4. `Worker::spawn` gains `grabbers: GrabberFactory` after `link`; `Ctx` gains the same field:

```rust
    pub fn spawn(
        config: Config,
        warnings: Vec<String>,
        config_path: Option<PathBuf>,
        mut link: Box<dyn DeviceLink>,
        grabbers: GrabberFactory,
        notify: Notify,
        options: WorkerOptions,
    ) -> Worker {
```

inside the thread closure `let ctx = Ctx { shared, notify, config_path, options, grabbers };`, and

```rust
struct Ctx {
    shared: Arc<Mutex<Snapshot>>,
    notify: Notify,
    config_path: Option<PathBuf>,
    options: WorkerOptions,
    grabbers: GrabberFactory,
}
```

5. Replace `run`, `RunState`, `process` and `save_now` (the temporary `StartStream`/`StopStream` arms from Task 5 go away) with:

```rust
/// One worker life. Returns normally only on exit or when all senders are gone.
fn run(ctrl: &mut Controller, link: &mut dyn DeviceLink, rx: &Receiver<Command>, ctx: &Ctx) {
    let mut state = RunState::default();
    process(ctrl, link, ctx, &mut state, Event::Startup);

    while !state.exit {
        let now = Instant::now();
        let deadline = [ctrl.next_deadline(), state.save_at, state.session_check_at, state.stream.next_frame]
            .into_iter()
            .flatten()
            .min();
        let wait = deadline.map_or(Duration::from_secs(3600), |d| d.saturating_duration_since(now));
        match rx.recv_timeout(wait) {
            Ok(first) => {
                let mut batch = vec![first];
                batch.extend(rx.try_iter());
                // Throttle: wait out the minimum interval, collecting newer commands.
                if batch.iter().any(Command::is_light_change)
                    && let Some(last) = state.last_apply
                {
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
                for cmd in coalesce(batch) {
                    process(ctrl, link, ctx, &mut state, Event::Command(cmd));
                    if state.exit {
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => process(ctrl, link, ctx, &mut state, Event::Tick),
            Err(RecvTimeoutError::Disconnected) => {
                state.stream.capture = None;
                save_now(ctrl, ctx, &mut state);
                return;
            }
        }
        if !state.exit && state.stream.next_frame.is_some_and(|t| Instant::now() >= t) {
            stream_step(ctrl, link, ctx, &mut state);
        }
        if state.save_at.is_some_and(|t| Instant::now() >= t) {
            save_now(ctrl, ctx, &mut state);
        }
    }
}

#[derive(Default)]
struct RunState {
    save_at: Option<Instant>,
    last_apply: Option<Instant>,
    /// While a session is open, when to re-check it (no network traffic).
    session_check_at: Option<Instant>,
    exit: bool,
    stream: Stream,
}

/// Frame pacing and the capture thread while the strip is in direct mode.
#[derive(Default)]
struct Stream {
    capture: Option<Capture>,
    /// Settings last handed to the capture thread.
    capture_settings: Option<CaptureSettings>,
    next_frame: Option<Instant>,
    last_sent: Option<(Vec<Rgb>, Instant)>,
    sends: RateMeter,
    publish_at: Option<Instant>,
}

/// Feeds one event and executes the resulting actions, following up with the
/// events they produce until nothing is left.
fn process(ctrl: &mut Controller, link: &mut dyn DeviceLink, ctx: &Ctx, st: &mut RunState, event: Event) {
    let mut queue = VecDeque::from([event]);
    while let Some(ev) = queue.pop_front() {
        let actions = ctrl.handle(ev, Instant::now());
        publish(ctrl, link, ctx, st);
        for action in actions {
            match action {
                Action::Resolve(target) => queue.push_back(Event::Resolved(resolve(link, &target))),
                Action::Discover => queue.push_back(Event::Discovered(link.discover())),
                Action::Apply { ip, change } => {
                    st.last_apply = Some(Instant::now());
                    queue.push_back(Event::Applied(link.apply(ip, change)));
                }
                Action::StartStream { ip } => {
                    let led_count = ctrl.config().device.led_count;
                    queue.push_back(Event::StreamStarted(link.start_stream(ip, led_count)));
                }
                Action::StopStream { ip, then } => {
                    st.stream.capture = None;
                    st.last_apply = Some(Instant::now());
                    queue.push_back(Event::StreamStopped(ip, link.stop_stream(ip, then, false)));
                }
                Action::Test { ip } => queue.push_back(Event::Tested(link.handshake(ip))),
                Action::SaveConfig => {
                    st.save_at.get_or_insert(Instant::now() + ctx.options.save_delay);
                }
                Action::OpenSettings => (ctx.notify)(Notice::OpenSettings),
                Action::Shutdown { power_off_ip, stop_stream } => {
                    st.stream.capture = None;
                    if let Some((ip, then)) = stop_stream
                        && let Err(e) = link.stop_stream(ip, then, true)
                    {
                        info!(%e, "leaving direct mode on exit failed");
                    }
                    if let Some(ip) = power_off_ip
                        && let Err(e) = link.power_off_for_exit(ip)
                    {
                        info!(%e, "power off on exit failed");
                    }
                    save_now(ctrl, ctx, st);
                    st.exit = true;
                    (ctx.notify)(Notice::Stopped);
                    return;
                }
            }
        }
        sync_stream(ctrl, ctx, st);
        publish(ctrl, link, ctx, st);
    }
    // Sessions expire locally without any event; wake up to republish then.
    st.session_check_at = link.session_open().then(|| Instant::now() + SESSION_RECHECK);
}

/// Starts, stops or updates the capture thread and the frame pacing to match the controller.
fn sync_stream(ctrl: &Controller, ctx: &Ctx, st: &mut RunState) {
    let s = &mut st.stream;
    if ctrl.capture_wanted() {
        let settings = CaptureSettings::from_config(ctrl.config());
        match &s.capture {
            None => s.capture = Some(Capture::start(Arc::clone(&ctx.grabbers), settings.clone())),
            Some(c) if s.capture_settings.as_ref() != Some(&settings) => c.update(settings.clone()),
            Some(_) => {}
        }
        s.capture_settings = Some(settings);
    } else {
        // Dropping the capture stops its thread.
        s.capture = None;
        s.capture_settings = None;
    }
    if ctrl.streaming() {
        s.next_frame.get_or_insert_with(Instant::now);
    } else {
        s.next_frame = None;
        s.last_sent = None;
    }
}

/// Sends the frame that is due (highlight or captured), then the keep-alive.
fn stream_step(ctrl: &mut Controller, link: &mut dyn DeviceLink, ctx: &Ctx, st: &mut RunState) {
    let now = Instant::now();
    let cfg = ctrl.config();
    let period = Duration::from_secs(1) / cfg.ambilight.fps.max(1);
    let fresh = match ctrl.highlight() {
        Some(target) => Some(layout::highlight_frame(target, &cfg.ambilight.segments, cfg.device.led_count)),
        None => st.stream.capture.as_ref().and_then(Capture::take_frame),
    };
    let s = &mut st.stream;
    let resend_due = s.last_sent.as_ref().is_none_or(|(_, at)| now.duration_since(*at) >= RESEND_EVERY);
    let to_send = match fresh {
        Some(f) if s.last_sent.as_ref().is_none_or(|(prev, _)| *prev != f) => Some(f),
        _ if resend_due => s.last_sent.as_ref().map(|(prev, _)| prev.clone()),
        _ => None,
    };
    let mut failure = None;
    if let Some(frame) = to_send {
        match link.send_frame(&frame) {
            Ok(()) => {
                s.sends.tick(now);
                s.last_sent = Some((frame, now));
            }
            Err(e) => failure = Some(e),
        }
    }
    if failure.is_none()
        && let Err(e) = link.stream_tick()
    {
        failure = Some(e);
    }
    s.next_frame = Some(now + period);
    if let Some(e) = failure {
        process(ctrl, link, ctx, st, Event::StreamFailed(e));
        return;
    }
    if st.stream.publish_at.is_none_or(|t| now >= t) {
        st.stream.publish_at = Some(now + STREAM_PUBLISH);
        publish(ctrl, link, ctx, st);
    }
}

/// Publishes the controller snapshot plus the worker's stream statistics.
fn publish(ctrl: &Controller, link: &dyn DeviceLink, ctx: &Ctx, st: &RunState) {
    let mut snap = ctrl.snapshot(link.session_open());
    snap.stream.capture = st.stream.capture.as_ref().map(Capture::status);
    snap.stream.send_fps = if ctrl.streaming() { st.stream.sends.rate(Instant::now()) } else { 0.0 };
    ctx.publish_snapshot(snap);
}

fn save_now(ctrl: &mut Controller, ctx: &Ctx, st: &mut RunState) {
    if st.save_at.take().is_none() {
        return;
    }
    if let Some(path) = &ctx.config_path
        && let Err(e) = config::save(path, ctrl.config())
    {
        ctrl.record_error(format!("could not save settings: {e}"));
        // Saving runs after the last publish; show the error right away,
        // keeping the stream statistics of that publish.
        let prev = ctx.snapshot();
        let mut snap = ctrl.snapshot(prev.session_open);
        snap.stream.capture = prev.stream.capture;
        snap.stream.send_fps = prev.stream.send_fps;
        ctx.publish_snapshot(snap);
    }
}
```

Borrow note for the implementer: in `stream_step`, `cfg` borrows `ctrl` only until `fresh` is computed; if the compiler complains, copy `period` and the highlight frame into locals before taking `&mut st.stream`.

- [ ] **Step 5: Implement the link**

In `src/app/link.rs`:

1. Add `use crate::protocol::Rgb;` to the imports.

2. Below `fn text` add, and use it in `power_off_for_exit` instead of the inline `quick` literal:

```rust
fn quick_config() -> SessionConfig {
    SessionConfig { reply_timeout: Duration::from_millis(400), handshake_attempts: 1, ..SessionConfig::default() }
}
```

3. Add to `impl DeviceLink for UdpLink`:

```rust
    fn start_stream(&mut self, ip: Ipv4Addr, led_count: usize) -> Result<(), String> {
        self.current = None;
        let mut dev = Chameleon2::connect(ip, led_count, SessionConfig::default()).map_err(text)?;
        // Hardware brightness stays at 100; Ambilight brightness is applied to the frames.
        dev.set_power(true).and_then(|()| dev.set_brightness(100)).and_then(|()| dev.enter_direct()).map_err(text)?;
        self.current = Some((ip, dev, Instant::now()));
        Ok(())
    }

    fn send_frame(&mut self, colors: &[Rgb]) -> Result<(), String> {
        let Some((_, dev, used)) = self.current.as_mut() else { return Err("no stream is running".into()) };
        if dev.led_count() != colors.len() {
            dev.set_led_count(colors.len()).map_err(text)?;
        }
        dev.send_frame(colors).map_err(text)?;
        *used = Instant::now();
        Ok(())
    }

    fn stream_tick(&mut self) -> Result<(), String> {
        let Some((_, dev, _)) = self.current.as_mut() else { return Err("no stream is running".into()) };
        // A single unanswered keep-alive is tolerated; the session decides when it is lost.
        if let Err(e) = dev.tick(Instant::now())
            && !dev.session().is_open()
        {
            return Err(text(e));
        }
        if dev.is_direct() { Ok(()) } else { Err("direct mode was lost".into()) }
    }

    fn stop_stream(&mut self, ip: Ipv4Addr, then: LightChange, quick: bool) -> Result<(), String> {
        let reusable = matches!(&self.current, Some((cur, dev, _)) if *cur == ip && dev.session().is_open());
        if !reusable {
            let cfg = if quick { quick_config() } else { SessionConfig::default() };
            self.current = Some((ip, Self::connect(ip, cfg).map_err(text)?, Instant::now()));
        }
        let (_, dev, used) = self.current.as_mut().expect("session just ensured");
        *used = Instant::now();
        dev.leave_direct().and_then(|()| send(dev, then)).map_err(text)
    }
```

- [ ] **Step 6: Give the tray a grabber factory**

In `src/ui/tray.rs` add imports

```rust
use crate::capture::dxgi::DxgiGrabber;
use crate::capture::{Grabber, GrabberFactory};
```

and pass a factory to `Worker::spawn`:

```rust
    let grabbers: GrabberFactory =
        Arc::new(|monitor: &str| DxgiGrabber::open(monitor).map(|g| Box::new(g) as Box<dyn Grabber>));
    let worker = Worker::spawn(
        loaded.config,
        loaded.warnings,
        Some(cfg_path),
        Box::new(UdpLink::new()),
        grabbers,
        notify,
        WorkerOptions::default(),
    );
```

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: PASS, including the 7 new worker tests (a few seconds: highlight expiry 3 s, resend 2.2 s, reconnect ≤ 2 s).

- [ ] **Step 8: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/protocol/device.rs src/app src/ui/tray.rs tests/protocol_tests.rs tests/worker_tests.rs
git commit -m "Stream Ambilight frames from the device worker"
```

---

### Task 9: Tray — mode items, Ambilight icon and tooltip

**Files:**
- Modify: `src/i18n.rs`, `src/ui/icons.rs`, `src/ui/tray.rs`, `tests/icons_tests.rs`
- Create: `tests/tray_tests.rs`

**Interfaces:**
- Consumes: Task 5 (`Snapshot.stream`, `Command::{SetMode, SetAmbilight}`), `config::Mode`.
- Produces: `IconKind::Ambilight { warn: bool }`; `ui::tray::tooltip(&Snapshot, &Strings) -> String` (now `pub`); strings `mode_ambilight`, `static_item`, `ambilight_item`, `capture_unavailable`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/icons_tests.rs` (add `use obsid_ambilight::app::state::CaptureStatus;` and `use obsid_ambilight::config::Mode;`):

```rust
#[test]
fn ambilight_icon_is_a_ring_of_four_colours() {
    let img = render(IconKind::Ambilight { warn: false }, 32);
    assert_eq!(px(&img, 32, 16, 16)[3], 0, "hollow centre");
    let corners = [px(&img, 32, 6, 6), px(&img, 32, 25, 6), px(&img, 32, 25, 25), px(&img, 32, 6, 25)];
    for c in corners {
        assert_eq!(c[3], 255, "{corners:?}");
    }
    for i in 0..4 {
        for j in i + 1..4 {
            assert_ne!(corners[i][..3], corners[j][..3], "quadrants {i} and {j}");
        }
    }
}

#[test]
fn ambilight_warning_adds_an_amber_dot() {
    let plain = render(IconKind::Ambilight { warn: false }, 32);
    let warned = render(IconKind::Ambilight { warn: true }, 32);
    assert_eq!(px(&warned, 32, 25, 25), [0xF2, 0xB3, 0x3D, 255]);
    assert_ne!(px(&plain, 32, 25, 25), px(&warned, 32, 25, 25));
}

#[test]
fn icon_kind_shows_ambilight_and_unavailable_capture() {
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false);
    s.connection = Connection::Ready;
    s.config.app.mode = Mode::Ambilight;
    assert_eq!(kind_for(&s), IconKind::Ambilight { warn: false });
    s.stream.capture = Some(CaptureStatus { active: false, error: Some("lost".into()), ..CaptureStatus::default() });
    assert_eq!(kind_for(&s), IconKind::Ambilight { warn: true });
    s.config.light.power = false;
    assert_eq!(kind_for(&s), IconKind::Off);
}
```

Create `tests/tray_tests.rs`:

```rust
use std::net::Ipv4Addr;
use std::time::Duration;

use obsid_ambilight::app::controller::Controller;
use obsid_ambilight::app::state::{CaptureStatus, Connection, DeviceInfo, Snapshot};
use obsid_ambilight::config::{Config, Mode};
use obsid_ambilight::i18n::EN;
use obsid_ambilight::reconnect::Backoff;
use obsid_ambilight::ui::tray::tooltip;

fn ready() -> Snapshot {
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(false);
    s.connection = Connection::Ready;
    s.device =
        Some(DeviceInfo { id: "x".into(), name: String::new(), ip: Ipv4Addr::new(192, 168, 1, 85), fw_ver: None });
    s
}

#[test]
fn static_tooltip_is_unchanged() {
    assert_eq!(tooltip(&ready(), &EN), "Obsid Ambilight — Static #F5D4A1 · 40% · 192.168.1.85");
}

#[test]
fn ambilight_tooltip_shows_rate_hdr_and_address() {
    let mut s = ready();
    s.config.app.mode = Mode::Ambilight;
    s.stream.capture = Some(CaptureStatus { hdr: true, active: true, ..CaptureStatus::default() });
    assert_eq!(tooltip(&s, &EN), "Obsid Ambilight — Ambilight · 25 FPS · HDR · 192.168.1.85");
}

#[test]
fn unavailable_capture_is_mentioned() {
    let mut s = ready();
    s.config.app.mode = Mode::Ambilight;
    s.stream.capture = Some(CaptureStatus { active: false, error: Some("lost".into()), ..CaptureStatus::default() });
    assert!(tooltip(&s, &EN).contains("capture unavailable"), "{}", tooltip(&s, &EN));
}
```

Run: `cargo test --test icons_tests --test tray_tests`
Expected: compile errors (`IconKind::Ambilight`, private `tooltip`).

- [ ] **Step 2: Add the strings**

In `src/i18n.rs`, add to `Strings` after `mode_off`:

```rust
    pub mode_ambilight: &'static str,
    pub capture_unavailable: &'static str,
```

and after `power_on_item`:

```rust
    pub static_item: &'static str,
    pub ambilight_item: &'static str,
```

Values — `RU`: `mode_ambilight: "Ambilight"`, `capture_unavailable: "захват недоступен"`, `static_item: "Статика"`, `ambilight_item: "Ambilight"`. `EN`: `mode_ambilight: "Ambilight"`, `capture_unavailable: "capture unavailable"`, `static_item: "Static"`, `ambilight_item: "Ambilight"`.

- [ ] **Step 3: Draw the Ambilight icon**

In `src/ui/icons.rs`:

1. Imports: `use std::f32::consts::{FRAC_PI_2, PI};`, `use crate::config::Mode;`.

2. Add the variant:

```rust
    /// Ambilight: a ring of four colours; `warn` adds an amber dot (capture unavailable).
    Ambilight { warn: bool },
```

3. Add the colours below `RED`:

```rust
const ARCS: [Rgb; 4] = [RED, Rgb::new(0x3D, 0xB2, 0x5A), Rgb::new(0x3D, 0x7E, 0xE5), AMBER];
```

4. In `kind_for`, put this arm first:

```rust
        Connection::Ready if s.config.light.power && s.config.app.mode == Mode::Ambilight => IconKind::Ambilight {
            warn: s.stream.capture.as_ref().is_some_and(|c| !c.active && c.error.is_some()),
        },
```

5. At the top of `render` add `if let IconKind::Ambilight { warn } = kind { return ambilight(size, warn); }`, add `IconKind::Ambilight { .. } => unreachable!("drawn by ambilight()"),` to its `match`, and add:

```rust
/// Ring split into four coloured quadrants, with an optional amber dot at the bottom right.
fn ambilight(size: u32, warn: bool) -> Vec<u8> {
    let s = size as f32;
    let c = s / 2.0;
    let (outer, inner) = (s * 0.44, s * 0.27);
    let (dot_c, dot_r) = (s * 0.78, s * 0.2);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let (dx, dy) = (px - c, py - c);
            let d = (dx * dx + dy * dy).sqrt();
            let ring = ((outer - d + 0.5).clamp(0.0, 1.0) - (inner - d + 0.5).clamp(0.0, 1.0)).max(0.0);
            let quadrant = (((dy.atan2(dx) + PI) / FRAC_PI_2) as usize).min(3);
            let dot = if warn {
                (dot_r - ((px - dot_c).powi(2) + (py - dot_c).powi(2)).sqrt() + 0.5).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (color, alpha) = if dot > 0.0 { (AMBER, dot.max(ring)) } else { (ARCS[quadrant], ring) };
            out.extend_from_slice(&[color.r, color.g, color.b, (alpha * 255.0).round() as u8]);
        }
    }
    out
}
```

- [ ] **Step 4: Tray menu, brightness and tooltip**

In `src/ui/tray.rs`:

1. Add `use crate::config::Mode;` and the ids:

```rust
const ID_MODE_STATIC: u32 = 6;
const ID_MODE_AMBILIGHT: u32 = 7;
```

2. Add a field to `TrayApp`: `/// Icon and tooltip last shown; unchanged ones are not re-sent. shown: Option<(icons::IconKind, String)>,` and initialise it with `shown: None` in `run`.

3. In `refresh_icon`, right after `let s = Self::strings(&snap);`:

```rust
        let kind = icons::kind_for(&snap);
        let tip_text = tooltip(&snap, s);
        // Stream statistics republish every second; skip identical updates.
        if !add && self.shown.as_ref() == Some(&(kind, tip_text.clone())) {
            return;
        }
        self.shown = Some((kind, tip_text.clone()));
```

and use `kind` / `tip_text` in place of `icons::kind_for(&snap)` / `tooltip(&snap, s)` below.

4. In `show_menu`, after the power item:

```rust
            let mode_check = |m: Mode| {
                if snap.config.light.power && snap.config.app.mode == m { MF_CHECKED } else { 0 }
            };
            append(menu, MF_STRING | mode_check(Mode::Static), ID_MODE_STATIC as usize, s.static_item);
            append(menu, MF_STRING | mode_check(Mode::Ambilight), ID_MODE_AMBILIGHT as usize, s.ambilight_item);
```

and in the brightness submenu use

```rust
            let current = if snap.config.app.mode == Mode::Ambilight {
                snap.config.ambilight.brightness
            } else {
                snap.config.light.brightness
            };
```

with `u32::from(current) == pct` for the check mark.

5. In `on_menu` add the mode arms and replace the brightness arm:

```rust
            ID_MODE_STATIC => self.send(Command::SetMode { mode: Mode::Static }),
            ID_MODE_AMBILIGHT => self.send(Command::SetMode { mode: Mode::Ambilight }),
```

```rust
            id if (ID_BRIGHT + 1..=ID_BRIGHT + 10).contains(&id) => {
                let pct = ((id - ID_BRIGHT) * 10) as u8;
                if snap.config.app.mode == Mode::Ambilight {
                    let mut settings = snap.config.ambilight.clone();
                    settings.brightness = pct;
                    self.send(Command::SetAmbilight { settings });
                } else {
                    self.send(Command::SetStatic { color: None, brightness: Some(pct) });
                }
            }
```

6. Replace `tooltip` with a public version:

```rust
pub fn tooltip(snap: &Snapshot, s: &Strings) -> String {
    let body = match snap.connection {
        Connection::Ready => {
            let ip = snap.device.as_ref().map(|d| d.ip.to_string()).unwrap_or_default();
            let l = &snap.config.light;
            if !l.power {
                format!("{} · {ip}", s.mode_off)
            } else if snap.config.app.mode == Mode::Ambilight {
                let mut parts = vec![s.mode_ambilight.to_owned(), format!("{} FPS", snap.config.ambilight.fps)];
                if let Some(c) = &snap.stream.capture {
                    if c.hdr {
                        parts.push("HDR".to_owned());
                    }
                    if !c.active && c.error.is_some() {
                        parts.push(s.capture_unavailable.to_owned());
                    }
                }
                parts.push(ip);
                parts.join(" · ")
            } else {
                format!("{} {} · {}% · {ip}", s.mode_static, l.color, l.brightness)
            }
        }
        _ => status_line(snap, s),
    };
    format!("Obsid Ambilight — {body}")
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --test icons_tests --test tray_tests`
Expected: PASS. Then `cargo test`: PASS.

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/i18n.rs src/ui/icons.rs src/ui/tray.rs tests/icons_tests.rs tests/tray_tests.rs
git commit -m "Add Ambilight to the tray menu, icon and tooltip"
```

---

### Task 10: Settings — Ambilight tab and diagnostics

**Files:**
- Create: `src/ui/settings/ambilight.rs`
- Modify: `src/app/state.rs` (`Tab`), `src/ui/settings/mod.rs`, `src/ui/settings/diagnostics.rs`, `src/ui/settings/device.rs` (hint text only), `src/i18n.rs`, `tests/settings_tests.rs`

**Interfaces:**
- Consumes: Tasks 5–6 (`Command::{SetMode, SetAmbilight}`, `Snapshot.stream`, `capture::outputs::{list, MonitorInfo}`), config ranges from Task 1.
- Produces: `Tab::Ambilight` (`--tab ambilight`); `ui::settings::ambilight::{Edit, show, monitor_label, aspect_of(&[MonitorInfo], &str) -> f32}`; `ui::settings::diagnostics::mode_text`; diagnostics summary lines `mode:`, `stream:`, `capture:`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/settings_tests.rs`:

```rust
#[test]
fn summary_reports_mode_stream_and_capture() {
    use obsid_ambilight::app::state::CaptureStatus;
    let mut s = Controller::new(
        Config::default(),
        vec![],
        Backoff::with_seed(Duration::from_secs(1), Duration::from_secs(1), None, 1),
    )
    .snapshot(true);
    let text = summary(&s);
    assert!(text.contains("mode: Static"), "{text}");
    assert!(text.contains("capture: not running"), "{text}");
    s.stream.streaming = true;
    s.stream.capture = Some(CaptureStatus {
        monitor: r"\\.\DISPLAY1".into(),
        adapter: "GTX".into(),
        width: 2560,
        height: 1440,
        hdr: true,
        sdr_white_nits: 240.0,
        fps: 25.0,
        active: true,
        ..CaptureStatus::default()
    });
    let text = summary(&s);
    for needle in [r"\\.\DISPLAY1", "GTX", "2560x1440", "hdr true", "240", "25.0 fps", "stream: true"] {
        assert!(text.contains(needle), "missing {needle}: {text}");
    }
}

#[test]
fn monitor_aspect_follows_the_setting() {
    use obsid_ambilight::capture::outputs::MonitorInfo;
    use obsid_ambilight::ui::settings::ambilight::aspect_of;
    let monitors = [
        MonitorInfo { name: r"\\.\DISPLAY1".into(), adapter: "A".into(), width: 2560, height: 1440, primary: true },
        MonitorInfo { name: r"\\.\DISPLAY2".into(), adapter: "B".into(), width: 1280, height: 1024, primary: false },
    ];
    assert_eq!(aspect_of(&monitors, ""), 2560.0 / 1440.0);
    assert_eq!(aspect_of(&monitors, r"\\.\DISPLAY2"), 1280.0 / 1024.0);
    assert_eq!(aspect_of(&monitors, "gone"), 16.0 / 9.0);
    assert_eq!(aspect_of(&[], ""), 16.0 / 9.0);
}

#[test]
fn ambilight_tab_is_a_command_line_tab() {
    use obsid_ambilight::app::state::Tab;
    assert_eq!(Tab::from_arg("ambilight"), Some(Tab::Ambilight));
    assert_eq!(Tab::from_index(Tab::Ambilight.index()), Tab::Ambilight);
}
```

Run: `cargo test --test settings_tests`
Expected: compile errors (`Tab::Ambilight`, `ui::settings::ambilight`).

- [ ] **Step 2: Strings**

In `src/i18n.rs`, add to `Strings`:

- after `tab_lighting`: `pub tab_ambilight: &'static str,`
- a new block after the lighting block:

```rust
    // settings: ambilight
    pub ambilight_enable: &'static str,
    pub monitor_label: &'static str,
    pub monitor_primary: &'static str,
    pub fps_label: &'static str,
    pub picture_heading: &'static str,
    pub saturation_label: &'static str,
    pub smoothing_label: &'static str,
    pub black_threshold_label: &'static str,
    pub letterbox_label: &'static str,
    pub depth_label: &'static str,
```

- at the end of the diagnostics block:

```rust
    pub mode_label: &'static str,
    pub capture_label: &'static str,
    pub capture_not_running: &'static str,
    pub hdr_label: &'static str,
    pub hdr_on: &'static str,
    pub hdr_off: &'static str,
    pub capture_fps: &'static str,
    pub send_fps: &'static str,
    pub bars_label: &'static str,
    pub bars_none: &'static str,
    pub capture_error: &'static str,
```

| Field | RU | EN |
|---|---|---|
| `tab_ambilight` | `Ambilight` | `Ambilight` |
| `ambilight_enable` | `Включить Ambilight` | `Ambilight on` |
| `monitor_label` | `Монитор` | `Monitor` |
| `monitor_primary` | `Основной` | `Primary` |
| `fps_label` | `Частота кадров` | `Frame rate` |
| `picture_heading` | `Изображение` | `Picture` |
| `saturation_label` | `Насыщенность` | `Saturation` |
| `smoothing_label` | `Плавность` | `Smoothing` |
| `black_threshold_label` | `Порог черноты` | `Black threshold` |
| `letterbox_label` | `Убирать чёрные полосы` | `Ignore black bars` |
| `depth_label` | `Глубина выборки` | `Sample depth` |
| `mode_label` | `Режим` | `Mode` |
| `capture_label` | `Захват` | `Capture` |
| `capture_not_running` | `не запущен` | `not running` |
| `hdr_label` | `HDR` | `HDR` |
| `hdr_on` | `да, белый SDR {} нит` | `yes, SDR white {} nits` |
| `hdr_off` | `нет` | `no` |
| `capture_fps` | `Захват, кадр/с` | `Capture, fps` |
| `send_fps` | `Отправка, кадр/с` | `Sent, fps` |
| `bars_label` | `Чёрные полосы` | `Black bars` |
| `bars_none` | `нет` | `none` |
| `capture_error` | `Ошибка захвата` | `Capture error` |

Also change `led_count_hint`: RU `"Используется раскладкой Ambilight."`, EN `"Used by the Ambilight layout."`.

- [ ] **Step 3: The tab id**

In `src/app/state.rs`, add `Ambilight` to `Tab` between `Lighting` and `General`, make `ALL` `[Tab; 5]` = `[Tab::Device, Tab::Lighting, Tab::Ambilight, Tab::General, Tab::Diagnostics]`, and add `Self::Ambilight => "ambilight",` to `as_arg`.

- [ ] **Step 4: The Ambilight tab**

Create `src/ui/settings/ambilight.rs`:

```rust
//! Ambilight tab: on/off, monitor, frame rate and picture settings.

use std::time::{Duration, Instant};

use eframe::egui;

use super::Backend;
use crate::app::commands::Command;
use crate::app::state::Snapshot;
use crate::capture::outputs::{self, MonitorInfo};
use crate::config::{AmbilightConfig, BLACK_THRESHOLD_MAX, DEPTH_RANGE, FPS_RANGE, Mode, SATURATION_RANGE, SMOOTHING_MAX};
use crate::i18n::Strings;

/// Local values win over the snapshot for this long after the user touched them.
const HOLD: Duration = Duration::from_millis(800);

#[derive(Default)]
pub struct Edit {
    settings: Option<AmbilightConfig>,
    touched: Option<Instant>,
    /// Listed once per window, when the tab is first shown.
    monitors: Option<Vec<MonitorInfo>>,
}

pub fn monitor_label(s: &Strings, m: &MonitorInfo) -> String {
    let name = m.name.trim_start_matches(r"\\.\");
    let primary = if m.primary { format!(" · {}", s.monitor_primary) } else { String::new() };
    format!("{name} · {}×{} · {}{primary}", m.width, m.height, m.adapter)
}

/// Width / height of the configured monitor (empty name = primary); 16:9 when unknown.
pub fn aspect_of(monitors: &[MonitorInfo], name: &str) -> f32 {
    monitors
        .iter()
        .find(|m| if name.is_empty() { m.primary } else { m.name == name })
        .filter(|m| m.height > 0)
        .map_or(16.0 / 9.0, |m| m.width as f32 / m.height as f32)
}

pub fn show(ui: &mut egui::Ui, s: &Strings, snap: &Snapshot, backend: &Backend, edit: &mut Edit) {
    let monitors = edit.monitors.get_or_insert_with(|| outputs::list().unwrap_or_default());
    let held = edit.touched.is_some_and(|t| t.elapsed() < HOLD);
    if edit.settings.is_none() || !held {
        edit.settings = Some(snap.config.ambilight.clone());
    }
    let mut a = edit.settings.clone().unwrap_or_else(|| snap.config.ambilight.clone());
    let before = a.clone();

    let mut on = snap.config.light.power && snap.config.app.mode == Mode::Ambilight;
    if ui.checkbox(&mut on, s.ambilight_enable).changed() {
        backend.send(Command::SetMode { mode: if on { Mode::Ambilight } else { Mode::Static } });
    }

    ui.add_space(8.0);
    egui::Grid::new("ambilight-main").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        ui.label(s.monitor_label);
        let selected = if a.monitor.is_empty() {
            s.monitor_primary.to_owned()
        } else {
            monitors.iter().find(|m| m.name == a.monitor).map_or_else(|| a.monitor.clone(), |m| monitor_label(s, m))
        };
        egui::ComboBox::from_id_salt("monitor").selected_text(selected).width(360.0).show_ui(ui, |ui| {
            ui.selectable_value(&mut a.monitor, String::new(), s.monitor_primary);
            for m in monitors.iter() {
                ui.selectable_value(&mut a.monitor, m.name.clone(), monitor_label(s, m));
            }
        });
        ui.end_row();
        ui.label(s.fps_label);
        ui.add(egui::Slider::new(&mut a.fps, FPS_RANGE));
        ui.end_row();
    });

    ui.add_space(8.0);
    ui.heading(s.picture_heading);
    egui::Grid::new("ambilight-picture").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
        ui.label(s.brightness_label);
        ui.add(egui::Slider::new(&mut a.brightness, 1..=100).suffix(" %"));
        ui.end_row();
        ui.label(s.saturation_label);
        ui.add(egui::Slider::new(&mut a.saturation, SATURATION_RANGE).fixed_decimals(2));
        ui.end_row();
        ui.label(s.smoothing_label);
        ui.add(egui::Slider::new(&mut a.smoothing, 0..=SMOOTHING_MAX).suffix(" %"));
        ui.end_row();
        ui.label(s.black_threshold_label);
        ui.add(egui::Slider::new(&mut a.black_threshold, 0..=BLACK_THRESHOLD_MAX).suffix(" %"));
        ui.end_row();
        ui.label(s.depth_label);
        let mut pct = (a.depth * 100.0).round() as u32;
        let range = (DEPTH_RANGE.start() * 100.0).round() as u32..=(DEPTH_RANGE.end() * 100.0).round() as u32;
        if ui.add(egui::Slider::new(&mut pct, range).suffix(" %")).changed() {
            a.depth = f64::from(pct) / 100.0;
        }
        ui.end_row();
        ui.label("");
        ui.checkbox(&mut a.letterbox, s.letterbox_label);
        ui.end_row();
    });

    // Bursts from a dragged slider are coalesced by the worker (`SetAmbilight` keeps the last).
    if a != before {
        edit.touched = Some(Instant::now());
        edit.settings = Some(a.clone());
        backend.send(Command::SetAmbilight { settings: a });
    }
}
```

- [ ] **Step 5: Wire the tab into the window**

In `src/ui/settings/mod.rs`:

1. `pub mod ambilight;` next to the other modules.
2. Start size: `.with_inner_size([900.0, 600.0])` (minimum unchanged).
3. Field `pub(crate) ambilight: ambilight::Edit,` in `SettingsApp`, initialised with `ambilight::Edit::default()`.
4. `tab_label`: `Tab::Ambilight => s.tab_ambilight,`.
5. Central panel match: `Tab::Ambilight => ambilight::show(ui, s, &snap, &self.backend, &mut self.ambilight),`.

- [ ] **Step 6: Diagnostics**

In `src/ui/settings/diagnostics.rs`:

1. Imports: `use crate::config::Mode;` and `use crate::i18n::{self, Strings};`.

2. Add:

```rust
pub fn mode_text(s: &Strings, snap: &Snapshot) -> &'static str {
    if !snap.config.light.power {
        s.mode_off
    } else if snap.config.app.mode == Mode::Ambilight {
        s.mode_ambilight
    } else {
        s.mode_static
    }
}
```

3. In `summary`, after the `manual ip` line:

```rust
    let _ = writeln!(t, "mode: {:?} (power {})", snap.config.app.mode, snap.config.light.power);
    let st = &snap.stream;
    let _ = writeln!(t, "stream: {} (highlight {}, sent {:.1} fps)", st.streaming, st.highlight, st.send_fps);
    match &st.capture {
        Some(c) => {
            let _ = writeln!(
                t,
                "capture: {} on {} {}x{} hdr {} white {:.0} nits, {:.1} fps, bars {:.2}/{:.2}, active {}, error {:?}",
                c.monitor,
                c.adapter,
                c.width,
                c.height,
                c.hdr,
                c.sdr_white_nits,
                c.fps,
                c.bars[0],
                c.bars[1],
                c.active,
                c.error
            );
        }
        None => {
            let _ = writeln!(t, "capture: not running");
        }
    }
```

4. In `show`, inside the grid after the session row:

```rust
        ui.label(s.mode_label);
        ui.label(mode_text(s, snap));
        ui.end_row();
        ui.label(s.capture_label);
        match &snap.stream.capture {
            Some(c) => {
                ui.label(format!("{} · {}×{} · {}", c.monitor, c.width, c.height, c.adapter));
                ui.end_row();
                ui.label(s.hdr_label);
                ui.label(if c.hdr {
                    i18n::fill(s.hdr_on, &[&format!("{:.0}", c.sdr_white_nits)])
                } else {
                    s.hdr_off.to_owned()
                });
                ui.end_row();
                ui.label(s.capture_fps);
                ui.label(format!("{:.1}", c.fps));
                ui.end_row();
                ui.label(s.bars_label);
                ui.label(if c.bars == [0.0, 0.0] {
                    s.bars_none.to_owned()
                } else {
                    format!("{:.0} % / {:.0} %", c.bars[0] * 100.0, c.bars[1] * 100.0)
                });
                ui.end_row();
                if let Some(e) = &c.error {
                    ui.label(s.capture_error);
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                    ui.end_row();
                }
            }
            None => {
                ui.label(s.capture_not_running);
                ui.end_row();
            }
        }
        if snap.stream.streaming {
            ui.label(s.send_fps);
            ui.label(format!("{:.1}", snap.stream.send_fps));
            ui.end_row();
        }
```

- [ ] **Step 7: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 8: Look at it**

Run the app (`cargo run`), open the settings, go to «Ambilight»: the monitors list shows both monitors with adapters, sliders move without snapping back, «Диагностика» shows the capture rows when Ambilight is on. Switch the language to EN: the tab is fully translated.

- [ ] **Step 9: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/app/state.rs src/ui/settings src/i18n.rs tests/settings_tests.rs
git commit -m "Add the Ambilight settings tab and capture diagnostics"
```

---

### Task 11: Layout editor

**Files:**
- Create: `src/ui/settings/layout_editor.rs`
- Modify: `src/ui/settings/ambilight.rs`, `src/ui/settings/mod.rs`, `src/i18n.rs`, `tests/settings_tests.rs`

**Interfaces:**
- Consumes: Task 1 (`layout::{errors, perimeter, default_counts, MAX_SEGMENTS, SegmentError, HighlightTarget}`, `Segment::{lo, hi, reversed}`), Task 5 (`Command::{SetLayout, Highlight, StopHighlight}`), Task 10 (`ambilight::{Edit, show, aspect_of}`).
- Produces: `ui::settings::layout_editor::{SNAP_DISTANCE, snap_point, to_screen, from_screen, fit, free_range, unused_count, Edit, Edit::leave, show}`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/settings_tests.rs`:

```rust
mod layout_editor {
    use eframe::egui::{Rect, pos2, vec2};
    use obsid_ambilight::config::Segment;
    use obsid_ambilight::ui::settings::layout_editor::{fit, free_range, from_screen, snap_point, to_screen, unused_count};

    fn seg(from: usize, to: usize) -> Segment {
        Segment { from, to, start: [0.0, 0.0], end: [1.0, 0.0] }
    }

    #[test]
    fn ends_snap_to_the_5_percent_grid_when_close() {
        assert_eq!(snap_point([0.012, 0.5]), [0.0, 0.5]);
        assert_eq!(snap_point([0.03, 0.27]), [0.03, 0.27], "too far from a grid line");
        assert_eq!(snap_point([0.991, 1.2]), [1.0, 1.0], "clamped, then snapped");
    }

    #[test]
    fn screen_mapping_round_trips() {
        let r = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 225.0));
        assert_eq!(to_screen(r, [0.5, 1.0]), pos2(210.0, 245.0));
        assert_eq!(from_screen(r, pos2(210.0, 245.0)), [0.5, 1.0]);
        assert_eq!(from_screen(r, pos2(-50.0, 999.0)), [0.0, 1.0], "outside points are clamped");
    }

    #[test]
    fn schematic_keeps_the_monitor_aspect() {
        let r = fit(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)), 16.0 / 9.0);
        assert_eq!((r.width(), r.height()), (400.0, 225.0));
        assert_eq!(r.center(), pos2(200.0, 150.0));
    }

    #[test]
    fn new_segments_take_the_first_free_leds() {
        assert_eq!(free_range(&[], 60), Some((0, 9)));
        assert_eq!(free_range(&[seg(0, 9), seg(20, 15)], 60), Some((10, 14)));
        assert_eq!(free_range(&[seg(0, 59)], 60), None);
        assert_eq!(unused_count(&[seg(0, 9), seg(20, 15)], 60), 44);
    }
}
```

Run: `cargo test --test settings_tests`
Expected: compile error — `layout_editor` does not exist.

- [ ] **Step 2: Strings**

Add to `Strings` (new block after the Ambilight block) and to `RU`/`EN`:

| Field | RU | EN |
|---|---|---|
| `layout_heading` | `Раскладка` | `Layout` |
| `layout_hint` | `Вид спереди. Перетаскивайте концы отрезков.` | `Front view. Drag the ends of the lines.` |
| `seg_from` | `с` | `from` |
| `seg_to` | `по` | `to` |
| `seg_start` | `начало (x, y)` | `start (x, y)` |
| `seg_end` | `конец (x, y)` | `end (x, y)` |
| `seg_add` | `Добавить сегмент` | `Add segment` |
| `seg_delete` | `Удалить` | `Delete` |
| `seg_reverse` | `Развернуть` | `Reverse` |
| `perimeter_sides` | `Слева, сверху, справа, снизу:` | `Left, top, right, bottom:` |
| `perimeter_btn` | `Шаблон: периметр` | `Template: perimeter` |
| `perimeter_too_many` | `Сумма больше числа светодиодов ({})` | `The sum exceeds the LED count ({})` |
| `find_led` | `Поиск диода` | `Find LED` |
| `show_on_strip` | `Показывать выбранный сегмент на ленте` | `Show the selected segment on the strip` |
| `highlight_needs_strip` | `Подсветка недоступна: лента не подключена` | `Highlight unavailable: the strip is not connected` |
| `no_segments` | `Сегментов нет — светодиоды не горят` | `No segments: the LEDs stay dark` |
| `unused_leds` | `Не используется светодиодов: {}` | `Unused LEDs: {}` |
| `seg_err_range` | `номер вне 0…{}` | `index outside 0…{}` |
| `seg_err_point` | `точка вне экрана` | `point outside the screen` |
| `seg_err_overlap` | `пересекается с сегментом {}` | `overlaps segment {}` |

- [ ] **Step 3: The editor**

Create `src/ui/settings/layout_editor.rs`:

```rust
//! Layout editor: segments drawn over a screen schematic (front view), with
//! draggable ends, a segment list, the perimeter template and strip highlights.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::Backend;
use crate::ambilight::layout::{self, HighlightTarget, MAX_SEGMENTS, SegmentError};
use crate::app::commands::Command;
use crate::app::state::{Connection, Snapshot};
use crate::config::Segment;
use crate::i18n::{self, Strings};

/// Grid lines per screen side (5 %).
const GRID_STEPS: f64 = 20.0;
/// A dragged end snaps to a grid line closer than this.
pub const SNAP_DISTANCE: f64 = 0.015;
/// Highlights are repeated this often while shown; the worker drops them after 3 s.
const RESEND: Duration = Duration::from_secs(1);
/// Local edits win over the snapshot for this long.
const HOLD: Duration = Duration::from_millis(800);
/// Free LEDs taken by a new segment.
const NEW_SEGMENT_LEDS: usize = 10;
const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0xE5, 0x48, 0x4D),
    Color32::from_rgb(0x3D, 0xB2, 0x5A),
    Color32::from_rgb(0x3D, 0x7E, 0xE5),
    Color32::from_rgb(0xF2, 0xB3, 0x3D),
    Color32::from_rgb(0xA8, 0x5C, 0xE6),
    Color32::from_rgb(0x2C, 0xC5, 0xC9),
    Color32::from_rgb(0xE6, 0x6F, 0xB4),
    Color32::from_rgb(0x9A, 0xA0, 0xA6),
];

/// Clamps to the screen and snaps each coordinate to a 5 % grid line when close.
pub fn snap_point(p: [f64; 2]) -> [f64; 2] {
    p.map(|v| {
        let v = v.clamp(0.0, 1.0);
        let g = (v * GRID_STEPS).round() / GRID_STEPS;
        if (v - g).abs() <= SNAP_DISTANCE { g } else { v }
    })
}

pub fn to_screen(rect: Rect, p: [f64; 2]) -> Pos2 {
    pos2(rect.left() + p[0] as f32 * rect.width(), rect.top() + p[1] as f32 * rect.height())
}

pub fn from_screen(rect: Rect, pos: Pos2) -> [f64; 2] {
    [
        f64::from(((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0)),
        f64::from(((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0)),
    ]
}

/// Largest rectangle of the given width/height ratio centred in `avail`.
pub fn fit(avail: Rect, aspect: f32) -> Rect {
    let aspect = aspect.max(0.1);
    let (w, h) = if avail.width() / avail.height() > aspect {
        (avail.height() * aspect, avail.height())
    } else {
        (avail.width(), avail.width() / aspect)
    };
    Rect::from_center_size(avail.center(), vec2(w, h))
}

fn used(segments: &[Segment], led: usize) -> bool {
    segments.iter().any(|s| (s.lo()..=s.hi()).contains(&led))
}

/// First run of free LEDs (at most 10) for a new segment.
pub fn free_range(segments: &[Segment], led_count: usize) -> Option<(usize, usize)> {
    let start = (0..led_count).find(|&i| !used(segments, i))?;
    let end = (start..led_count).take_while(|&i| !used(segments, i)).take(NEW_SEGMENT_LEDS).last()?;
    Some((start, end))
}

pub fn unused_count(segments: &[Segment], led_count: usize) -> usize {
    (0..led_count).filter(|&i| !used(segments, i)).count()
}

fn error_text(s: &Strings, e: SegmentError) -> String {
    match e {
        SegmentError::OutOfRange { led_count } => {
            i18n::fill(s.seg_err_range, &[&led_count.saturating_sub(1).to_string()])
        }
        SegmentError::BadPoint => s.seg_err_point.to_owned(),
        SegmentError::Overlap(i) => i18n::fill(s.seg_err_overlap, &[&(i + 1).to_string()]),
    }
}

#[derive(Default)]
pub struct Edit {
    segments: Option<Vec<Segment>>,
    touched: Option<Instant>,
    selected: Option<usize>,
    find_led: bool,
    led: usize,
    show_segment: bool,
    perimeter: Option<[usize; 4]>,
    /// Last highlight sent and when.
    sent: Option<(HighlightTarget, Instant)>,
}

impl Edit {
    /// Ends a strip highlight started here (tab switch, nothing to show).
    pub fn leave(&mut self, backend: &Backend) {
        if self.sent.take().is_some() {
            backend.send(Command::StopHighlight);
        }
    }
}

fn point_editor(ui: &mut egui::Ui, p: &mut [f64; 2]) {
    ui.horizontal(|ui| {
        for c in p.iter_mut() {
            ui.add(egui::DragValue::new(c).range(0.0..=1.0).speed(0.005).fixed_decimals(2));
        }
    });
}

pub fn show(ui: &mut egui::Ui, s: &Strings, snap: &Snapshot, backend: &Backend, edit: &mut Edit, aspect: f32) {
    let led_count = snap.config.device.led_count;
    let saved = &snap.config.ambilight.segments;
    let held = edit.touched.is_some_and(|t| t.elapsed() < HOLD);
    // A local layout with errors is never sent, so it must not be overwritten either.
    let invalid = edit.segments.as_ref().is_some_and(|l| layout::errors(l, led_count).iter().any(Option::is_some));
    if edit.segments.is_none() || (!held && !invalid) {
        edit.segments = Some(saved.clone());
    }
    let mut segs = edit.segments.clone().unwrap_or_default();
    let before = segs.clone();
    if edit.selected.is_some_and(|i| i >= segs.len()) {
        edit.selected = None;
    }

    ui.heading(s.layout_heading);
    ui.weak(s.layout_hint);

    // Schematic with draggable ends.
    let width = ui.available_width().min(560.0);
    let (response, painter) = ui.allocate_painter(vec2(width, width * 0.6), Sense::hover());
    let screen = fit(response.rect.shrink(14.0), aspect);
    painter.rect_stroke(screen, 4.0, Stroke::new(1.5, ui.visuals().text_color()), StrokeKind::Middle);
    for (i, seg) in segs.iter_mut().enumerate() {
        let color = PALETTE[i % PALETTE.len()];
        let w = if edit.selected == Some(i) { 4.0 } else { 2.5 };
        let (a, b) = (to_screen(screen, seg.start), to_screen(screen, seg.end));
        painter.line_segment([a, b], Stroke::new(w, color));
        painter.arrow(a + (b - a) * 0.4, (b - a) * 0.2, Stroke::new(w, color));
        painter.text(a, Align2::RIGHT_BOTTOM, seg.from.to_string(), FontId::proportional(11.0), color);
        painter.text(b, Align2::LEFT_TOP, seg.to.to_string(), FontId::proportional(11.0), color);
        for (which, end) in [&mut seg.start, &mut seg.end].into_iter().enumerate() {
            let pos = to_screen(screen, *end);
            let id = ui.id().with(("segment-end", i, which));
            let r = ui.interact(Rect::from_center_size(pos, vec2(16.0, 16.0)), id, Sense::drag());
            if r.dragged()
                && let Some(p) = r.interact_pointer_pos()
            {
                *end = snap_point(from_screen(screen, p));
                edit.selected = Some(i);
            }
            painter.circle_filled(pos, if r.hovered() || r.dragged() { 6.0 } else { 4.0 }, color);
        }
    }

    // Segment list.
    ui.add_space(6.0);
    let errors = layout::errors(&segs, led_count);
    let mut remove = None;
    egui::Grid::new("segments").num_columns(7).striped(true).spacing([10.0, 4.0]).show(ui, |ui| {
        for h in ["", s.seg_from, s.seg_to, s.seg_start, s.seg_end, "", ""] {
            ui.label(h);
        }
        ui.end_row();
        for (i, seg) in segs.iter_mut().enumerate() {
            let label = egui::RichText::new(format!("#{}", i + 1)).color(PALETTE[i % PALETTE.len()]).strong();
            if ui.selectable_label(edit.selected == Some(i), label).clicked() {
                edit.selected = Some(i);
            }
            ui.add(egui::DragValue::new(&mut seg.from).range(0..=led_count - 1));
            ui.add(egui::DragValue::new(&mut seg.to).range(0..=led_count - 1));
            point_editor(ui, &mut seg.start);
            point_editor(ui, &mut seg.end);
            ui.horizontal(|ui| {
                if ui.small_button(s.seg_reverse).clicked() {
                    *seg = seg.reversed();
                }
                if ui.small_button(s.seg_delete).clicked() {
                    remove = Some(i);
                }
            });
            match errors[i] {
                Some(e) => {
                    ui.colored_label(ui.visuals().error_fg_color, error_text(s, e));
                }
                None => {
                    ui.label("");
                }
            }
            ui.end_row();
        }
    });
    if let Some(i) = remove {
        segs.remove(i);
        edit.selected = None;
    }

    ui.horizontal(|ui| {
        let free = free_range(&segs, led_count).filter(|_| segs.len() < MAX_SEGMENTS);
        if ui.add_enabled(free.is_some(), egui::Button::new(s.seg_add)).clicked()
            && let Some((from, to)) = free
        {
            segs.push(Segment { from, to, start: [0.3, 0.5], end: [0.7, 0.5] });
            edit.selected = Some(segs.len() - 1);
        }
        if segs.is_empty() {
            ui.weak(s.no_segments);
        } else {
            ui.weak(i18n::fill(s.unused_leds, &[&unused_count(&segs, led_count).to_string()]));
        }
    });

    ui.horizontal(|ui| {
        let counts = edit.perimeter.get_or_insert_with(|| layout::default_counts(led_count));
        ui.label(s.perimeter_sides);
        for c in counts.iter_mut() {
            ui.add(egui::DragValue::new(c).range(0..=led_count));
        }
        let total: usize = counts.iter().sum();
        if ui.add_enabled(total > 0 && total <= led_count, egui::Button::new(s.perimeter_btn)).clicked() {
            segs = layout::perimeter(*counts);
            edit.selected = None;
        }
        if total > led_count {
            ui.colored_label(ui.visuals().error_fg_color, i18n::fill(s.perimeter_too_many, &[&led_count.to_string()]));
        }
    });

    if segs != before {
        edit.touched = Some(Instant::now());
        if layout::errors(&segs, led_count).iter().all(Option::is_none) {
            backend.send(Command::SetLayout { segments: segs.clone() });
        }
        edit.segments = Some(segs);
    }

    // Strip highlight.
    ui.separator();
    ui.horizontal(|ui| {
        ui.checkbox(&mut edit.find_led, s.find_led);
        ui.add_enabled(edit.find_led, egui::Slider::new(&mut edit.led, 0..=led_count - 1));
    });
    ui.checkbox(&mut edit.show_segment, s.show_on_strip);
    let target = if edit.find_led {
        Some(HighlightTarget::Led(edit.led.min(led_count - 1)))
    } else if edit.show_segment {
        edit.selected.filter(|i| *i < saved.len()).map(HighlightTarget::Segment)
    } else {
        None
    };
    let ready = snap.connection == Connection::Ready;
    if (edit.find_led || edit.show_segment) && !ready {
        ui.colored_label(ui.visuals().warn_fg_color, s.highlight_needs_strip);
    }
    match target.filter(|_| ready) {
        Some(t) => {
            if edit.sent.is_none_or(|(prev, at)| prev != t || at.elapsed() >= RESEND) {
                backend.send(Command::Highlight { target: t });
                edit.sent = Some((t, Instant::now()));
            }
            ui.ctx().request_repaint_after(RESEND);
        }
        None => edit.leave(backend),
    }
}
```

- [ ] **Step 4: Hook it into the tab and the window**

1. `src/ui/settings/mod.rs`: add `pub mod layout_editor;`; at the start of `SettingsApp::ui`, after the tab request is taken, stop a highlight when the tab is left:

```rust
        if self.tab != Tab::Ambilight {
            self.ambilight.layout.leave(&self.backend);
        }
```

(Place it after the left panel too, so a click on another tab takes effect in the same frame: simplest is right before `egui::CentralPanel::default()`.)

2. `src/ui/settings/ambilight.rs`: add the field `pub(crate) layout: super::layout_editor::Edit,` to `Edit`, and at the end of `show`:

```rust
    ui.separator();
    let aspect = aspect_of(edit.monitors.as_deref().unwrap_or_default(), &snap.config.ambilight.monitor);
    super::layout_editor::show(ui, s, snap, backend, &mut edit.layout, aspect);
```

- [ ] **Step 5: Run the tests**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Try it on the strip**

`cargo run`, open «Ambilight»:
- «Поиск диода» + slider: one white LED moves along the strip, dim red behind it, dim green ahead; the window closed → the strip returns to its mode within ~3 s.
- Drag a segment end: the line follows and snaps near grid lines; the strip (in Ambilight mode) follows the new layout within a second.
- Enter `from = 999`: the row shows the range error and nothing is sent.
- «Шаблон: периметр» replaces the layout.

- [ ] **Step 7: Lint and commit**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
git add src/ui/settings src/i18n.rs tests/settings_tests.rs
git commit -m "Add the Ambilight layout editor with strip highlights"
```

---

### Task 12: Documentation, hardware checklist, final verification

**Files:**
- Modify: `README.md`, `docs/testing.md`, `docs/protocol.md`

**Interfaces:**
- Consumes: everything above.
- Produces: Stage 2 documentation and recorded hardware results.

- [ ] **Step 1: README**

- Status table: replace the last row with

```markdown
| Stage 2 — Ambilight: DXGI capture (HDR, letterbox), segment layout editor, streaming, resume | ✅ implemented, hardware checklist in docs/testing.md |
| Windows lifecycle (autostart, shutdown, sleep) | Stage 3 |
```

- «Running the app»: add a bullet: «Ambilight: tray → «Ambilight», or Settings → «Ambilight». Build the layout there: «Поиск диода» shows where each LED is; draw one segment per straight run of the strip.»
- Layout block: add `ambilight/      layout, colour maths, letterbox, smoothing, pipeline (no Win32)` and `capture/        DXGI Desktop Duplication, monitors, SDR white level, capture thread`.
- Settings line: mention `[ambilight]` and `[[ambilight.segments]]`.

- [ ] **Step 2: Stage 2 hardware checklist**

Append to `docs/testing.md`:

```markdown
## Stage 2 on hardware

Run `target\release\obsid-ambilight.exe`. After each row, check that Apple Home
still controls the strip. GPU tests: `cargo test --test capture_tests -- --ignored`.

| # | Scenario | Expected | Result |
|---|---|---|---|
| A1 | `obsid-poc --ip <ip> --leds 60 direct --pattern ends`, then `--pattern chase` | Real LED count and where index 0 is | ⏳ |
| A2 | Settings → Ambilight → «Поиск диода» along the whole strip | One white LED, red behind, green ahead | ⏳ |
| A3 | Build the two-loop spiral layout (outer and inner loops as segments) | Both loops follow the screen edges | ⏳ |
| A4 | Tray → «Ambilight» on the desktop, then «Статика» | Stream starts within ~1 s; static colour returns | ⏳ |
| A5 | Full-screen game (exclusive and borderless) | Colours follow the game | ⏳ |
| A6 | 21:9 film in a 16:9 player | Bars ignored after ~1 s, colours from the picture | ⏳ |
| A7 | Toggle Windows HDR while Ambilight runs | Short pause, then colours not washed out; Diagnostics shows HDR and SDR white | ⏳ |
| A8 | UAC prompt / Win+L | Yellow dot, last colours held, capture resumes afterwards | ⏳ |
| A9 | 20, 25, 30 FPS for 10 min each | Smooth; Diagnostics send FPS ≤ setting; Apple Home OK | ⏳ |
| A10 | Ambilight brightness 30 % vs 100 % | Visibly dimmer; note whether `set_bright` affects direct mode | ⏳ |
| A11 | Unplug the strip during Ambilight, plug back | Offline, retries; Ambilight resumes by itself | ⏳ |
| A12 | Exit with «Оставить цвет» / «Выключить ленту» from Ambilight | Static colour / off | ⏳ |
| A13 | Kill the process in Task Manager during Ambilight; wait 30 s | Record what the strip shows (open question from Stage 0) | ⏳ |
| A14 | Restart the app in Ambilight mode | Ambilight resumes automatically | ⏳ |
| A15 | CPU and RAM of the background process in Ambilight, 25 FPS, 1440p, 5 min | ≤ 3 % CPU average, ≤ 60 MB RAM | ⏳ |
| A16 | CPU and RAM in Static after leaving Ambilight | Stage 1 targets (≤ 35 MB, ≤ 1 % CPU) | ⏳ |
```

- [ ] **Step 3: Protocol notes**

In `docs/protocol.md`, «Open questions», add:

```markdown
- Whether `set_bright` scales `update_leds` colours in direct mode (the app pins
  hardware brightness to 100 and scales frames itself; checklist A10).
- Stability of 25–30 FPS streams and of the Apple Home hand-over after long
  Ambilight sessions (checklist A9).
```

- [ ] **Step 4: Full verification**

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test capture_tests -- --ignored
cargo build --release
```

Expected: all green; `target/release/obsid-ambilight.exe` built.

- [ ] **Step 5: Hardware run with the user**

Walk through A1–A16 with the user (the strip, Apple Home and the eyes are theirs). Record each result in the table, with the date. For A15/A16 measure with:

```powershell
Get-Process obsid-ambilight | Select-Object WorkingSet64, PrivateMemorySize64, CPU
```

taken 5 minutes apart (CPU delta / 300 s / logical cores = average share).

- [ ] **Step 6: Commit**

```bash
git add README.md docs/testing.md docs/protocol.md
git commit -m "Document Stage 2 and its hardware checklist"
```

After the user confirms the checklist, a separate commit records the results and marks Stage 2 verified in README (as for Stage 1).
