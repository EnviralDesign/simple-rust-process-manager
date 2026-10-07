#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod config;
mod log_classification;
mod platform;
mod process_manager;
mod rest_api;
mod ui;

fn main() -> eframe::Result<()> {
    if normalize_config_from_args() {
        return Ok(());
    }

    platform::initialize();
    ui::run()
}

fn normalize_config_from_args() -> bool {
    let mut args = std::env::args_os().skip(1);
    let Some(command) = args.next() else {
        return false;
    };

    if command != "--normalize-config" && command != "--compact-config" {
        return false;
    }

    let Some(path) = args.next() else {
        eprintln!("Usage: simple-rust-process-manager --normalize-config <processes.json>");
        std::process::exit(2);
    };

    match config::AppConfig::load_from_path(&path).and_then(|config| config.save_to_path(&path)) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("Failed to normalize config: {err}");
            std::process::exit(1);
        }
    }
}
