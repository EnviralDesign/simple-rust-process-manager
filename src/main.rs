#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod config;
mod log_classification;
mod process_manager;
mod rest_api;
mod ui;

fn main() -> eframe::Result<()> {
    ui::run()
}
