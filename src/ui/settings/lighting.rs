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
            let r = ui
                .add(egui::Button::new("").fill(to_color32(*c)).min_size(egui::vec2(28.0, 28.0)))
                .on_hover_text(c.to_string());
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
