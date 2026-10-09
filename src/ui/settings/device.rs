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
    /// A search was started from this window; only then is an empty list news.
    searched: bool,
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
            edit.searched = true;
            backend.send(Command::Discover);
        }
        if snap.searching {
            ui.spinner();
            ui.label(s.searching);
        }
    });
    if snap.discovered.is_empty() && !snap.searching && edit.searched {
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
    let text =
        edit.manual_ip.get_or_insert_with(|| snap.config.device.manual_ip.map(|ip| ip.to_string()).unwrap_or_default());
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
