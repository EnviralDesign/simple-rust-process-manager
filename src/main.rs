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
    --bg-primary: #0f0f14;
    --bg-secondary: #16161e;
    --bg-tertiary: #1e1e28;
    --bg-hover: #252532;
    --accent-primary: #7c3aed;
    --accent-secondary: #a78bfa;
    --accent-glow: rgba(124, 58, 237, 0.3);
    --text-primary: #e4e4e7;
    --text-secondary: #a1a1aa;
    --text-muted: #71717a;
    --success: #22c55e;
    --success-glow: rgba(34, 197, 94, 0.2);
    --warning: #f59e0b;
    --danger: #ef4444;
    --danger-glow: rgba(239, 68, 68, 0.2);
    --border: #27272a;
    --border-light: #3f3f46;
    --radius: 8px;
    --radius-lg: 12px;
    --shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
    --transition: all 0.2s ease;
}

* {
    margin: 0;
    padding: 0;
    box-sizing: border-box;
}

body {
    font-family: 'Segoe UI', system-ui, -apple-system, sans-serif;
    background: var(--bg-primary);
    color: var(--text-primary);
    overflow: hidden;
}

.app-container {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: linear-gradient(135deg, var(--bg-primary) 0%, #12121a 100%);
}

/* Header */
.header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 20px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.3);
    z-index: 100;
}

.header-title {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 18px;
    font-weight: 600;
    color: var(--text-primary);
}

.header-title-icon {
    font-size: 22px;
}

.header-actions {
    display: flex;
    gap: 8px;
}

.header-separator {
    color: var(--text-muted);
    margin: 0 4px;
}

.stack-name {
    color: var(--accent-secondary);
    cursor: pointer;
    padding: 2px 8px;
    border-radius: var(--radius);
    transition: var(--transition);
}

.stack-name:hover {
    background: var(--bg-hover);
}

.edit-icon {
    font-size: 14px;
    opacity: 0.5;
    transition: var(--transition);
}

.stack-name:hover .edit-icon {
    opacity: 1;
}

.stack-name-input {
    background: var(--bg-primary);
    border: 1px solid var(--accent-primary);
    border-radius: var(--radius);
    color: var(--accent-secondary);
    font-size: 18px;
    font-weight: 600;
    padding: 2px 10px;
    width: 200px;
    outline: none;
}

.btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border: none;
    border-radius: var(--radius);
    font-size: 13px;
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
    transform: translateY(-1px);
}

.btn-primary {
    background: var(--accent-primary);
    border-color: var(--accent-primary);
    color: white;
}

.btn-primary:hover {
    background: #8b5cf6;
    box-shadow: 0 0 20px var(--accent-glow);
}

.btn-success {
    background: var(--success);
    border-color: var(--success);
    color: white;
}

.btn-success:hover {
    box-shadow: 0 0 20px var(--success-glow);
}

.btn-danger {
    background: var(--danger);
    border-color: var(--danger);
    color: white;
}

.btn-danger:hover {
    box-shadow: 0 0 20px var(--danger-glow);
}

.btn-warning {
    background: var(--warning);
    border-color: var(--warning);
    color: #1a1a1a;
}

.btn-icon {
    padding: 8px;
    min-width: 34px;
    justify-content: center;
}

.btn-small {
    padding: 5px 10px;
    font-size: 12px;
}

/* Main Layout */
.main-content {
    display: flex;
    flex: 1;
    overflow: hidden;
}

/* Sidebar */
.sidebar {
    width: 280px;
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
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
}

.sidebar-title {
    font-size: 12px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: var(--text-muted);
}

.process-list {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
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
    gap: 10px;
    padding: 12px 14px;
    margin-bottom: 4px;
    border-radius: var(--radius);
    cursor: pointer;
    transition: var(--transition);
    border: 1px solid transparent;
}

.process-item:hover {
    background: var(--bg-hover);
}

.process-item.active {
    background: var(--bg-tertiary);
    border-color: var(--accent-primary);
    box-shadow: 0 0 15px var(--accent-glow);
}

.process-status-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
}

.process-status-dot.running {
    background: var(--success);
    box-shadow: 0 0 8px var(--success);
    animation: pulse 2s infinite;
}

.process-status-dot.stopped {
    background: var(--text-muted);
}

.process-status-dot.starting,
.process-status-dot.stopping {
    background: var(--warning);
    animation: pulse 1s infinite;
}

.process-status-dot.error {
    background: var(--danger);
    box-shadow: 0 0 8px var(--danger);
}

@keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
}

.process-info {
    flex: 1;
    min-width: 0;
}

.process-name {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.process-type {
    font-size: 11px;
    color: var(--text-muted);
    display: flex;
    align-items: center;
    gap: 4px;
    margin-top: 2px;
}

.process-type-badge {
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--bg-primary);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.3px;
}

.process-type-badge.docker {
    background: #1d4ed8;
    color: white;
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
    padding: 16px 20px;
    background: var(--bg-secondary);
    border-bottom: 1px solid var(--border);
}

.content-title {
    font-size: 16px;
    font-weight: 600;
}

.content-actions {
    display: flex;
    gap: 6px;
}

.log-container {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
}

.log-output {
    flex: 1;
    padding: 16px;
    overflow-y: auto;
    font-family: 'Cascadia Code', 'Fira Code', 'Consolas', monospace;
    font-size: 13px;
    line-height: 1.6;
    background: var(--bg-primary);
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
    font-style: italic;
}

/* Empty State */
.empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    color: var(--text-muted);
    gap: 16px;
}

.empty-state-icon {
    font-size: 48px;
    opacity: 0.3;
}

.empty-state-text {
    font-size: 16px;
}

.empty-state-hint {
    font-size: 13px;
    color: var(--text-muted);
}

/* Modal */
.modal-overlay {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(4px);
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
    width: 450px;
    max-width: 90vw;
    max-height: 90vh;
    overflow: hidden;
}

.modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 20px;
    border-bottom: 1px solid var(--border);
}

.modal-title {
    font-size: 16px;
    font-weight: 600;
}

.modal-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 20px;
    padding: 4px;
    border-radius: 4px;
    transition: var(--transition);
}

.modal-close:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
}

.modal-body {
    padding: 20px;
}

.form-group {
    margin-bottom: 18px;
}

.form-label {
    display: block;
    font-size: 13px;
    font-weight: 500;
    color: var(--text-secondary);
    margin-bottom: 6px;
}

.form-input {
    width: 100%;
    padding: 10px 14px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 14px;
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
    padding: 10px 14px;
    background: var(--bg-primary);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text-primary);
    font-size: 14px;
    cursor: pointer;
}

.form-select:focus {
    outline: none;
    border-color: var(--accent-primary);
}

.form-hint {
    font-size: 11px;
    color: var(--text-muted);
    margin-top: 4px;
}

.modal-footer {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    padding: 16px 20px;
    border-top: 1px solid var(--border);
    background: var(--bg-tertiary);
}

/* Status Badge */
.status-badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border-radius: 20px;
    font-size: 12px;
    font-weight: 500;
}

.status-badge.running {
    background: var(--success-glow);
    color: var(--success);
}

.status-badge.stopped {
    background: rgba(113, 113, 122, 0.2);
    color: var(--text-muted);
}

.status-badge.starting,
.status-badge.stopping {
    background: rgba(245, 158, 11, 0.2);
    color: var(--warning);
}

.status-badge.error {
    background: var(--danger-glow);
    color: var(--danger);
}

/* Confirm Dialog */
.confirm-dialog {
    text-align: center;
    padding: 20px;
}

.confirm-dialog-text {
    font-size: 15px;
    color: var(--text-secondary);
    margin-bottom: 20px;
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
                        .with_window_icon(icon)
                )
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
}

impl Default for NewProcessForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            working_directory: String::new(),
            process_type: "Process".to_string(), // Default to shell command
        }
    }
}

#[component]
fn App() -> Element {
    // Initialize state
    let config = use_signal(AppConfig::load);
    let selected_process: Signal<Option<String>> = use_signal(|| None);
    let show_add_modal = use_signal(|| false);
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
            if state.show_confirm_delete.read().is_some() {
                DeleteConfirmModal { state }
            }
        }
    }
}

fn get_manager() -> Arc<ProcessManager> {
    GLOBAL_MANAGER.get().expect("Manager not initialized").clone()
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
                span { class: "header-title-icon", "⚡" }
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
                        span { class: "edit-icon", title: "Click to edit", " ✏️" }
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
                    "▶ Start All"
                }
                button {
                    class: "btn btn-danger",
                    title: "Stop All",
                    onclick: move |_| {
                        get_manager().stop_all();
                    },
                    "◼ Stop All"
                }
                button {
                    class: "btn btn-warning",
                    title: "Restart All",
                    onclick: move |_| {
                        get_manager().restart_all();
                    },
                    "↻ Restart All"
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
                        div { class: "empty-state-icon", "📋" }
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
    let status = manager.get_status(&process.id).unwrap_or(ProcessStatus::Stopped);
    
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
                div { class: "empty-state-icon", "🖥️" }
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
    
    let manager = get_manager();
    let status = manager.get_status(&process.id).unwrap_or(ProcessStatus::Stopped);
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
    let id_delete = process.id.clone();

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
                    button {
                        class: "btn btn-success btn-small",
                        title: "Start",
                        onclick: move |_| {
                            get_manager().start_process(&id_start);
                        },
                        "▶"
                    }
                    button {
                        class: "btn btn-danger btn-small",
                        title: "Stop",
                        onclick: move |_| {
                            get_manager().stop_process(&id_stop);
                        },
                        "◼"
                    }
                    button {
                        class: "btn btn-warning btn-small",
                        title: "Restart",
                        onclick: move |_| {
                            get_manager().restart_process(&id_restart);
                        },
                        "↻"
                    }
                    button {
                        class: "btn btn-small",
                        title: "Delete",
                        style: "margin-left: 10px;",
                        onclick: move |_| {
                            confirm_delete.set(Some(id_delete.clone()));
                        },
                        "🗑"
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
                            let (name, command, working_directory, process_type_str) = {
                                let f = form.read();
                                (f.name.clone(), f.command.clone(), f.working_directory.clone(), f.process_type.clone())
                            };

                            if name.is_empty() || command.is_empty() {
                                return;
                            }

                            let process_type = match process_type_str.as_str() {
                                "Docker" => ProcessType::Docker,
                                _ => ProcessType::Process,
                            };

                            let new_process = ProcessConfig::new(
                                name,
                                command,
                                working_directory,
                                process_type,
                            );

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
fn DeleteConfirmModal(state: AppState) -> Element {
    let mut confirm_delete = state.show_confirm_delete;
    let mut config = state.config;
    let mut selected = state.selected_process;

    let id = confirm_delete.read().clone().unwrap_or_default();
    let process_name = config.read()
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
