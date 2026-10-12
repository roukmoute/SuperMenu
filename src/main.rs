#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod about;
mod app_icons;
mod brand_icon;
mod catalog;
mod cleanup;
mod config;
mod core;
mod i18n;
mod installer;
mod language_dialog;
mod launch_feedback;
mod native;
mod packages;
mod process_tree;
mod setup_control;
mod system_info;
mod timer;
mod timer_alert;
mod timer_runtime;
mod ui;
mod unicode;
mod uninstall_inventory;

/// Written to the `--report` path when a smoke test fails before its own report.
#[derive(serde::Serialize)]
struct SmokeFailure<'a> {
    error: &'a str,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = setup_control::run_from_args(&args) {
        std::process::exit(code);
    }
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
        if args
            .iter()
            .any(|argument| matches!(argument.as_str(), "--smoke-test" | "--smoke-fixture"))
        {
            if let Some(path) = args
                .iter()
                .position(|argument| argument == "--report")
                .and_then(|position| args.get(position + 1))
            {
                let _ = serde_json::to_string(&SmokeFailure { error: &error })
                    .map(|json| std::fs::write(path, json));
            }
            eprintln!("{error}");
            std::process::exit(1);
        }
        ui::show_error(&error);
    }
}
