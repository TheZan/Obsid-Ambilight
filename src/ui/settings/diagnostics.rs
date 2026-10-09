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
