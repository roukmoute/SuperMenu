#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod about;
mod app_icons;
mod catalog;
mod cleanup;
mod config;
mod core;
mod installer;
mod launch_feedback;
mod native;
mod packages;
mod system_info;
mod timer;
mod timer_alert;
mod timer_runtime;
mod ui;
mod uninstall_inventory;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = installer::run_from_args(&args) {
        std::process::exit(code);
    }
    if args.iter().any(|a| a == "--benchmark") {
        let report = core::benchmark();
        if let Some(pos) = args.iter().position(|a| a == "--report") {
            if let Some(path) = args.get(pos + 1) {
                let _ = std::fs::write(path, &report);
            }
        }
        println!("{report}");
        return;
    }
    if let Err(error) = ui::run(&args) {
        ui::show_error(&error);
    }
}
