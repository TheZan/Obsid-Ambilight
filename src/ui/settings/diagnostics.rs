//! Diagnostics tab and the copyable plain-text summary (no secrets: the
//! snapshot never contains the session token).

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::app::state::Snapshot;
use crate::config::Mode;
use crate::i18n::{self, Strings};
use crate::platform::{self, paths};

pub fn mode_text(s: &Strings, snap: &Snapshot) -> &'static str {
    if !snap.config.light.power {
        s.mode_off
    } else if snap.config.app.mode == Mode::Ambilight {
        s.mode_ambilight
    } else {
        s.mode_static
    }
}

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
    let _ = writeln!(t, "mode: {:?} (power {})", snap.config.app.mode, snap.config.light.power);
    let paused = if snap.dark.is_empty() {
        "none".to_owned()
    } else {
        snap.dark.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>().join(", ")
    };
    let _ = writeln!(t, "paused: {paused}");
    let _ = writeln!(t, "autostart: {}", snap.config.startup.autostart);
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
        ui.label(s.mode_label);
        ui.label(mode_text(s, snap));
        ui.end_row();
        ui.label(s.pause_label);
        ui.label(if snap.dark.is_empty() { s.pause_none.to_owned() } else { i18n::dark_reasons(s, &snap.dark) });
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
        if ui.button(s.open_logs).clicked()
            && let Ok(dir) = paths::logs_dir()
        {
            platform::open_folder(&dir);
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
