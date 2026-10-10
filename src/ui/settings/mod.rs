//! Settings window: a separate process (`--settings`) that talks to the
//! background process over the pipe and exits when the window closes.

pub mod ambilight;
pub mod device;
pub mod diagnostics;
pub mod general;
pub mod lighting;

use std::process::ExitCode;
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
use crate::ipc::{Request, Response, TabRequest};
use crate::platform::{self, paths};
use crate::ui::icons;

const POLL: Duration = Duration::from_millis(250);

pub fn run(tab: Tab) -> ExitCode {
    if let Ok(logs) = paths::logs_dir() {
        let _ = crate::diagnostics::init_logging(&logs.join("settings.log"), tracing::Level::INFO);
    }
    let icon = egui::IconData {
        rgba: icons::render(icons::IconKind::Lit(crate::protocol::Rgb::new(0xF5, 0xD4, 0xA1)), 64),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Obsid Ambilight")
            .with_inner_size([900.0, 600.0])
            .with_min_inner_size([560.0, 400.0])
            .with_icon(icon),
        ..Default::default()
    };
    let result =
        eframe::run_native("Obsid Ambilight", options, Box::new(move |cc| Ok(Box::new(SettingsApp::new(cc, tab)))));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(%e, "settings window failed");
            ExitCode::FAILURE
        }
    }
}

/// Connection to the background process, run on its own thread. The
/// snapshot is `None` while the background process is unreachable.
pub struct Backend {
    snapshot: Arc<Mutex<Option<Snapshot>>>,
    tx: Sender<Command>,
    /// Tab the tray asked an already open window to switch to.
    tab: TabRequest,
}

impl Backend {
    fn spawn(ctx: egui::Context) -> Self {
        let snapshot = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::channel();
        let s = Arc::clone(&snapshot);
        let tab = TabRequest::default();
        let t = tab.clone();
        thread::Builder::new()
            .name("settings-ipc".into())
            .spawn(move || link_loop(&ctx, &s, &t, &rx))
            .expect("spawn settings IPC thread");
        Self { snapshot, tx, tab }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    fn snapshot(&self) -> Option<Snapshot> {
        self.snapshot.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

fn link_loop(ctx: &egui::Context, snap: &Mutex<Option<Snapshot>>, tab: &TabRequest, rx: &Receiver<Command>) {
    let Ok(name) = pipe::pipe_name() else { return };
    loop {
        let mut client = match Client::connect(&name, Duration::from_millis(500)) {
            Ok(c) => c,
            Err(_) => {
                *snap.lock().unwrap_or_else(PoisonError::into_inner) = None;
                ctx.request_repaint();
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
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
            match client.request(&Request::TakeTab) {
                Ok(Response::Tab { tab: Some(t) }) => {
                    tab.set(t);
                    ctx.request_repaint();
                }
                Ok(_) => {}
                Err(_) => break,
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
    pub(crate) ambilight: ambilight::Edit,
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
            ambilight: ambilight::Edit::default(),
            device: device::Edit::default(),
            copied_at: None,
            started_backend: false,
        }
    }

    fn tab_label(s: &Strings, tab: Tab) -> &'static str {
        match tab {
            Tab::Device => s.tab_device,
            Tab::Lighting => s.tab_lighting,
            Tab::Ambilight => s.tab_ambilight,
            Tab::General => s.tab_general,
            Tab::Diagnostics => s.tab_diagnostics,
        }
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let snap = self.backend.snapshot();
        if let Some(tab) = self.backend.tab.take() {
            self.tab = tab;
        }
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
                if ui.add_enabled(!self.started_backend, egui::Button::new(s.start_backend)).clicked()
                    && let Ok(exe) = std::env::current_exe()
                {
                    let _ = std::process::Command::new(exe).spawn();
                    self.started_backend = true;
                }
                return;
            };
            egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                Tab::Device => device::show(ui, s, &snap, &self.backend, &mut self.device),
                Tab::Lighting => lighting::show(ui, s, &snap, &self.backend, &mut self.lighting),
                Tab::Ambilight => ambilight::show(ui, s, &snap, &self.backend, &mut self.ambilight),
                Tab::General => general::show(ui, s, &snap, &self.backend),
                Tab::Diagnostics => diagnostics::show(ui, s, &snap, &mut self.copied_at),
            });
        });
    }
}
