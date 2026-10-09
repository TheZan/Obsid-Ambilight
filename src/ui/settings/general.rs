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
