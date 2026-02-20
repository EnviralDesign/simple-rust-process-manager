//! Simple Rust Process Manager
//! A lightweight GUI tool for managing development processes and Docker containers.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod config;
mod process_manager;

use config::{AppConfig, ProcessConfig, ProcessType};
use dioxus::prelude::*;
use process_manager::{ProcessManager, ProcessStatus};
use std::sync::Arc;

// CSS Styles embedded in the app
const STYLES: &str = r#"
:root {
    --bg-primary: #0c1117;
    --bg-secondary: #111722;
    --bg-tertiary: #161d2a;
    --bg-hover: #1c2534;
    --accent-primary: #3b82f6;
    --accent-secondary: #93c5fd;
    --accent-glow: rgba(59, 130, 246, 0.14);
    --text-primary: #e5e7eb;
    --text-secondary: #b7c0cd;
    --text-muted: #8892a0;
    --success: #16a34a;
    --success-soft: rgba(22, 163, 74, 0.2);
    --warning: #d97706;
    --warning-soft: rgba(217, 119, 6, 0.2);
    --danger: #dc2626;
    --danger-soft: rgba(220, 38, 38, 0.2);
    --border: #253043;
    --border-light: #344259;
    --radius: 7px;
    --radius-lg: 10px;
    --shadow: 0 10px 28px rgba(0, 0, 0, 0.35);
    --transition: background-color 0.14s ease, border-color 0.14s ease, color 0.14s ease, box-shadow 0.14s ease;
}

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: 'Segoe UI Variable Text', 'Segoe UI', system-ui, -apple-system, sans-serif;
    background: var(--bg-primary);
    color: var(--text-primary);
    overflow: hidden;
    font-size: 13px;
}

.app-container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: linear-gradient(180deg, #0d131b 0%, #0b1118 100%);
}

/* Header */
.header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.02);
    z-index: 100;
}

.header-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 15px;
    font-weight: 600;
    color: var(--text-primary);
}

.header-title-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    height: 22px;
    padding: 0 6px;
    border: 1px solid var(--border-light);
    border-radius: 6px;
    background: var(--bg-tertiary);
    font-size: 10px;
    letter-spacing: 0.5px;
    font-family: 'Cascadia Mono', 'Consolas', monospace;
    color: var(--text-secondary);
}

.header-actions {
    display: flex;
    gap: 6px;
}

.header-separator {
    color: var(--text-muted);
    margin: 0 2px;
}

.stack-name {
    color: var(--text-secondary);
    cursor: pointer;
    padding: 2px 6px;
    border-radius: var(--radius);
    transition: var(--transition);
}

.stack-name:hover {
    background: var(--bg-tertiary);
    color: var(--accent-secondary);
}

.edit-icon {
    font-size: 11px;
    opacity: 0.5;
    transition: var(--transition);
    letter-spacing: 0.2px;
}

.stack-name:hover .edit-icon {
    opacity: 1;
}

.stack-name-input {
    background: var(--bg-primary);
    border: 1px solid var(--border-light);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 15px;
    font-weight: 600;
    padding: 4px 8px;
    width: 190px;
    outline: none;
}

.btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 10px;
    border: none;
    border-radius: var(--radius);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    transition: var(--transition);
    background: var(--bg-tertiary);
    color: var(--text-primary);
    border: 1px solid var(--border);
}

.btn:hover {
    background: var(--bg-hover);
    border-color: var(--border-light);
}

.btn-primary {
    background: rgba(59, 130, 246, 0.18);
    border-color: rgba(96, 165, 250, 0.5);
    color: #bfdbfe;
}

.btn-primary:hover {
    background: rgba(59, 130, 246, 0.26);
    box-shadow: 0 0 0 3px var(--accent-glow);
}

.btn-success {
    background: var(--success-soft);
    border-color: rgba(34, 197, 94, 0.45);
    color: #86efac;
}

.btn-success:hover {
    background: rgba(22, 163, 74, 0.28);
}

.btn-danger {
    background: var(--danger-soft);
    border-color: rgba(248, 113, 113, 0.45);
    color: #fca5a5;
}

.btn-danger:hover {
    background: rgba(220, 38, 38, 0.28);
}

.btn-warning {
    background: var(--warning-soft);
    border-color: rgba(251, 191, 36, 0.45);
    color: #fcd34d;
}

.btn-icon {
    padding: 6px;
    min-width: 30px;
    justify-content: center;
}

.btn-small {
    padding: 4px 8px;
    font-size: 11px;
}

/* Main Layout */
.main-content {
    display: flex;
    flex: 1;
    overflow: hidden;
}

/* Sidebar */
.sidebar {
    width: 248px;
    background: var(--bg-secondary);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
}

.sidebar-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
}

.sidebar-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.8px;
    color: var(--text-muted);
}

.process-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px;
}

.process-list::-webkit-scrollbar {
    width: 6px;
}

.process-list::-webkit-scrollbar-track {
    background: transparent;
}

.process-list::-webkit-scrollbar-thumb {
    background: var(--border-light);
    border-radius: 3px;
}

.process-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 10px;
    margin-bottom: 3px;
    border-radius: var(--radius);
    cursor: pointer;
    transition: var(--transition);
    border: 1px solid var(--bg-secondary);
}

.process-item:hover {
    background: rgba(148, 163, 184, 0.07);
    border-color: var(--border);
}

.process-item.active {
    background: rgba(59, 130, 246, 0.1);
    border-color: rgba(96, 165, 250, 0.55);
    box-shadow: inset 0 0 0 1px rgba(59, 130, 246, 0.1);
}

.process-status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
}

.process-status-dot.running {
    background: var(--success);
    box-shadow: 0 0 0 3px var(--success-soft);
}

.process-status-dot.stopped {
    background: var(--text-muted);
}

.process-status-dot.starting,
.process-status-dot.stopping {
    background: var(--warning);
    animation: pulse 1.3s infinite;
}

.process-status-dot.error {
    background: var(--danger);
    box-shadow: 0 0 0 3px var(--danger-soft);
}

@keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.55; }
}

.process-info {
    flex: 1;
    min-width: 0;
}

.process-name {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.process-type {
    font-size: 10px;
    color: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 2px;
}

.process-type-badge {
    padding: 1px 5px;
    border-radius: 4px;
    background: var(--bg-tertiary);
    border: 1px solid var(--border);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.35px;
}

.process-type-badge.docker {
    background: rgba(37, 99, 235, 0.2);
    border-color: rgba(96, 165, 250, 0.5);
    color: #93c5fd;
}

.process-type-badge.managed {
    background: var(--success-soft);
    border-color: rgba(34, 197, 94, 0.45);
    color: #86efac;
}

/* Content Area */
.content-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
}

.content-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
}

.content-title {
    font-size: 15px;
    font-weight: 600;
}

.content-actions {
    display: flex;
    gap: 5px;
}

.log-container {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
}

.log-output {
    flex: 1;
    padding: 12px 14px;
    overflow-y: auto;
    font-family: 'Cascadia Mono', 'Consolas', monospace;
    font-size: 12px;
    line-height: 1.45;
    background: #0a0f15;
    white-space: pre-wrap;
    word-break: break-all;
}

.log-output::-webkit-scrollbar {
    width: 8px;
}

.log-output::-webkit-scrollbar-track {
    background: var(--bg-secondary);
}

.log-output::-webkit-scrollbar-thumb {
    background: var(--border-light);
    border-radius: 4px;
}

.log-line {
    color: var(--text-secondary);
}

.log-line.stderr {
    color: var(--text-secondary);
}

.log-line.warn {
    color: var(--warning);
}

.log-line.error {
    color: var(--danger);
}

.log-line.system {
    color: var(--accent-secondary);
}

/* Empty State */
.empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
    gap: 8px;
    text-align: center;
}

.empty-state-icon {
    font-family: 'Cascadia Mono', 'Consolas', monospace;
    font-size: 20px;
    letter-spacing: 0.5px;
    color: var(--text-secondary);
    border: 1px dashed var(--border-light);
    border-radius: 6px;
    padding: 6px 10px;
    opacity: 0.85;
}

.empty-state-text {
    font-size: 15px;
}

.empty-state-hint {
    font-size: 12px;
    color: var(--text-muted);
}

/* Modal */
.modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.58);
    backdrop-filter: blur(2px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
}

.modal {
    background: var(--bg-secondary);
    border-radius: var(--radius-lg);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    width: 420px;
    max-width: 90vw;
    max-height: 90vh;
    overflow: hidden;
}

.modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border);
}

.modal-title {
    font-size: 15px;
    font-weight: 600;
}

.modal-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 18px;
    padding: 2px 6px;
    border-radius: 4px;
    transition: var(--transition);
}

.modal-close:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
}

.modal-body {
    padding: 14px;
}

.form-group {
    margin-bottom: 14px;
}

.form-label {
    display: block;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-secondary);
    margin-bottom: 5px;
}

.form-input {
    width: 100%;
    padding: 8px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    transition: var(--transition);
}

.form-input:focus {
    outline: none;
    border-color: var(--accent-primary);
    box-shadow: 0 0 0 3px var(--accent-glow);
}

.form-input::placeholder {
    color: var(--text-muted);
}

.form-select {
    width: 100%;
    padding: 8px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 13px;
    cursor: pointer;
}

.form-select:focus {
    outline: none;
    border-color: var(--accent-primary);
}

.form-hint {
    font-size: 10px;
    color: var(--text-muted);
    margin-top: 4px;
}

.form-checkbox-row {
    display: flex;
    align-items: center;
    gap: 8px;
}

.form-checkbox {
    width: 14px;
    height: 14px;
    accent-color: var(--accent-primary);
}

.modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
    background: var(--bg-tertiary);
}

/* Status Badge */
.status-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 20px;
    font-size: 11px;
    font-weight: 500;
    border: 1px solid transparent;
}

.status-badge.running {
    background: var(--success-soft);
    color: #86efac;
    border-color: rgba(34, 197, 94, 0.45);
}

.status-badge.stopped {
    background: rgba(136, 146, 160, 0.16);
    color: var(--text-muted);
    border-color: rgba(136, 146, 160, 0.35);
}

.status-badge.starting,
.status-badge.stopping {
    background: var(--warning-soft);
    color: #fcd34d;
    border-color: rgba(251, 191, 36, 0.45);
}

.status-badge.error {
    background: var(--danger-soft);
    color: #fca5a5;
    border-color: rgba(248, 113, 113, 0.45);
}

/* Confirm Dialog */
.confirm-dialog {
    text-align: center;
    padding: 16px;
}

.confirm-dialog-text {
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: 14px;
}

.confirm-dialog-actions {
    display: flex;
    justify-content: center;
    gap: 12px;
}
"#;

/// Global process manager for cleanup on exit
static GLOBAL_MANAGER: std::sync::OnceLock<Arc<ProcessManager>> = std::sync::OnceLock::new();

fn main() {
    // Load icon from assets folder next to executable
    let icon = load_icon();

    // Build the Dioxus app with desktop config
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_menu(None) // Hide menu bar
                .with_window(
                    dioxus::desktop::WindowBuilder::new()
                        .with_title("Process Manager")
                        .with_inner_size(dioxus::desktop::LogicalSize::new(1100.0, 700.0))
                        .with_min_inner_size(dioxus::desktop::LogicalSize::new(800.0, 500.0))
                        .with_window_icon(icon),
                ),
        )
        .launch(App);
}

/// Load the application icon
fn load_icon() -> Option<dioxus::desktop::tao::window::Icon> {
    // Try to load icon from assets folder next to executable
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;
    let icon_path = exe_dir.join("assets").join("icon.png");

    // If not found next to exe, try current working directory
    let icon_path = if icon_path.exists() {
        icon_path
    } else {
        std::path::PathBuf::from("assets/icon.png")
    };

    let icon_bytes = std::fs::read(&icon_path).ok()?;
    let image = image::load_from_memory(&icon_bytes).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();

    dioxus::desktop::tao::window::Icon::from_rgba(rgba, width, height).ok()
}

/// Main app state - using Copy-friendly signals for component props
#[derive(Clone, Copy, PartialEq)]
struct AppState {
    config: Signal<AppConfig>,
    selected_process: Signal<Option<String>>,
    show_add_modal: Signal<bool>,
    show_edit_modal: Signal<Option<String>>,
    show_confirm_delete: Signal<Option<String>>,
    // Force re-render counter for log updates
    refresh_counter: Signal<u64>,
}

/// New process form state
#[derive(Clone, PartialEq)]
struct NewProcessForm {
    name: String,
    command: String,
    working_directory: String,
    process_type: String,
    auto_restart: bool,
}

impl Default for NewProcessForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            working_directory: String::new(),
            process_type: "Process".to_string(), // Default to shell command
            auto_restart: false,
        }
    }
}

impl NewProcessForm {
    fn from_process(process: &ProcessConfig) -> Self {
        Self {
            name: process.name.clone(),
            command: process.command.clone(),
            working_directory: process.working_directory.clone(),
            process_type: process.process_type.to_string(),
            auto_restart: process.auto_restart,
        }
    }
}

#[component]
fn App() -> Element {
    // Initialize state
    let config = use_signal(AppConfig::load);
    let selected_process: Signal<Option<String>> = use_signal(|| None);
    let show_add_modal = use_signal(|| false);
    let show_edit_modal: Signal<Option<String>> = use_signal(|| None);
    let show_confirm_delete: Signal<Option<String>> = use_signal(|| None);
    let mut refresh_counter = use_signal(|| 0u64);
    let last_error_version = use_signal(|| 0u64);
    let window = dioxus::desktop::use_window();

    // Initialize manager once and store globally for cleanup
    let manager = use_hook(|| {
        let m = Arc::new(ProcessManager::new());
        let _ = GLOBAL_MANAGER.set(m.clone());
        m
    });

    // Initialize manager with config
    use_effect({
        let manager = manager.clone();
        let config = config.read().clone();
        move || {
            manager.init_from_config(&config.processes);
            manager.start_background_tasks();
        }
    });

    // Subscribe to manager events to trigger re-renders
    use_future({
        let manager = manager.clone();
        move || {
            let mut rx = manager.subscribe();
            async move {
                loop {
                    if rx.changed().await.is_err() {
                        break;
                    }
                    refresh_counter.set(*rx.borrow());
                }
            }
        }
    });

    // Flash taskbar icon when new errors arrive and the window isn't focused.
    use_effect({
        let manager = manager.clone();
        let mut last_error_version = last_error_version.clone();
        let window = window.clone();
        move || {
            let _ = *refresh_counter.read();
            let current = manager.error_version();
            let last_seen = *last_error_version.read();
            if current > last_seen {
                if !window.is_focused() {
                    window.request_user_attention(Some(
                        dioxus::desktop::tao::window::UserAttentionType::Informational,
                    ));

                    // Stop flashing after 5 seconds
                    let window = window.clone();
                    spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        window.request_user_attention(None);
                    });
                }
                last_error_version.set(current);
            }
        }
    });

    // Setup cleanup on window close via drop guard
    use_drop({
        let manager = manager.clone();
        move || {
            manager.stop_non_docker();
        }
    });

    let state = AppState {
        config,
        selected_process,
        show_add_modal,
        show_edit_modal,
        show_confirm_delete,
        refresh_counter,
    };

    rsx! {
        style { {STYLES} }
        div {
            class: "app-container",
            Header { state }
            div {
                class: "main-content",
                Sidebar { state }
                ContentArea { state }
            }
            if *state.show_add_modal.read() {
                AddProcessModal { state }
            }
            if let Some(process_id) = state.show_edit_modal.read().clone() {
                EditProcessModal {
                    key: "{process_id}",
                    state: state,
                    process_id: process_id.clone(),
                }
            }
            if state.show_confirm_delete.read().is_some() {
                DeleteConfirmModal { state }
            }
        }
    }
}

fn get_manager() -> Arc<ProcessManager> {
    GLOBAL_MANAGER
        .get()
        .expect("Manager not initialized")
        .clone()
}

fn wait_for_process_stop(manager: &ProcessManager, id: &str) {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if let Some(status) = manager.get_status(id) {
            if status == ProcessStatus::Stopped || matches!(status, ProcessStatus::Error(_)) {
                break;
            }
        } else {
            break;
        }
    }
}

#[component]
fn Header(state: AppState) -> Element {
    let mut config = state.config;
    let mut editing_stack_name = use_signal(|| false);
    let mut temp_stack_name = use_signal(|| String::new());

    let stack_name = config.read().stack_name.clone();

    rsx! {
        header {
            class: "header",
            div {
                class: "header-title",
                span { class: "header-title-icon", "PM" }
                span { "Process Manager" }
                span { class: "header-separator", " — " }
                if *editing_stack_name.read() {
                    input {
                        class: "stack-name-input",
                        r#type: "text",
                        value: "{temp_stack_name.read()}",
                        autofocus: true,
                        oninput: move |e| {
                            temp_stack_name.set(e.value());
                        },
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                let new_name = temp_stack_name.read().clone();
                                if !new_name.is_empty() {
                                    config.write().stack_name = new_name;
                                    let _ = config.read().save();
                                }
                                editing_stack_name.set(false);
                            } else if e.key() == Key::Escape {
                                editing_stack_name.set(false);
                            }
                        },
                        onblur: move |_| {
                            let new_name = temp_stack_name.read().clone();
                            if !new_name.is_empty() {
                                config.write().stack_name = new_name;
                                let _ = config.read().save();
                            }
                            editing_stack_name.set(false);
                        },
                    }
                } else {
                    span {
                        class: "stack-name",
                        onclick: move |_| {
                            temp_stack_name.set(config.read().stack_name.clone());
                            editing_stack_name.set(true);
                        },
                        "{stack_name}"
                        span { class: "edit-icon", title: "Click to edit", " rename" }
                    }
                }
            }
            div {
                class: "header-actions",
                button {
                    class: "btn btn-success",
                    title: "Start All",
                    onclick: move |_| {
                        get_manager().start_all();
                    },
                    "Start All"
                }
                button {
                    class: "btn btn-danger",
                    title: "Stop All",
                    onclick: move |_| {
                        get_manager().stop_all();
                    },
                    "Stop All"
                }
                button {
                    class: "btn btn-warning",
                    title: "Restart All",
                    onclick: move |_| {
                        get_manager().restart_all();
                    },
                    "Restart All"
                }
            }
        }
    }
}

#[component]
fn Sidebar(state: AppState) -> Element {
    let config = state.config.read();
    let mut show_add_modal = state.show_add_modal;
    let refresh_token = *state.refresh_counter.read();

    rsx! {
        aside {
            class: "sidebar",
            div {
                class: "sidebar-header",
                span { class: "sidebar-title", "Processes" }
                button {
                    class: "btn btn-primary btn-icon btn-small",
                    title: "Add Process",
                    onclick: move |_| {
                        show_add_modal.set(true);
                    },
                    "+"
                }
            }
            div {
                class: "process-list",
                if config.processes.is_empty() {
                    div {
                        class: "empty-state",
                        style: "padding: 40px 20px;",
                        div { class: "empty-state-icon", "[ ]" }
                        div { class: "empty-state-text", "No processes yet" }
                        div { class: "empty-state-hint", "Click + to add one" }
                    }
                } else {
                    for process in config.processes.iter() {
                        ProcessItem {
                            key: "{process.id}",
                            state: state,
                            process: process.clone(),
                            refresh_token: refresh_token,
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ProcessItem(state: AppState, process: ProcessConfig, refresh_token: u64) -> Element {
    let _ = refresh_token;
    let selected = state.selected_process.read();
    let is_active = selected.as_ref() == Some(&process.id);

    let manager = get_manager();
    let status = manager
        .get_status(&process.id)
        .unwrap_or(ProcessStatus::Stopped);

    let status_class = match &status {
        ProcessStatus::Running => "running",
        ProcessStatus::Stopped => "stopped",
        ProcessStatus::Starting => "starting",
        ProcessStatus::Stopping => "stopping",
        ProcessStatus::Error(_) => "error",
    };

    let type_class = match process.process_type {
        ProcessType::Docker => "docker",
        ProcessType::Process => "",
    };

    let id = process.id.clone();
    let mut selected_signal = state.selected_process;

    rsx! {
        div {
            class: if is_active { "process-item active" } else { "process-item" },
            onclick: move |_| {
                selected_signal.set(Some(id.clone()));
            },
            div {
                class: "process-status-dot {status_class}",
            }
            div {
                class: "process-info",
                div { class: "process-name", "{process.name}" }
                div {
                    class: "process-type",
                    span {
                        class: "process-type-badge {type_class}",
                        "{process.process_type}"
                    }
                    if process.auto_restart {
                        span {
                            class: "process-type-badge managed",
                            title: "Managed restart enabled",
                            "AUTO"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ContentArea(state: AppState) -> Element {
    let selected = state.selected_process.read();
    // Read refresh counter to trigger re-renders
    let _ = *state.refresh_counter.read();

    match selected.as_ref() {
        Some(id) => {
            let config = state.config.read();
            if let Some(process) = config.get_process(id) {
                rsx! {
                    ProcessDetail {
                        state: state,
                        process: process.clone(),
                    }
                }
            } else {
                rsx! { EmptyState {} }
            }
        }
        None => {
            rsx! { EmptyState {} }
        }
    }
}

#[component]
fn EmptyState() -> Element {
    rsx! {
        div {
            class: "content-area",
            div {
                class: "empty-state",
                div { class: "empty-state-icon", "[log]" }
                div { class: "empty-state-text", "Select a process to view logs" }
                div { class: "empty-state-hint", "Or add a new process using the + button" }
            }
        }
    }
}

#[component]
fn ProcessDetail(state: AppState, process: ProcessConfig) -> Element {
    // Read refresh counter to trigger re-renders when process state changes
    let _ = *state.refresh_counter.read();

    use_effect({
        let refresh_counter = state.refresh_counter;
        let process_id = process.id.clone();
        move || {
            let _ = *refresh_counter.read();
            let script = r#"
                    (() => {
                        const el = document.getElementById("log-output");
                        if (!el) return;
                        const threshold = 16;
                        const processId = "__PROCESS_ID__";
                        if (!el.dataset.pmAutoScrollBound) {
                            el.dataset.pmAutoScrollBound = "1";
                            el.dataset.pmStickBottom = "true";
                            el.addEventListener("scroll", () => {
                                const distance = el.scrollHeight - el.clientHeight - el.scrollTop;
                                el.dataset.pmStickBottom = distance <= threshold ? "true" : "false";
                            });
                        }
                        if (el.dataset.pmProcessId !== processId) {
                            el.dataset.pmProcessId = processId;
                            el.dataset.pmStickBottom = "true";
                        }
                        const distance = el.scrollHeight - el.clientHeight - el.scrollTop;
                        const shouldStick = el.dataset.pmStickBottom !== "false" || distance <= threshold;
                        if (shouldStick) {
                            el.scrollTop = el.scrollHeight;
                            el.dataset.pmStickBottom = "true";
                        }
                    })();
                "#
                .replace("__PROCESS_ID__", &process_id);
            dioxus::document::eval(script.as_str());
        }
    });

    let manager = get_manager();
    let status = manager
        .get_status(&process.id)
        .unwrap_or(ProcessStatus::Stopped);
    let logs = manager.get_logs(&process.id);

    let status_class = match &status {
        ProcessStatus::Running => "running",
        ProcessStatus::Stopped => "stopped",
        ProcessStatus::Starting => "starting",
        ProcessStatus::Stopping => "stopping",
        ProcessStatus::Error(_) => "error",
    };

    let id_start = process.id.clone();
    let id_stop = process.id.clone();
    let id_restart = process.id.clone();
    let id_edit = process.id.clone();
    let id_delete = process.id.clone();

    let mut show_edit_modal = state.show_edit_modal;
    let mut confirm_delete = state.show_confirm_delete;
    rsx! {
        div {
            class: "content-area",
            div {
                class: "content-header",
                div {
                    class: "content-title",
                    "{process.name}"
                    span {
                        class: "status-badge {status_class}",
                        style: "margin-left: 12px;",
                        "{status.to_string()}"
                    }
                }
                div {
                    class: "content-actions",
                    span {
                        class: "form-hint",
                        if process.auto_restart { "Managed restart: ON" } else { "Managed restart: OFF" }
                    }
                    button {
                        class: "btn btn-success btn-small",
                        title: "Start",
                        onclick: move |_| {
                            get_manager().start_process(&id_start);
                        },
                        "Start"
                    }
                    button {
                        class: "btn btn-danger btn-small",
                        title: "Stop",
                        onclick: move |_| {
                            get_manager().stop_process(&id_stop);
                        },
                        "Stop"
                    }
                    button {
                        class: "btn btn-warning btn-small",
                        title: "Restart",
                        onclick: move |_| {
                            get_manager().restart_process(&id_restart);
                        },
                        "Restart"
                    }
                    button {
                        class: "btn btn-small",
                        title: "Edit",
                        onclick: move |_| {
                            show_edit_modal.set(Some(id_edit.clone()));
                        },
                        "Edit"
                    }
                    button {
                        class: "btn btn-small",
                        title: "Delete",
                        style: "margin-left: 10px;",
                        onclick: move |_| {
                            confirm_delete.set(Some(id_delete.clone()));
                        },
                        "Del"
                    }
                }
            }
            div {
                class: "log-container",
                div {
                    class: "log-output",
                    id: "log-output",
                    if logs.is_empty() {
                        div {
                            class: "log-line system",
                            "No output yet. Start the process to see logs."
                        }
                    } else {
                        for (i, line) in logs.iter().enumerate() {
                            {
                                let trimmed = line.trim();
                                let (content, from_stderr) = if let Some(rest) = trimmed.strip_prefix("[stderr] ") {
                                    (rest, true)
                                } else if let Some(rest) = trimmed.strip_prefix("[stderr]") {
                                    (rest.trim_start(), true)
                                } else {
                                    (trimmed, false)
                                };

                                let lower = content.to_ascii_lowercase();
                                let is_error = lower.contains("error")
                                    || lower.contains("critical")
                                    || lower.contains("fatal")
                                    || lower.contains("panic")
                                    || lower.contains("traceback")
                                    || lower.contains("exception");
                                let is_warn = lower.contains("warn");
                                let is_system = trimmed.starts_with("[") && trimmed.ends_with("]");

                                let log_class = if is_system {
                                    "log-line system"
                                } else if is_error {
                                    "log-line error"
                                } else if is_warn {
                                    "log-line warn"
                                } else if from_stderr {
                                    "log-line stderr"
                                } else {
                                    "log-line"
                                };

                                rsx! {
                                    div {
                                        key: "{i}",
                                        class: "{log_class}",
                                        "{line}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn AddProcessModal(state: AppState) -> Element {
    let mut form = use_signal(NewProcessForm::default);
    let mut show_modal = state.show_add_modal;
    let mut config = state.config;

    rsx! {
        div {
            class: "modal-overlay",
            onclick: move |_| show_modal.set(false),
            div {
                class: "modal",
                onclick: |e| e.stop_propagation(),
                div {
                    class: "modal-header",
                    span { class: "modal-title", "Add New Process" }
                    button {
                        class: "modal-close",
                        onclick: move |_| show_modal.set(false),
                        "×"
                    }
                }
                div {
                    class: "modal-body",
                    div {
                        class: "form-group",
                        label { class: "form-label", "Name" }
                        input {
                            class: "form-input",
                            r#type: "text",
                            placeholder: "e.g., Frontend Dev Server",
                            value: "{form.read().name}",
                            oninput: move |e| {
                                form.write().name = e.value();
                            },
                        }
                    }
                    div {
                        class: "form-group",
                        label { class: "form-label", "Type" }
                        select {
                            class: "form-select",
                            value: "{form.read().process_type}",
                            onchange: move |e| {
                                form.write().process_type = e.value();
                            },
                            option { value: "Process", "Process (Shell Command)" }
                            option { value: "Docker", "Docker Container" }
                        }
                    }
                    div {
                        class: "form-group",
                        label { class: "form-label",
                            if form.read().process_type == "Docker" {
                                "Container Name"
                            } else {
                                "Command"
                            }
                        }
                        input {
                            class: "form-input",
                            r#type: "text",
                            placeholder: if form.read().process_type == "Docker" {
                                "e.g., my-postgres-container"
                            } else {
                                "e.g., npm run dev"
                            },
                            value: "{form.read().command}",
                            oninput: move |e| {
                                form.write().command = e.value();
                            },
                        }
                        div {
                            class: "form-hint",
                            if form.read().process_type == "Docker" {
                                "The name of the Docker container to manage"
                            } else {
                                "The shell command to execute"
                            }
                        }
                    }
                    if form.read().process_type != "Docker" {
                        div {
                            class: "form-group",
                            label { class: "form-label", "Working Directory (optional)" }
                            input {
                                class: "form-input",
                                r#type: "text",
                                placeholder: "e.g., C:/projects/my-app",
                                value: "{form.read().working_directory}",
                                oninput: move |e| {
                                    form.write().working_directory = e.value();
                                },
                            }
                            div { class: "form-hint", "Leave empty to use current directory" }
                        }
                    }
                    div {
                        class: "form-group",
                        label { class: "form-label", "Managed Restart" }
                        div {
                            class: "form-checkbox-row",
                            input {
                                class: "form-checkbox",
                                r#type: "checkbox",
                                checked: form.read().auto_restart,
                                onchange: move |e| {
                                    let value = e.value();
                                    form.write().auto_restart = value == "true" || value == "on";
                                }
                            }
                            span { "Keep this entry running (auto-restart if it goes down)" }
                        }
                    }
                }
                div {
                    class: "modal-footer",
                    button {
                        class: "btn",
                        onclick: move |_| show_modal.set(false),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            // Clone values out of the form before any mutations
                            let (name, command, working_directory, process_type_str, auto_restart) = {
                                let f = form.read();
                                (
                                    f.name.clone(),
                                    f.command.clone(),
                                    f.working_directory.clone(),
                                    f.process_type.clone(),
                                    f.auto_restart,
                                )
                            };

                            if name.is_empty() || command.is_empty() {
                                return;
                            }

                            let process_type = match process_type_str.as_str() {
                                "Docker" => ProcessType::Docker,
                                _ => ProcessType::Process,
                            };

                            let mut new_process = ProcessConfig::new(
                                name,
                                command,
                                working_directory,
                                process_type,
                            );
                            new_process.auto_restart = auto_restart;

                            // Add to config and save
                            config.write().add_process(new_process.clone());
                            let _ = config.read().save();

                            // Add to manager
                            get_manager().add_process(new_process);

                            // Reset and close
                            form.set(NewProcessForm::default());
                            show_modal.set(false);
                        },
                        "Add Process"
                    }
                }
            }
        }
    }
}

#[component]
fn EditProcessModal(state: AppState, process_id: String) -> Element {
    let process = state.config.read().get_process(&process_id).cloned();
    let Some(process) = process else {
        return rsx! {};
    };

    let mut form = use_signal({
        let process = process.clone();
        move || NewProcessForm::from_process(&process)
    });
    let mut show_modal = state.show_edit_modal;
    let mut config = state.config;

    let id_save = process_id.clone();

    rsx! {
        div {
            class: "modal-overlay",
            onclick: move |_| show_modal.set(None),
            div {
                class: "modal",
                onclick: |e| e.stop_propagation(),
                div {
                    class: "modal-header",
                    span { class: "modal-title", "Edit Process" }
                    button {
                        class: "modal-close",
                        onclick: move |_| show_modal.set(None),
                        "×"
                    }
                }
                div {
                    class: "modal-body",
                    div {
                        class: "form-group",
                        label { class: "form-label", "Name" }
                        input {
                            class: "form-input",
                            r#type: "text",
                            value: "{form.read().name}",
                            oninput: move |e| {
                                form.write().name = e.value();
                            },
                        }
                    }
                    div {
                        class: "form-group",
                        label { class: "form-label", "Type" }
                        select {
                            class: "form-select",
                            value: "{form.read().process_type}",
                            onchange: move |e| {
                                form.write().process_type = e.value();
                            },
                            option { value: "Process", "Process (Shell Command)" }
                            option { value: "Docker", "Docker Container" }
                        }
                    }
                    div {
                        class: "form-group",
                        label { class: "form-label",
                            if form.read().process_type == "Docker" {
                                "Container Name"
                            } else {
                                "Command"
                            }
                        }
                        input {
                            class: "form-input",
                            r#type: "text",
                            value: "{form.read().command}",
                            oninput: move |e| {
                                form.write().command = e.value();
                            },
                        }
                        div {
                            class: "form-hint",
                            if form.read().process_type == "Docker" {
                                "The name of the Docker container to manage"
                            } else {
                                "The shell command to execute"
                            }
                        }
                    }
                    if form.read().process_type != "Docker" {
                        div {
                            class: "form-group",
                            label { class: "form-label", "Working Directory (optional)" }
                            input {
                                class: "form-input",
                                r#type: "text",
                                value: "{form.read().working_directory}",
                                oninput: move |e| {
                                    form.write().working_directory = e.value();
                                },
                            }
                            div { class: "form-hint", "Leave empty to use current directory" }
                        }
                    }
                    div {
                        class: "form-group",
                        label { class: "form-label", "Managed Restart" }
                        div {
                            class: "form-checkbox-row",
                            input {
                                class: "form-checkbox",
                                r#type: "checkbox",
                                checked: form.read().auto_restart,
                                onchange: move |e| {
                                    let value = e.value();
                                    form.write().auto_restart = value == "true" || value == "on";
                                }
                            }
                            span { "Keep this entry running (auto-restart if it goes down)" }
                        }
                    }
                }
                div {
                    class: "modal-footer",
                    button {
                        class: "btn",
                        onclick: move |_| show_modal.set(None),
                        "Cancel"
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            let (name, command, working_directory, process_type_str, auto_restart) = {
                                let f = form.read();
                                (
                                    f.name.clone(),
                                    f.command.clone(),
                                    f.working_directory.clone(),
                                    f.process_type.clone(),
                                    f.auto_restart,
                                )
                            };

                            if name.is_empty() || command.is_empty() {
                                return;
                            }

                            let process_type = match process_type_str.as_str() {
                                "Docker" => ProcessType::Docker,
                                _ => ProcessType::Process,
                            };

                            let auto_start = config
                                .read()
                                .get_process(&id_save)
                                .map(|p| p.auto_start)
                                .unwrap_or(false);

                            let updated = ProcessConfig {
                                id: id_save.clone(),
                                name,
                                command,
                                working_directory,
                                process_type,
                                auto_start,
                                auto_restart,
                            };

                            let manager = get_manager();
                            if matches!(
                                manager.get_status(&id_save),
                                Some(ProcessStatus::Running | ProcessStatus::Starting | ProcessStatus::Stopping)
                            ) {
                                manager.stop_process(&id_save);
                                wait_for_process_stop(&manager, &id_save);
                            }

                            config.write().update_process(&id_save, updated.clone());
                            let _ = config.read().save();
                            let _ = manager.update_process_config(updated);
                            show_modal.set(None);
                        },
                        "Save"
                    }
                }
            }
        }
    }
}

#[component]
fn DeleteConfirmModal(state: AppState) -> Element {
    let mut confirm_delete = state.show_confirm_delete;
    let mut config = state.config;
    let mut selected = state.selected_process;

    let id = confirm_delete.read().clone().unwrap_or_default();
    let process_name = config
        .read()
        .get_process(&id)
        .map(|p| p.name.clone())
        .unwrap_or_default();

    let id_confirm = id.clone();

    rsx! {
        div {
            class: "modal-overlay",
            onclick: move |_| confirm_delete.set(None),
            div {
                class: "modal",
                style: "width: 380px;",
                onclick: |e| e.stop_propagation(),
                div {
                    class: "modal-header",
                    span { class: "modal-title", "Confirm Delete" }
                    button {
                        class: "modal-close",
                        onclick: move |_| confirm_delete.set(None),
                        "×"
                    }
                }
                div {
                    class: "confirm-dialog",
                    div {
                        class: "confirm-dialog-text",
                        "Are you sure you want to delete "
                        strong { "{process_name}" }
                        "? This action cannot be undone."
                    }
                    div {
                        class: "confirm-dialog-actions",
                        button {
                            class: "btn",
                            onclick: move |_| confirm_delete.set(None),
                            "Cancel"
                        }
                        button {
                            class: "btn btn-danger",
                            onclick: move |_| {
                                // Stop and remove
                                get_manager().remove_process(&id_confirm);
                                config.write().remove_process(&id_confirm);
                                let _ = config.read().save();

                                // Clear selection if deleted
                                if selected.read().as_ref() == Some(&id_confirm) {
                                    selected.set(None);
                                }

                                confirm_delete.set(None);
                            },
                            "Delete"
                        }
                    }
                }
            }
        }
    }
}
