#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod catalog;
mod config;
mod core;
mod native;
mod ui;

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
