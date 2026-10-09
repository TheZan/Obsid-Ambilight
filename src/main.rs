// Release builds have no console window; debug builds keep one for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

use obsid_ambilight::app::state::Tab;
use obsid_ambilight::ui;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--settings") {
        let tab = args
            .iter()
            .position(|a| a == "--tab")
            .and_then(|i| args.get(i + 1))
            .and_then(|t| Tab::from_arg(t))
            .unwrap_or_default();
        return ui::settings::run(tab);
    }
    ui::tray::run()
}
