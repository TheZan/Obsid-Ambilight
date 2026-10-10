//! Ambilight tab: on/off, monitor, frame rate and picture settings.

use std::time::{Duration, Instant};

use eframe::egui;

use super::Backend;
use crate::app::commands::Command;
use crate::app::state::Snapshot;
use crate::capture::outputs::{self, MonitorInfo};
use crate::config::{
    AmbilightConfig, BLACK_THRESHOLD_MAX, DEPTH_RANGE, FPS_RANGE, Mode, SATURATION_RANGE, SMOOTHING_MAX,
};
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
