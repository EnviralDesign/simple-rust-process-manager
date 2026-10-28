//! Native desktop shell built with egui/eframe.
//!
//! `mod.rs` holds the app's state, navigation and actions; each screen draws itself in its own
//! module (`header`, `sidebar`, `console`, `editor`, `overview`, `palette`, `stack_settings`,
//! `dialogs`) from the kit in `theme`, `widgets` and `icons`, with the wording in `format`.

mod console;
mod dialogs;
mod editor;
mod format;
mod header;
mod icons;
mod overview;
mod palette;
mod sidebar;
mod stack_settings;
mod theme;
mod widgets;

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, CentralPanel, Context, Key, Pos2, SidePanel, TopBottomPanel, Vec2, ViewportBuilder,
    ViewportCommand, Window,
};
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::runtime::Runtime;

use crate::config::{AppConfig, ProcessConfig, ProcessGroupConfig, ProcessType};
use crate::process_manager::{
    ProcessManager, ProcessResourceUsage, ProcessRuntimeInfo, ProcessStatus, UiRuntimeSnapshot,
};
use crate::rest_api::{build_agent_bootstrap, RestServerController};

use editor::Editor;
use format::StatusView;
use stack_settings::StackSettingsForm;
use theme::Tier;

const UI_LOG_LIMIT: usize = 1000;
/// How long a footer message stays.
const MESSAGE_SECONDS: u64 = 6;
const PROJECT_WEBSITE_URL: &str = "https://github.com/EnviralDesign/simple-rust-process-manager";
const PROJECT_GITHUB_ACCOUNT_URL: &str = "https://github.com/EnviralDesign";
const PROJECT_GITHUB_ACCOUNT_HANDLE: &str = "@EnviralDesign";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RendererProfile {
    WgpuDefault,
    WgpuDx12,
    WgpuVulkan,
    Glow,
}

impl RendererProfile {
    fn label(self) -> &'static str {
        match self {
            Self::WgpuDefault => "wgpu-default",
            Self::WgpuDx12 => "wgpu-dx12",
            Self::WgpuVulkan => "wgpu-vulkan",
            Self::Glow => "glow",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PresentProfile {
    AutoVsync,
    AutoNoVsync,
}

#[derive(Clone, Debug)]
struct RuntimeToggles {
    renderer: RendererProfile,
    present: PresentProfile,
    vsync: bool,
    run_and_return: bool,
    diagnostics: bool,
    /// `PM_SCREENSHOT=<file.png>`: save one frame to that file and quit (docs and visual checks).
    screenshot: Option<PathBuf>,
    /// `PM_WINDOW=<width>x<height>`: the window's inner size at start, in points.
    window_size: Option<Vec2>,
    /// `PM_OPEN=<view>`: what the window shows first (`overview`, `settings`, `activity`,
    /// `palette`, `stack`, `new`), for screenshots.
    open: Option<String>,
    /// `PM_SCREENSHOT_AFTER=<seconds>`: how long to let the stack run before the screenshot.
    screenshot_after: Duration,
}

impl RuntimeToggles {
    fn from_env() -> Self {
        let mut toggles = Self {
            renderer: default_renderer_profile(),
            present: default_present_profile(),
            vsync: default_vsync_enabled(),
            run_and_return: false,
            diagnostics: false,
            screenshot: None,
            window_size: None,
            open: None,
            screenshot_after: Duration::from_secs(3),
        };

        toggles.renderer = match std::env::var("PM_RENDERER")
            .unwrap_or_else(|_| toggles.renderer.label().to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "wgpu" | "wgpu-default" => RendererProfile::WgpuDefault,
            "wgpu-dx12" | "dx12" => RendererProfile::WgpuDx12,
            "wgpu-vulkan" | "vulkan" => RendererProfile::WgpuVulkan,
            "glow" | "opengl" => RendererProfile::Glow,
            _ => RendererProfile::WgpuDefault,
        };

        toggles.present = match std::env::var("PM_PRESENT_MODE")
            .unwrap_or_else(|_| default_present_label(toggles.present).to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "auto-no-vsync" | "no-vsync" | "immediate" => PresentProfile::AutoNoVsync,
            _ => PresentProfile::AutoVsync,
        };

        toggles.vsync = std::env::var("PM_VSYNC")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(matches!(toggles.present, PresentProfile::AutoVsync));

        toggles.run_and_return = std::env::var("PM_RUN_AND_RETURN")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        toggles.diagnostics = std::env::var("PM_DIAGNOSTICS")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        toggles.screenshot = std::env::var_os("PM_SCREENSHOT").map(PathBuf::from);
        toggles.window_size = std::env::var("PM_WINDOW").ok().and_then(|value| {
            let (w, h) = value.split_once('x')?;
            Some(Vec2::new(w.trim().parse().ok()?, h.trim().parse().ok()?))
        });
        toggles.open = std::env::var("PM_OPEN").ok();
        if let Some(seconds) = std::env::var("PM_SCREENSHOT_AFTER")
            .ok()
            .and_then(|value| value.trim().parse::<f32>().ok())
        {
            toggles.screenshot_after = Duration::from_secs_f32(seconds.max(0.5));
        }

        toggles
    }
}

fn default_renderer_profile() -> RendererProfile {
    #[cfg(windows)]
    {
        RendererProfile::WgpuDx12
    }

    #[cfg(not(windows))]
    {
        RendererProfile::WgpuDefault
    }
}

fn default_present_profile() -> PresentProfile {
    #[cfg(windows)]
    {
        PresentProfile::AutoNoVsync
    }

    #[cfg(not(windows))]
    {
        PresentProfile::AutoVsync
    }
}

fn default_vsync_enabled() -> bool {
    #[cfg(windows)]
    {
        false
    }

    #[cfg(not(windows))]
    {
        true
    }
}

fn default_present_label(present: PresentProfile) -> &'static str {
    match present {
        PresentProfile::AutoVsync => "auto-vsync",
        PresentProfile::AutoNoVsync => "auto-no-vsync",
    }
}

#[derive(Default)]
struct DiagnosticsState {
    start_time: Option<Instant>,
    updates: u64,
    slow_updates: u64,
    max_update_ms: f32,
    max_snapshot_ms: f32,
    last_summary: Option<Instant>,
    log_path: Option<PathBuf>,
    session_label: Option<String>,
    renderer_backend: Option<String>,
    renderer_adapter: Option<String>,
    last_viewport_pos: Option<Pos2>,
    motion_burst_start: Option<Instant>,
    motion_active_until: Option<Instant>,
    motion_updates: u64,
    motion_move_events: u64,
    motion_resize_events: u64,
    last_motion_fps: f32,
    last_motion_updates: u64,
    last_motion_move_events: u64,
    last_motion_resize_events: u64,
}

pub fn run(
    #[cfg(windows)] instance: &crate::single_instance::SingleInstance,
) -> eframe::Result<()> {
    let toggles = RuntimeToggles::from_env();

    let mut viewport = ViewportBuilder::default()
        .with_title("Process Manager")
        .with_inner_size(toggles.window_size.unwrap_or(Vec2::new(1280.0, 800.0)))
        .with_min_inner_size([420.0, 320.0]);

    if let Some(icon) = load_icon_data() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        renderer: renderer_backend(toggles.renderer),
        vsync: toggles.vsync,
        run_and_return: toggles.run_and_return,
        wgpu_options: wgpu_configuration(toggles.renderer, toggles.present),
        // Screenshots must start at the size they ask for, not the last window's.
        persist_window: toggles.window_size.is_none(),
        ..Default::default()
    };

    eframe::run_native(
        "Process Manager",
        native_options,
        Box::new(move |cc| {
            #[cfg(windows)]
            if let Some(hwnd) = extract_root_hwnd(cc) {
                instance.register_window(hwnd)?;
            }
            Ok(Box::new(ProcessManagerApp::new(cc, toggles.clone())))
        }),
    )
}

fn renderer_backend(profile: RendererProfile) -> eframe::Renderer {
    match profile {
        RendererProfile::Glow => eframe::Renderer::Glow,
        RendererProfile::WgpuDefault | RendererProfile::WgpuDx12 | RendererProfile::WgpuVulkan => {
            eframe::Renderer::Wgpu
        }
    }
}

fn wgpu_configuration(
    profile: RendererProfile,
    present: PresentProfile,
) -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;

    let mut config = eframe::egui_wgpu::WgpuConfiguration {
        present_mode: match present {
            PresentProfile::AutoVsync => wgpu::PresentMode::AutoVsync,
            PresentProfile::AutoNoVsync => wgpu::PresentMode::AutoNoVsync,
        },
        ..Default::default()
    };

    let mut create_new = WgpuSetupCreateNew::default();
    create_new.instance_descriptor.backends = match profile {
        RendererProfile::WgpuDx12 => wgpu::Backends::DX12,
        RendererProfile::WgpuVulkan => wgpu::Backends::VULKAN,
        _ => create_new.instance_descriptor.backends,
    };

    config.wgpu_setup = WgpuSetup::CreateNew(create_new);
    config
}

fn load_icon_data() -> Option<egui::IconData> {
    use image::imageops::FilterType;

    let image = image::load_from_memory(include_bytes!("../../assets/icon.png"))
        .ok()?
        .resize(64, 64, FilterType::Lanczos3)
        .to_rgba8();
    let (width, height) = image.dimensions();

    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// What the main area shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    /// The selected process: its header, stats and tabs.
    Process,
    /// Every process in one table, with the merged stack log.
    Overview,
    /// Stack name, log folder, local API, about.
    StackSettings,
    /// The settings form for a process that does not exist yet.
    NewProcess,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcessTab {
    Logs,
    Settings,
    Activity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LevelFilter {
    All,
    Errors,
    Warnings,
    Events,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverviewChip {
    All,
    Running,
    Stopped,
    Attention,
    Scheduled,
}

/// Where the user asked to go; held while unsaved settings wait for an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Nav {
    Process(String),
    Overview,
    StackSettings,
    NewProcess,
}

#[derive(Clone, Debug)]
struct LogSelection {
    process_id: String,
    /// Line numbers (stable for the session).
    anchor: u64,
    focus: u64,
}

impl LogSelection {
    fn range(&self) -> (u64, u64) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }
}

struct TimedFlash {
    started_at: Instant,
    until: Instant,
}

#[derive(Clone, Debug)]
enum SidebarRenderItem {
    Group(ProcessGroupConfig),
    Process {
        process: ProcessConfig,
        group_id: Option<String>,
        last_in_group: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SidebarDragItem {
    Process(String),
    Group(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SidebarDropTarget {
    TopLevel {
        before_process_id: Option<String>,
    },
    InGroup {
        group_id: String,
        before_process_id: Option<String>,
    },
}

#[derive(Clone, Debug)]
struct SidebarDropSlot {
    y: f32,
    left: f32,
    right: f32,
    target: SidebarDropTarget,
}

/// Rolled-up numbers for a set of processes (a group, the stack, a selection).
#[derive(Clone, Copy, Debug, Default)]
struct Rollup {
    total: usize,
    running: usize,
    busy: usize,
    attention: usize,
    stopped: usize,
    scheduled: usize,
    cpu: f32,
    memory: u64,
}

pub struct ProcessManagerApp {
    toggles: RuntimeToggles,
    runtime: Runtime,
    manager: Arc<ProcessManager>,
    rest_controller: Arc<RestServerController>,
    config: AppConfig,
    view: View,
    tab: ProcessTab,
    /// The process the console shows (and returns to).
    selected_process: Option<String>,
    /// The multi-selection: sidebar Shift/Ctrl-clicks and the overview's checkboxes.
    checked: Vec<String>,
    selection_anchor_process: Option<String>,
    /// Limits the overview to one group (opened from a group's menu).
    overview_group: Option<String>,
    overview_filter: String,
    overview_chip: OverviewChip,
    overview_collapsed: HashSet<String>,
    dragged_sidebar_item: Option<SidebarDragItem>,
    editor: Option<Editor>,
    pending_nav: Option<Nav>,
    delete_process_id: Option<String>,
    reload_processes_confirm_open: bool,
    rename_group: Option<(String, String)>,
    stack_form: StackSettingsForm,
    palette: Option<palette::PaletteState>,
    message: Option<(String, Instant)>,
    log_filter: String,
    log_level: LevelFilter,
    show_log_time: bool,
    wrap_logs: bool,
    follow_logs: bool,
    /// `log_total` when Follow turned off, to count lines that arrived since.
    unfollowed_at: u64,
    jump_to_latest: bool,
    log_selection: Option<LogSelection>,
    /// A line double-clicked to select its text: (process id, line number).
    frozen_log_line: Option<(String, u64)>,
    focus_log_filter: bool,
    /// Narrow windows fold the command and badges away until asked.
    details_open: bool,
    last_error_version: u64,
    last_process_error_versions: HashMap<String, u64>,
    process_row_flashes: HashMap<String, TimedFlash>,
    current_title: String,
    #[cfg(windows)]
    root_hwnd: Option<windows_sys::Win32::Foundation::HWND>,
    #[cfg(windows)]
    taskbar_icon_applied: bool,
    #[cfg(windows)]
    native_caption_applied: bool,
    #[cfg(windows)]
    taskbar_big_icon_handle: Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON>,
    #[cfg(windows)]
    taskbar_small_icon_handle: Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON>,
    last_viewport_size: Option<Vec2>,
    last_manager_version: u64,
    last_rest_config_reload_version: u64,
    snapshot_key: (Option<String>, bool),
    runtime_snapshot: UiRuntimeSnapshot,
    screenshot_requested: bool,
    launched_at: Instant,
    diagnostics: DiagnosticsState,
}

impl ProcessManagerApp {
    fn new(cc: &eframe::CreationContext<'_>, toggles: RuntimeToggles) -> Self {
        theme::apply(&cc.egui_ctx);

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("pm-runtime")
            .build()
            .expect("failed to build tokio runtime");

        let config = AppConfig::load();
        let manager = Arc::new(ProcessManager::new());
        manager.set_log_directory(config.log_directory.clone());
        manager.init_from_config(&config.processes);
        manager.start_background_tasks();

        let rest_controller = Arc::new(RestServerController::new(manager.clone()));
        {
            let _guard = runtime.enter();
            rest_controller.apply_config(config.stack_name.clone(), config.remote_control.clone());
        }
        manager.start_auto_start_processes();

        let selected_process = config.processes.first().map(|process| process.id.clone());
        let runtime_snapshot =
            manager.build_ui_snapshot(selected_process.as_deref(), UI_LOG_LIMIT, false);
        let last_manager_version = manager.current_version();
        let current_title = window_title(&config.stack_name);
        let stack_form = StackSettingsForm::from_config(&config);
        let last_process_error_versions = manager.error_versions();
        cc.egui_ctx
            .send_viewport_cmd(ViewportCommand::Title(current_title.clone()));
        let (renderer_backend, renderer_adapter) = cc
            .wgpu_render_state
            .as_ref()
            .map(|render_state| {
                let info = render_state.adapter.get_info();
                (Some(format!("{:?}", info.backend)), Some(info.name.clone()))
            })
            .unwrap_or((None, None));
        let session_label = std::env::var("PM_DIAG_SESSION")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let mut app = Self {
            toggles,
            runtime,
            manager,
            rest_controller,
            config,
            view: View::Process,
            tab: ProcessTab::Logs,
            selected_process: selected_process.clone(),
            checked: Vec::new(),
            selection_anchor_process: selected_process.clone(),
            overview_group: None,
            overview_filter: String::new(),
            overview_chip: OverviewChip::All,
            overview_collapsed: HashSet::new(),
            dragged_sidebar_item: None,
            editor: None,
            pending_nav: None,
            delete_process_id: None,
            reload_processes_confirm_open: false,
            rename_group: None,
            stack_form,
            palette: None,
            message: None,
            log_filter: String::new(),
            log_level: LevelFilter::All,
            show_log_time: true,
            wrap_logs: false,
            follow_logs: true,
            unfollowed_at: 0,
            jump_to_latest: false,
            log_selection: None,
            frozen_log_line: None,
            focus_log_filter: false,
            details_open: false,
            last_error_version: 0,
            last_process_error_versions,
            process_row_flashes: HashMap::new(),
            current_title,
            #[cfg(windows)]
            root_hwnd: extract_root_hwnd(cc),
            #[cfg(windows)]
            taskbar_icon_applied: false,
            #[cfg(windows)]
            native_caption_applied: false,
            #[cfg(windows)]
            taskbar_big_icon_handle: None,
            #[cfg(windows)]
            taskbar_small_icon_handle: None,
            last_viewport_size: None,
            last_manager_version,
            last_rest_config_reload_version: 0,
            snapshot_key: (selected_process, false),
            runtime_snapshot,
            screenshot_requested: false,
            launched_at: Instant::now(),
            diagnostics: DiagnosticsState {
                session_label,
                renderer_backend,
                renderer_adapter,
                ..DiagnosticsState::default()
            },
        };
        app.apply_open_toggle();
        app
    }

    /// `PM_OPEN`: start on a given screen, for screenshots of every state.
    fn apply_open_toggle(&mut self) {
        let Some(open) = self.toggles.open.clone() else {
            return;
        };
        let (what, arg) = open.split_once(':').unwrap_or((open.as_str(), ""));
        if !arg.is_empty() {
            if let Some(process) = self
                .config
                .processes
                .iter()
                .find(|process| process.name.eq_ignore_ascii_case(arg) || process.id == arg)
            {
                self.selected_process = Some(process.id.clone());
            }
        }
        match what {
            "overview" => self.view = View::Overview,
            "stack" => self.view = View::StackSettings,
            "new" => self.open_new_process(),
            "settings" => {
                self.view = View::Process;
                self.tab = ProcessTab::Settings;
            }
            "activity" => {
                self.view = View::Process;
                self.tab = ProcessTab::Activity;
            }
            "palette" => self.palette = Some(palette::PaletteState::default()),
            _ => {}
        }
    }

    fn update_title(&mut self, ctx: &Context) {
        let next = window_title(&self.config.stack_name);
        if next != self.current_title {
            self.current_title = next.clone();
            ctx.send_viewport_cmd(ViewportCommand::Title(next));
        }
    }

    /// A message in the footer for a few seconds.
    fn set_banner(&mut self, message: impl Into<String>) {
        self.message = Some((
            message.into(),
            Instant::now() + Duration::from_secs(MESSAGE_SECONDS),
        ));
    }

    fn visible_message(&mut self) -> Option<String> {
        if self
            .message
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            self.message = None;
        }
        self.message.as_ref().map(|(message, _)| message.clone())
    }

    fn trigger_process_flash(&mut self, process_id: &str) {
        if self.config.process_error_flash_seconds == 0 {
            return;
        }

        let started_at = Instant::now();
        self.process_row_flashes.insert(
            process_id.to_string(),
            TimedFlash {
                started_at,
                until: started_at + Duration::from_secs(self.config.process_error_flash_seconds),
            },
        );
    }

    fn process_row_flash_fade(&self, process_id: &str, now: Instant) -> Option<f32> {
        let flash = self.process_row_flashes.get(process_id)?;
        if now >= flash.until {
            return None;
        }

        let total = flash.until.saturating_duration_since(flash.started_at);
        if total.is_zero() {
            return Some(1.0);
        }

        Some(
            (flash.until.saturating_duration_since(now).as_secs_f32() / total.as_secs_f32())
                .clamp(0.0, 1.0),
        )
    }

    fn any_process_row_flash_active(&self, now: Instant) -> bool {
        self.process_row_flashes
            .values()
            .any(|flash| now < flash.until)
    }

    /// 0..1: how strongly a sidebar row glows red for a fresh error.
    fn process_row_flash_intensity(&self, ctx: &Context, process_id: &str) -> f32 {
        let Some(fade) = self.process_row_flash_fade(process_id, Instant::now()) else {
            return 0.0;
        };

        let tail_fade = if fade > 0.18 { 1.0 } else { fade / 0.18 };
        let pulse =
            0.6 + ((((ctx.input(|input| input.time) as f32) * 8.5).sin() * 0.5) + 0.5) * 0.4;
        pulse * tail_fade.clamp(0.0, 1.0)
    }

    fn sync_process_error_flashes(&mut self) {
        let current_versions = self.manager.error_versions();
        for (process_id, current_version) in &current_versions {
            let previous_version = self
                .last_process_error_versions
                .get(process_id)
                .copied()
                .unwrap_or(0);
            if *current_version > previous_version {
                self.trigger_process_flash(process_id);
            }
        }
        self.last_process_error_versions = current_versions;
    }

    fn ensure_valid_selection(&mut self) {
        let valid: HashSet<&str> = self
            .config
            .processes
            .iter()
            .map(|process| process.id.as_str())
            .collect();
        self.checked.retain(|id| valid.contains(id.as_str()));
        if self
            .selected_process
            .as_deref()
            .is_some_and(|id| !valid.contains(id))
        {
            self.selected_process = None;
        }
        if self
            .selection_anchor_process
            .as_deref()
            .is_some_and(|id| !valid.contains(id))
        {
            self.selection_anchor_process = None;
        }
        if self
            .overview_group
            .as_ref()
            .is_some_and(|id| self.config.get_group(id).is_none())
        {
            self.overview_group = None;
        }
        if self.selected_process.is_none() {
            self.selected_process = self.config.processes.first().map(|p| p.id.clone());
        }
        if let Some(editor) = &self.editor {
            if editor
                .process_id
                .as_deref()
                .is_some_and(|id| !valid.contains(id))
            {
                self.editor = None;
            }
        }
    }

    fn persist_config(&mut self) {
        if let Err(err) = self.config.save() {
            self.set_banner(err);
        }
    }

    fn apply_rest_config(&self) {
        let _guard = self.runtime.enter();
        self.rest_controller.apply_config(
            self.config.stack_name.clone(),
            self.config.remote_control.clone(),
        );
    }

    // ---------- navigation ----------

    /// Goes somewhere, asking first when it would leave unsaved settings behind.
    fn navigate(&mut self, nav: Nav) {
        if self.leaves_unsaved_editor(&nav) {
            self.pending_nav = Some(nav);
            return;
        }
        self.go(nav);
    }

    fn leaves_unsaved_editor(&self, nav: &Nav) -> bool {
        let Some(editor) = &self.editor else {
            return false;
        };
        if !editor.is_dirty() {
            return false;
        }
        match (nav, &editor.process_id) {
            (Nav::Process(id), Some(editing)) => id != editing,
            (Nav::NewProcess, None) => false,
            _ => true,
        }
    }

    fn go(&mut self, nav: Nav) {
        self.pending_nav = None;
        match nav {
            Nav::Process(id) => {
                if self.selected_process.as_deref() != Some(id.as_str()) {
                    self.log_selection = None;
                    self.follow_logs = true;
                }
                if self
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.process_id.as_deref() != Some(id.as_str()))
                {
                    self.editor = None;
                }
                self.selected_process = Some(id.clone());
                self.selection_anchor_process = Some(id);
                self.checked.clear();
                self.view = View::Process;
                if self.tab == ProcessTab::Settings {
                    self.ensure_editor();
                }
            }
            Nav::Overview => {
                self.drop_clean_editor();
                self.view = View::Overview;
            }
            Nav::StackSettings => {
                self.drop_clean_editor();
                self.stack_form = StackSettingsForm::from_config(&self.config);
                self.view = View::StackSettings;
            }
            Nav::NewProcess => {
                self.editor = Some(Editor::new_process());
                self.view = View::NewProcess;
            }
        }
        self.refresh_runtime_snapshot(true);
    }

    fn drop_clean_editor(&mut self) {
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| !editor.is_dirty())
        {
            self.editor = None;
        }
    }

    fn open_new_process(&mut self) {
        self.navigate(Nav::NewProcess);
    }

    fn select_process(&mut self, process_id: String) {
        self.navigate(Nav::Process(process_id));
    }

    fn set_tab(&mut self, tab: ProcessTab) {
        self.tab = tab;
        if tab == ProcessTab::Settings {
            self.ensure_editor();
        }
        if tab == ProcessTab::Logs {
            self.follow_logs = true;
        }
    }

    /// The editor for the selected process, created on first use and kept while it is dirty.
    fn ensure_editor(&mut self) {
        let Some(id) = self.selected_process.clone() else {
            return;
        };
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.process_id.as_deref() == Some(id.as_str()))
        {
            return;
        }
        if let Some(process) = self.process_config(&id) {
            self.editor = Some(Editor::for_process(&process));
        }
    }

    /// Sidebar Shift-click: the range from the anchor to `process_id` goes into the selection.
    fn extend_selection_to(&mut self, process_id: String) {
        let anchor = self
            .selection_anchor_process
            .clone()
            .or_else(|| self.selected_process.clone())
            .unwrap_or_else(|| process_id.clone());
        self.checked = self.process_ids_between(&anchor, &process_id);
        self.navigate(Nav::Overview);
    }

    /// Sidebar Ctrl-click: one process in or out of the selection.
    fn toggle_checked(&mut self, process_id: &str) {
        if self.checked.is_empty() {
            if let Some(current) = self.selected_process.clone() {
                if current != process_id {
                    self.checked.push(current);
                }
            }
        }
        if let Some(index) = self.checked.iter().position(|id| id == process_id) {
            self.checked.remove(index);
        } else {
            self.checked.push(process_id.to_string());
        }
        self.selection_anchor_process = Some(process_id.to_string());
        if self.view != View::Overview {
            self.navigate(Nav::Overview);
        }
    }

    fn is_checked(&self, process_id: &str) -> bool {
        self.checked.iter().any(|id| id == process_id)
    }

    /// The process ids in sidebar order, as the keyboard walks them.
    fn ordered_process_ids(&self) -> Vec<String> {
        let mut ids = Vec::new();
        for item in self.sidebar_items() {
            if let SidebarRenderItem::Process { process, .. } = item {
                ids.push(process.id);
            }
        }
        ids
    }

    fn select_adjacent(&mut self, step: isize) {
        let ids = self.ordered_process_ids();
        if ids.is_empty() {
            return;
        }
        let current = self
            .selected_process
            .as_ref()
            .and_then(|id| ids.iter().position(|candidate| candidate == id));
        let next = match current {
            Some(index) => (index as isize + step).clamp(0, ids.len() as isize - 1) as usize,
            None => 0,
        };
        self.select_process(ids[next].clone());
    }

    fn process_ids_between(&self, first_id: &str, second_id: &str) -> Vec<String> {
        let ids = self.ordered_process_ids();
        let Some(first_index) = ids.iter().position(|id| id == first_id) else {
            return vec![second_id.to_string()];
        };
        let Some(second_index) = ids.iter().position(|id| id == second_id) else {
            return vec![second_id.to_string()];
        };
        let (start, end) = if first_index <= second_index {
            (first_index, second_index)
        } else {
            (second_index, first_index)
        };
        ids[start..=end].to_vec()
    }

    // ---------- lookups ----------

    fn process_config(&self, process_id: &str) -> Option<ProcessConfig> {
        self.manager
            .get_process_config(process_id)
            .or_else(|| self.config.get_process(process_id).cloned())
    }

    fn selected_process_config(&self) -> Option<ProcessConfig> {
        self.selected_process
            .as_ref()
            .and_then(|id| self.process_config(id))
    }

    fn status_of(&self, process_id: &str) -> ProcessStatus {
        self.runtime_snapshot
            .statuses
            .get(process_id)
            .cloned()
            .unwrap_or(ProcessStatus::Stopped)
    }

    fn info_of(&self, process_id: &str) -> Option<&ProcessRuntimeInfo> {
        self.runtime_snapshot.info.get(process_id)
    }

    fn usage_of(&self, process_id: &str) -> Option<ProcessResourceUsage> {
        self.runtime_snapshot
            .resource_usage
            .get(process_id)
            .copied()
    }

    fn status_view(&self, process: &ProcessConfig) -> StatusView {
        format::status_view(
            process,
            &self.status_of(&process.id),
            self.info_of(&process.id),
            self.usage_of(&process.id),
        )
    }

    fn is_live(&self, process_id: &str) -> bool {
        matches!(
            self.status_of(process_id),
            ProcessStatus::Running | ProcessStatus::Starting
        )
    }

    fn rollup<'a>(&self, ids: impl IntoIterator<Item = &'a String>) -> Rollup {
        let mut rollup = Rollup::default();
        for id in ids {
            let Some(process) = self.config.get_process(id) else {
                continue;
            };
            rollup.total += 1;
            let view = self.status_view(process);
            match self.status_of(id) {
                ProcessStatus::Running => rollup.running += 1,
                ProcessStatus::Starting | ProcessStatus::Stopping => rollup.busy += 1,
                _ if view.alert => rollup.attention += 1,
                _ if process.scheduled_run.enabled => rollup.scheduled += 1,
                _ => rollup.stopped += 1,
            }
            if let Some(usage) = self.usage_of(id) {
                rollup.cpu += usage.cpu_percent.unwrap_or(0.0);
                rollup.memory += usage.memory_bytes.unwrap_or(0);
            }
        }
        rollup
    }

    fn stack_rollup(&self) -> Rollup {
        let ids: Vec<String> = self.config.processes.iter().map(|p| p.id.clone()).collect();
        self.rollup(ids.iter())
    }

    fn sidebar_items(&self) -> Vec<SidebarRenderItem> {
        let mut items = Vec::new();
        let mut rendered_group_ids = HashSet::new();
        let mut rendered_process_ids = HashSet::new();

        for process in &self.config.processes {
            if rendered_process_ids.contains(&process.id) {
                continue;
            }

            if let Some(group) = self.config.group_for_process(&process.id) {
                if rendered_group_ids.insert(group.id.clone()) {
                    items.push(SidebarRenderItem::Group(group.clone()));
                    for process_id in &group.process_ids {
                        rendered_process_ids.insert(process_id.clone());
                    }
                    if group.expanded {
                        for (member_index, process_id) in group.process_ids.iter().enumerate() {
                            if let Some(process) = self.config.get_process(process_id) {
                                items.push(SidebarRenderItem::Process {
                                    process: process.clone(),
                                    group_id: Some(group.id.clone()),
                                    last_in_group: member_index + 1 == group.process_ids.len(),
                                });
                            }
                        }
                    }
                }
            } else {
                rendered_process_ids.insert(process.id.clone());
                items.push(SidebarRenderItem::Process {
                    process: process.clone(),
                    group_id: None,
                    last_in_group: false,
                });
            }
        }

        items
    }

    // ---------- actions ----------

    fn start_process(&mut self, process_id: &str) {
        self.manager.start_process(process_id);
        self.refresh_runtime_snapshot(true);
    }

    fn stop_process(&mut self, process_id: &str) {
        self.manager.stop_process(process_id);
        self.refresh_runtime_snapshot(true);
    }

    /// Restarts wait for the stop, so they run off the UI thread.
    fn restart_process(&mut self, process_id: &str) {
        let manager = self.manager.clone();
        let id = process_id.to_string();
        std::thread::spawn(move || manager.restart_process(&id));
    }

    fn toggle_process(&mut self, process_id: &str) {
        if self.is_live(process_id) {
            self.stop_process(process_id);
        } else {
            self.start_process(process_id);
        }
    }

    fn stack_action(&mut self, action: StackAction) {
        let key = |p: &ProcessConfig| match action {
            StackAction::Start => p.respond_to_start_all,
            StackAction::Stop => p.respond_to_stop_all,
            StackAction::Restart => p.respond_to_restart_all,
        };
        let skipped: Vec<String> = self
            .config
            .processes
            .iter()
            .filter(|p| !key(p))
            .map(|p| p.name.clone())
            .collect();
        let manager = self.manager.clone();
        let label = match action {
            StackAction::Start => {
                manager.start_all();
                "Start all"
            }
            StackAction::Stop => {
                manager.stop_all();
                "Stop all"
            }
            StackAction::Restart => {
                std::thread::spawn(move || manager.restart_all());
                "Restart all"
            }
        };
        let note = if skipped.is_empty() {
            String::new()
        } else {
            format!(" · skipped by setting: {}", skipped.join(", "))
        };
        self.set_banner(format!("{label}{note}"));
        self.refresh_runtime_snapshot(true);
    }

    fn checked_action(&mut self, action: StackAction) {
        let ids = self.checked.clone();
        match action {
            StackAction::Start => ids.iter().for_each(|id| self.manager.start_process(id)),
            StackAction::Stop => ids.iter().for_each(|id| self.manager.stop_process(id)),
            StackAction::Restart => {
                let manager = self.manager.clone();
                std::thread::spawn(move || {
                    ids.iter().for_each(|id| manager.stop_process(id));
                    ids.iter()
                        .for_each(|id| wait_for_process_stop(&manager, id));
                    ids.iter().for_each(|id| manager.start_process(id));
                });
            }
        }
        self.refresh_runtime_snapshot(true);
    }

    fn group_action(&mut self, group_id: &str, action: StackAction) {
        let Some(group) = self.config.get_group(group_id).cloned() else {
            return;
        };
        let manager = self.manager.clone();
        let ids = group.process_ids.clone();
        let cause = format!("Group {}", group.name);
        match action {
            StackAction::Start => {
                manager.start_processes_respecting_start_all(&ids, &cause);
            }
            StackAction::Stop => {
                manager.stop_processes_respecting_stop_all(&ids, &cause);
            }
            StackAction::Restart => {
                std::thread::spawn(move || {
                    manager.restart_processes_respecting_restart_all(&ids, &cause);
                });
            }
        }
        self.refresh_runtime_snapshot(true);
    }

    fn reload_process_from_disk(&mut self, process_id: &str) {
        let config = match AppConfig::load_from_disk() {
            Ok(mut config) => {
                config.normalize();
                config
            }
            Err(err) => {
                self.set_banner(format!("Failed to reload processes.json: {}", err));
                return;
            }
        };

        let Some(updated) = config
            .processes
            .iter()
            .find(|process| process.id == process_id)
        else {
            self.set_banner(format!("{} is not in processes.json.", process_id));
            return;
        };

        if !self.manager.reload_process_from_config(updated.clone()) {
            self.set_banner(format!(
                "Process '{}' is not currently managed in this session.",
                process_id
            ));
            return;
        }

        self.config.update_process(process_id, updated.clone());
        self.last_process_error_versions = self.manager.error_versions();
        self.process_row_flashes.remove(process_id);
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.process_id.as_deref() == Some(process_id))
        {
            self.editor = Some(Editor::for_process(updated));
        }
        self.refresh_runtime_snapshot(true);
        self.set_banner(format!("Reloaded {} from processes.json", updated.name));
    }

    fn reload_processes_from_disk(&mut self) {
        let config = match AppConfig::load_from_disk() {
            Ok(mut config) => {
                config.normalize();
                config
            }
            Err(err) => {
                self.set_banner(format!("Failed to reload processes.json: {}", err));
                return;
            }
        };

        self.manager.set_log_directory(config.log_directory.clone());
        self.manager.reload_from_config(&config.processes);
        self.config = config;
        self.editor = None;
        self.apply_rest_config();
        self.last_process_error_versions = self.manager.error_versions();
        self.process_row_flashes.clear();
        self.ensure_valid_selection();
        self.refresh_runtime_snapshot(true);
        self.set_banner("Reloaded processes.json");
    }

    fn toggle_api_enabled(&mut self) {
        self.config.remote_control.enabled = !self.config.remote_control.enabled;
        self.persist_config();
        self.apply_rest_config();
        self.stack_form.api_enabled = self.config.remote_control.enabled;
        self.set_banner(if self.config.remote_control.enabled {
            format!("Local API on at {}", self.api_address())
        } else {
            "Local API off".to_string()
        });
    }

    fn api_address(&self) -> String {
        std::net::SocketAddr::new(
            self.config.remote_control.bind_address,
            self.config.remote_control.port,
        )
        .to_string()
    }

    fn copy_agent_skill(&mut self) {
        let payload = build_agent_bootstrap(
            &self.config.stack_name,
            &self.config.remote_control,
            &self.manager.list_processes(),
            &self.config.groups,
        );

        match copy_text_to_clipboard(&payload) {
            Ok(()) => self.set_banner(format!(
                "Copied the agent skill: address, endpoints and {} process ids",
                self.config.processes.len()
            )),
            Err(err) => self.set_banner(err),
        }
    }

    fn copy_text(&mut self, text: &str, done: impl Into<String>) {
        match copy_text_to_clipboard(text) {
            Ok(()) => self.set_banner(done),
            Err(err) => self.set_banner(err),
        }
    }

    fn open_folder(&mut self, path: PathBuf) {
        let path = if path.as_os_str().is_empty() {
            crate::platform::data_directory()
        } else if path.is_relative() {
            crate::platform::data_directory().join(path)
        } else {
            path
        };
        #[cfg(windows)]
        let opened = std::process::Command::new("explorer").arg(&path).spawn();
        #[cfg(target_os = "macos")]
        let opened = std::process::Command::new("open").arg(&path).spawn();
        #[cfg(not(any(windows, target_os = "macos")))]
        let opened = std::process::Command::new("xdg-open").arg(&path).spawn();
        match opened {
            Ok(_) => self.set_banner(format!("Opened {}", path.display())),
            Err(err) => self.set_banner(format!("Could not open {}: {err}", path.display())),
        }
    }

    /// Where a process's session logs go: `<log dir>/Process Manager logs/<name>`.
    fn process_log_folder(&self, process: &ProcessConfig) -> PathBuf {
        let root = {
            let trimmed = self.config.log_directory.trim();
            let data = crate::platform::data_directory();
            if trimmed.is_empty() || trimmed == "." {
                data
            } else if std::path::Path::new(trimmed).is_absolute() {
                PathBuf::from(trimmed)
            } else {
                data.join(trimmed)
            }
        };
        let name: String = process
            .name
            .chars()
            .map(|ch| match ch {
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
                ch if ch.is_control() => '_',
                ch => ch,
            })
            .collect();
        root.join("Process Manager logs")
            .join(name.trim().trim_matches('.'))
    }

    fn delete_process(&mut self, process_id: &str) {
        let name = self
            .config
            .get_process(process_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        self.manager.remove_process(process_id);
        self.config.remove_process(process_id);
        self.last_process_error_versions.remove(process_id);
        self.process_row_flashes.remove(process_id);
        self.persist_config();
        if self.selected_process.as_deref() == Some(process_id) {
            self.selected_process = None;
        }
        self.checked.retain(|selected_id| selected_id != process_id);
        self.ensure_valid_selection();
        self.set_banner(format!("Deleted {name}"));
    }

    fn duplicate_process(&mut self, process_id: &str) {
        let Some(duplicate) = self.config.duplicate_process(process_id) else {
            self.set_banner("Process no longer exists.");
            return;
        };

        self.manager.add_process(duplicate.clone());
        self.persist_config();
        self.last_process_error_versions = self.manager.error_versions();
        self.select_process(duplicate.id.clone());
        self.refresh_runtime_snapshot(true);
        self.set_banner(format!("Duplicated as {}, stopped", duplicate.name));
    }

    fn move_process_up(&mut self, process_id: &str) {
        if self.config.move_process_up(process_id) {
            self.persist_config();
        }
    }

    fn move_process_down(&mut self, process_id: &str) {
        if self.config.move_process_down(process_id) {
            self.persist_config();
        }
    }

    fn apply_sidebar_drop(&mut self, dragged: SidebarDragItem, target: SidebarDropTarget) {
        let changed = match (dragged, target) {
            (
                SidebarDragItem::Process(process_id),
                SidebarDropTarget::TopLevel { before_process_id },
            ) => self
                .config
                .move_process_to_top_level(&process_id, before_process_id.as_deref()),
            (
                SidebarDragItem::Process(process_id),
                SidebarDropTarget::InGroup {
                    group_id,
                    before_process_id,
                },
            ) => self.config.move_process_into_group(
                &process_id,
                &group_id,
                before_process_id.as_deref(),
            ),
            (
                SidebarDragItem::Group(group_id),
                SidebarDropTarget::TopLevel { before_process_id },
            ) => self
                .config
                .move_group_to_top_level(&group_id, before_process_id.as_deref()),
            (SidebarDragItem::Group(_), SidebarDropTarget::InGroup { .. }) => false,
        };

        if changed {
            self.persist_config();
        }
    }

    fn valid_sidebar_drop_slot(&self, dragged: &SidebarDragItem, slot: &SidebarDropSlot) -> bool {
        match (dragged, &slot.target) {
            (
                SidebarDragItem::Process(process_id),
                SidebarDropTarget::TopLevel { before_process_id },
            ) => {
                before_process_id.as_deref() != Some(process_id.as_str())
                    || self.config.group_for_process(process_id).is_some()
            }
            (
                SidebarDragItem::Process(process_id),
                SidebarDropTarget::InGroup {
                    before_process_id, ..
                },
            ) => before_process_id.as_deref() != Some(process_id.as_str()),
            (
                SidebarDragItem::Group(group_id),
                SidebarDropTarget::TopLevel { before_process_id },
            ) => before_process_id.as_deref().map_or(true, |before_id| {
                self.config.get_group(group_id).map_or(true, |group| {
                    !group
                        .process_ids
                        .iter()
                        .any(|process_id| process_id == before_id)
                })
            }),
            (SidebarDragItem::Group(_), SidebarDropTarget::InGroup { .. }) => false,
        }
    }

    fn nearest_sidebar_drop_slot(
        &self,
        dragged: &SidebarDragItem,
        slots: &[SidebarDropSlot],
        pointer_pos: egui::Pos2,
    ) -> Option<SidebarDropSlot> {
        slots
            .iter()
            .filter(|slot| self.valid_sidebar_drop_slot(dragged, slot))
            .min_by(|a, b| {
                (a.y - pointer_pos.y)
                    .abs()
                    .partial_cmp(&(b.y - pointer_pos.y).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .cloned()
    }

    fn group_checked_processes(&mut self) {
        if self.checked.len() < 2 {
            self.set_banner("Select two or more processes to make a group.");
            return;
        }

        let name = self.next_group_name();
        let selected = self.checked.clone();
        let Some(group) = self
            .config
            .create_group_from_processes(name.clone(), &selected)
        else {
            self.set_banner("Select two or more valid processes to make a group.");
            return;
        };

        self.persist_config();
        self.checked.clear();
        self.overview_group = Some(group.id.clone());
        self.rename_group = Some((group.id, name.clone()));
        self.set_banner(format!("Made {name}; name it"));
    }

    fn next_group_name(&self) -> String {
        for index in 1.. {
            let candidate = format!("Group {}", index);
            if !self
                .config
                .groups
                .iter()
                .any(|group| group.name == candidate)
            {
                return candidate;
            }
        }

        "Group".to_string()
    }

    fn toggle_group_expanded(&mut self, group_id: &str) {
        let Some(group) = self.config.get_group(group_id) else {
            return;
        };
        let expanded = !group.expanded;
        if self.config.set_group_expanded(group_id, expanded) {
            self.persist_config();
        }
    }

    fn rename_group_to(&mut self, group_id: &str, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.set_banner("A group needs a name.");
            return;
        }
        if let Some(group) = self
            .config
            .groups
            .iter_mut()
            .find(|group| group.id == group_id)
        {
            group.name = name.to_string();
            self.persist_config();
            self.set_banner(format!("Renamed the group to {name}"));
        }
    }

    fn ungroup(&mut self, group_id: &str) {
        if self.config.remove_group(group_id) {
            self.persist_config();
            if self.overview_group.as_deref() == Some(group_id) {
                self.overview_group = None;
            }
            self.set_banner("Ungrouped; the processes stay");
        }
    }

    // ---------- runtime ----------

    fn refresh_runtime_snapshot(&mut self, force: bool) {
        let current_version = self.manager.current_version();
        let key = (self.selected_process.clone(), self.view == View::Overview);
        let key_changed = self.snapshot_key != key;
        if !force && !key_changed && current_version == self.last_manager_version {
            return;
        }

        let started = Instant::now();
        self.runtime_snapshot =
            self.manager
                .build_ui_snapshot(key.0.as_deref(), UI_LOG_LIMIT, key.1);
        self.last_manager_version = current_version;
        self.snapshot_key = key;
        self.record_snapshot_refresh(started.elapsed());
    }

    fn sync_rest_config_reload(&mut self) {
        let event = self.rest_controller.config_reload_event();
        if event.version == self.last_rest_config_reload_version {
            return;
        }
        self.last_rest_config_reload_version = event.version;

        let Some(config) = event.config else {
            return;
        };

        if let Some(process_id) = event.process_id {
            if let Some(process) = config.get_process(&process_id).cloned() {
                self.config.update_process(&process_id, process);
            }
        } else {
            self.config = config;
            self.stack_form = StackSettingsForm::from_config(&self.config);
            self.last_process_error_versions = self.manager.error_versions();
            self.process_row_flashes.clear();
            self.editor = None;
            self.ensure_valid_selection();
        }
        self.refresh_runtime_snapshot(true);
    }

    fn selected_log_range(&self, process_id: &str) -> Option<(u64, u64)> {
        self.log_selection
            .as_ref()
            .filter(|selection| selection.process_id == process_id)
            .map(LogSelection::range)
    }

    fn select_log_line(&mut self, process_id: &str, line: u64, extend: bool) {
        if extend {
            if let Some(selection) = self
                .log_selection
                .as_mut()
                .filter(|selection| selection.process_id == process_id)
            {
                selection.focus = line;
                return;
            }
        }
        if self
            .log_selection
            .as_ref()
            .is_some_and(|s| s.process_id == process_id && s.anchor == line && s.focus == line)
        {
            self.log_selection = None;
            return;
        }
        self.log_selection = Some(LogSelection {
            process_id: process_id.to_string(),
            anchor: line,
            focus: line,
        });
    }

    fn copy_selected_logs(&mut self) {
        let Some(process_id) = self.selected_process.clone() else {
            return;
        };
        let Some((start, end)) = self.selected_log_range(&process_id) else {
            return;
        };
        let first = self.runtime_snapshot.selected_first_line;
        let lines: Vec<String> = self
            .runtime_snapshot
            .selected_logs
            .iter()
            .enumerate()
            .filter(|(index, _)| (start..=end).contains(&(first + *index as u64)))
            .map(|(_, line)| {
                format!(
                    "{}  {}",
                    line.at.format("%H:%M:%S%.3f"),
                    format::classify(&line.text).2
                )
            })
            .collect();
        if lines.is_empty() {
            self.set_banner("Those lines are no longer in memory.");
            self.log_selection = None;
            return;
        }
        let count = lines.len();
        self.copy_text(
            &lines.join("\n"),
            format!(
                "Copied {count} {}",
                if count == 1 { "line" } else { "lines" }
            ),
        );
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        let typing = ctx.wants_keyboard_input();
        let mut actions = Vec::new();
        ctx.input(|input| {
            let cmd = input.modifiers.command;
            let shift = input.modifiers.shift;
            let pressed = |key| input.key_pressed(key);
            if cmd && pressed(Key::K) {
                actions.push(Shortcut::Palette);
            }
            if self.palette.is_some() {
                return;
            }
            if cmd && shift && pressed(Key::S) {
                actions.push(Shortcut::Stack(StackAction::Start));
            } else if cmd && shift && pressed(Key::X) {
                actions.push(Shortcut::Stack(StackAction::Stop));
            } else if cmd && shift && pressed(Key::R) {
                actions.push(Shortcut::Stack(StackAction::Restart));
            } else if cmd && pressed(Key::R) {
                actions.push(Shortcut::RestartSelected);
            } else if cmd && pressed(Key::Enter) {
                actions.push(Shortcut::ToggleSelected);
            } else if cmd && pressed(Key::N) {
                actions.push(Shortcut::NewProcess);
            } else if cmd && pressed(Key::Comma) {
                actions.push(Shortcut::Tab(ProcessTab::Settings));
            } else if cmd && pressed(Key::Num0) {
                actions.push(Shortcut::Overview);
            } else if cmd && pressed(Key::F) {
                actions.push(Shortcut::FocusFilter);
            }
            if typing {
                return;
            }
            if pressed(Key::Slash) {
                actions.push(Shortcut::FocusFilter);
            }
            if cmd && pressed(Key::C) {
                actions.push(Shortcut::CopyLines);
            }
            if pressed(Key::Escape) {
                actions.push(Shortcut::Escape);
            }
            if !cmd && pressed(Key::ArrowDown) {
                actions.push(Shortcut::Next(1));
            }
            if !cmd && pressed(Key::ArrowUp) {
                actions.push(Shortcut::Next(-1));
            }
        });

        for action in actions {
            match action {
                Shortcut::Palette => {
                    self.palette = match self.palette {
                        Some(_) => None,
                        None => Some(palette::PaletteState::default()),
                    }
                }
                Shortcut::Stack(action) => self.stack_action(action),
                Shortcut::RestartSelected => {
                    if let Some(id) = self.selected_process.clone() {
                        self.restart_process(&id);
                    }
                }
                Shortcut::ToggleSelected => {
                    if let Some(id) = self.selected_process.clone() {
                        self.toggle_process(&id);
                    }
                }
                Shortcut::NewProcess => self.open_new_process(),
                Shortcut::Tab(tab) => {
                    if let Some(id) = self.selected_process.clone() {
                        self.navigate(Nav::Process(id));
                        self.set_tab(tab);
                    }
                }
                Shortcut::Overview => self.navigate(Nav::Overview),
                Shortcut::FocusFilter => {
                    if self.view == View::Process {
                        self.set_tab(ProcessTab::Logs);
                        self.focus_log_filter = true;
                    }
                }
                Shortcut::CopyLines => self.copy_selected_logs(),
                Shortcut::Escape => {
                    if self.log_selection.is_some() {
                        self.log_selection = None;
                    } else if !self.checked.is_empty() {
                        self.checked.clear();
                    }
                }
                Shortcut::Next(step) => {
                    if self.view == View::Process && self.tab != ProcessTab::Settings {
                        self.select_adjacent(step);
                    }
                }
            }
        }
    }

    fn record_snapshot_refresh(&mut self, elapsed: Duration) {
        if self.toggles.diagnostics {
            self.diagnostics.max_snapshot_ms = self
                .diagnostics
                .max_snapshot_ms
                .max(elapsed.as_secs_f32() * 1000.0);
        }
    }

    fn record_viewport_motion(
        &mut self,
        viewport_pos: Option<Pos2>,
        viewport_moved: bool,
        viewport_resized: bool,
    ) {
        if !self.toggles.diagnostics {
            self.diagnostics.last_viewport_pos = viewport_pos;
            return;
        }

        let now = Instant::now();
        let diagnostics = &mut self.diagnostics;
        diagnostics.last_viewport_pos = viewport_pos;

        let motion_event = viewport_moved || viewport_resized;
        let active = diagnostics
            .motion_active_until
            .is_some_and(|until| now <= until);

        if motion_event && !active {
            diagnostics.motion_burst_start = Some(now);
            diagnostics.motion_updates = 0;
            diagnostics.motion_move_events = 0;
            diagnostics.motion_resize_events = 0;
        }

        if motion_event {
            diagnostics.motion_active_until = Some(now + Duration::from_millis(250));
            if viewport_moved {
                diagnostics.motion_move_events += 1;
            }
            if viewport_resized {
                diagnostics.motion_resize_events += 1;
            }
        }

        let still_active = diagnostics
            .motion_active_until
            .is_some_and(|until| now <= until);
        if still_active {
            diagnostics.motion_updates += 1;
            if let Some(start) = diagnostics.motion_burst_start {
                let elapsed = now.duration_since(start).as_secs_f32().max(0.001);
                diagnostics.last_motion_fps = diagnostics.motion_updates as f32 / elapsed;
                diagnostics.last_motion_updates = diagnostics.motion_updates;
                diagnostics.last_motion_move_events = diagnostics.motion_move_events;
                diagnostics.last_motion_resize_events = diagnostics.motion_resize_events;
            }
        }
    }

    fn next_repaint_delay(&self) -> Option<Duration> {
        let counts = self.runtime_snapshot.counts;
        if counts.starting > 0 || counts.stopping > 0 {
            return Some(Duration::from_millis(50));
        }
        if counts.running > 0 {
            // Uptime and the restart countdowns tick each second; output wakes us sooner.
            return Some(Duration::from_millis(250));
        }

        let now = Instant::now();
        if self.message.as_ref().is_some_and(|(_, until)| now < *until) {
            return Some(Duration::from_millis(250));
        }

        if self.any_process_row_flash_active(now) {
            return Some(Duration::from_millis(50));
        }

        if self
            .config
            .processes
            .iter()
            .any(|process| matches!(process.process_type, ProcessType::Docker))
        {
            return Some(Duration::from_millis(750));
        }

        Some(Duration::from_secs(5))
    }

    fn record_update_timing(&mut self, elapsed: Duration) {
        if !self.toggles.diagnostics {
            return;
        }

        let now = Instant::now();
        let diagnostics = &mut self.diagnostics;
        diagnostics.start_time.get_or_insert(now);
        diagnostics.updates += 1;

        let elapsed_ms = elapsed.as_secs_f32() * 1000.0;
        diagnostics.max_update_ms = diagnostics.max_update_ms.max(elapsed_ms);
        if elapsed_ms > 32.0 {
            diagnostics.slow_updates += 1;
        }

        let should_flush = diagnostics
            .last_summary
            .map(|last| now.duration_since(last) >= Duration::from_secs(1))
            .unwrap_or(true);

        if should_flush {
            diagnostics.last_summary = Some(now);
            let uptime = diagnostics
                .start_time
                .map(|start| now.duration_since(start).as_secs_f32())
                .unwrap_or_default();
            let line = format!(
                "session={} uptime={uptime:.1}s updates={} slow_updates={} max_update_ms={:.2} max_snapshot_ms={:.2} renderer={} backend={} adapter={} present={:?} vsync={} run_and_return={} motion_fps={:.1} motion_updates={} move_events={} resize_events={}\n",
                diagnostics
                    .session_label
                    .as_deref()
                    .unwrap_or("-"),
                diagnostics.updates,
                diagnostics.slow_updates,
                diagnostics.max_update_ms,
                diagnostics.max_snapshot_ms,
                self.toggles.renderer.label(),
                diagnostics
                    .renderer_backend
                    .as_deref()
                    .unwrap_or("-"),
                diagnostics
                    .renderer_adapter
                    .as_deref()
                    .unwrap_or("-"),
                self.toggles.present,
                self.toggles.vsync,
                self.toggles.run_and_return,
                diagnostics.last_motion_fps,
                diagnostics.last_motion_updates,
                diagnostics.last_motion_move_events,
                diagnostics.last_motion_resize_events,
            );
            append_diagnostics_line(&mut diagnostics.log_path, &line);
            diagnostics.max_update_ms = 0.0;
            diagnostics.max_snapshot_ms = 0.0;
        }
    }

    fn draw_diagnostics_overlay(&mut self, ctx: &Context) {
        if !self.toggles.diagnostics {
            return;
        }

        let uptime = self
            .diagnostics
            .start_time
            .map(|start| Instant::now().duration_since(start).as_secs_f32())
            .unwrap_or_default();

        Window::new("Diagnostics")
            .default_pos([14.0, 90.0])
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                if let Some(session) = &self.diagnostics.session_label {
                    ui.label(format!("session: {session}"));
                }
                ui.label(format!("renderer: {}", self.toggles.renderer.label()));
                ui.label(format!(
                    "backend: {}",
                    self.diagnostics
                        .renderer_backend
                        .as_deref()
                        .unwrap_or("n/a")
                ));
                ui.label(format!(
                    "adapter: {}",
                    self.diagnostics
                        .renderer_adapter
                        .as_deref()
                        .unwrap_or("n/a")
                ));
                ui.label(format!("present: {:?}", self.toggles.present));
                ui.label(format!("vsync: {}", self.toggles.vsync));
                ui.label(format!("run_and_return: {}", self.toggles.run_and_return));
                ui.label(format!("uptime: {uptime:.1}s"));
                ui.label(format!("updates: {}", self.diagnostics.updates));
                ui.label(format!(
                    "slow updates (>32ms): {}",
                    self.diagnostics.slow_updates
                ));
                ui.label(format!(
                    "motion fps: {:.1} (updates={} move={} resize={})",
                    self.diagnostics.last_motion_fps,
                    self.diagnostics.last_motion_updates,
                    self.diagnostics.last_motion_move_events,
                    self.diagnostics.last_motion_resize_events
                ));
                ui.label(format!(
                    "selected logs: {}",
                    self.runtime_snapshot.selected_logs.len()
                ));
                if let Some(path) = &self.diagnostics.log_path {
                    ui.label(format!("log: {}", path.display()));
                }
            });
    }

    fn maybe_request_attention(&mut self, ctx: &Context) {
        let current = self.manager.error_version();
        if current <= self.last_error_version {
            return;
        }

        self.sync_process_error_flashes();
        self.last_error_version = current;

        ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Informational,
        ));
    }

    /// `PM_SCREENSHOT`: after the layout has settled, ask for a frame and save it, then quit.
    fn handle_screenshot(&mut self, ctx: &Context) {
        let Some(path) = self.toggles.screenshot.clone() else {
            return;
        };
        // Wait for fonts, the first snapshot and some output to arrive.
        if !self.screenshot_requested && self.launched_at.elapsed() >= self.toggles.screenshot_after
        {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [w, h] = image.size;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            match image::save_buffer(
                &path,
                &rgba,
                w as u32,
                h as u32,
                image::ExtendedColorType::Rgba8,
            ) {
                Ok(()) => eprintln!("Saved screenshot {}", path.display()),
                Err(err) => eprintln!("Failed to save screenshot {}: {err}", path.display()),
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    #[cfg(windows)]
    fn ensure_windows_native_caption(&mut self) {
        if self.native_caption_applied {
            return;
        }

        let hwnd = self
            .root_hwnd
            .or_else(|| find_window_by_title(&self.current_title));
        let Some(hwnd) = hwnd else {
            return;
        };

        apply_windows_caption_theme(hwnd);
        self.native_caption_applied = true;
    }

    #[cfg(not(windows))]
    fn ensure_windows_native_caption(&mut self) {}

    #[cfg(windows)]
    fn ensure_windows_taskbar_icon(&mut self) {
        if self.taskbar_icon_applied {
            return;
        }

        let hwnd = self
            .root_hwnd
            .or_else(|| find_window_by_title(&self.current_title));
        let Some(hwnd) = hwnd else {
            return;
        };

        if self.taskbar_big_icon_handle.is_none() || self.taskbar_small_icon_handle.is_none() {
            let (big_icon, small_icon) = load_executable_taskbar_icon_handles();
            self.taskbar_big_icon_handle = big_icon;
            self.taskbar_small_icon_handle = small_icon;
        }

        let Some(big_icon_handle) = self.taskbar_big_icon_handle else {
            return;
        };
        let small_icon_handle = self.taskbar_small_icon_handle;

        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageW, SetClassLongPtrW, GCLP_HICON, GCLP_HICONSM, ICON_BIG, ICON_SMALL,
            WM_SETICON,
        };

        unsafe {
            SetClassLongPtrW(hwnd, GCLP_HICON, big_icon_handle as isize);
            SendMessageW(
                hwnd,
                WM_SETICON,
                ICON_BIG as usize,
                big_icon_handle as isize,
            );
            if let Some(small_icon_handle) = small_icon_handle {
                SetClassLongPtrW(hwnd, GCLP_HICONSM, small_icon_handle as isize);
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    ICON_SMALL as usize,
                    small_icon_handle as isize,
                );
            }
        }

        self.taskbar_icon_applied = true;
    }

    #[cfg(not(windows))]
    fn ensure_windows_taskbar_icon(&mut self) {}

    fn draw(&mut self, ctx: &Context) {
        let size = ctx.content_rect().size();
        let tier = Tier::for_width(size.x);

        TopBottomPanel::top("header")
            .frame(egui::Frame::new().fill(theme::BG))
            .show_separator_line(true)
            .show(ctx, |ui| self.draw_header(ui, tier));

        TopBottomPanel::bottom("footer")
            .exact_height(theme::FOOTER_HEIGHT)
            .frame(egui::Frame::new().fill(theme::PANEL))
            .show_separator_line(true)
            .show(ctx, |ui| self.draw_footer(ui, tier));

        if tier.has_sidebar() {
            SidePanel::left("sidebar")
                .resizable(true)
                .default_width(theme::SIDEBAR_WIDTH)
                .width_range(theme::SIDEBAR_MIN_WIDTH..=theme::SIDEBAR_MAX_WIDTH)
                .frame(egui::Frame::new().fill(theme::PANEL))
                .show_separator_line(true)
                .show(ctx, |ui| self.draw_sidebar(ui));
        }

        CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG))
            .show(ctx, |ui| match self.view {
                View::Process => match self.selected_process_config() {
                    Some(process) => self.draw_console(ui, &process, tier),
                    None => self.draw_empty_state(ui),
                },
                View::Overview => self.draw_overview(ui, tier),
                View::StackSettings => self.draw_stack_settings(ui, tier),
                View::NewProcess => self.draw_new_process(ui, tier),
            });

        self.draw_palette(ctx, tier);
        self.draw_dialogs(ctx);
        self.draw_diagnostics_overlay(ctx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StackAction {
    Start,
    Stop,
    Restart,
}

enum Shortcut {
    Palette,
    Stack(StackAction),
    RestartSelected,
    ToggleSelected,
    NewProcess,
    Tab(ProcessTab),
    Overview,
    FocusFilter,
    CopyLines,
    Escape,
    Next(isize),
}

impl eframe::App for ProcessManagerApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let update_started = Instant::now();
        self.ensure_valid_selection();
        self.update_title(ctx);
        self.ensure_windows_native_caption();
        self.ensure_windows_taskbar_icon();
        let (viewport_pos, viewport_size) = ctx.input(|input| {
            let viewport = input.viewport();
            let viewport_pos = viewport.outer_rect.map(|rect| rect.min);
            let viewport_size = viewport
                .inner_rect
                .map(|rect| rect.size())
                .unwrap_or_default();
            (viewport_pos, viewport_size)
        });
        let viewport_pos_changed = self
            .diagnostics
            .last_viewport_pos
            .zip(viewport_pos)
            .map(|(previous, current)| {
                (previous.x - current.x).abs() > 0.5 || (previous.y - current.y).abs() > 0.5
            })
            .unwrap_or(false);
        let viewport_size_changed = self
            .last_viewport_size
            .map(|previous| previous != viewport_size)
            .unwrap_or(true);
        self.record_viewport_motion(viewport_pos, viewport_pos_changed, viewport_size_changed);
        self.last_viewport_size = Some(viewport_size);

        self.handle_shortcuts(ctx);
        self.maybe_request_attention(ctx);
        self.sync_rest_config_reload();
        self.refresh_runtime_snapshot(false);

        if viewport_pos_changed || viewport_size_changed {
            ctx.request_repaint();
        }
        if let Some(delay) = self.next_repaint_delay() {
            ctx.request_repaint_after(delay);
        }

        self.draw(ctx);
        self.handle_screenshot(ctx);
        self.record_update_timing(update_started.elapsed());
    }
}

impl Drop for ProcessManagerApp {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(icon_handle) = self.taskbar_big_icon_handle.take() {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(icon_handle);
            }
        }

        #[cfg(windows)]
        if let Some(icon_handle) = self.taskbar_small_icon_handle.take() {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(icon_handle);
            }
        }

        self.rest_controller.shutdown();
        self.manager.stop_non_docker();
    }
}

#[cfg(test)]
mod appearance_tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_renderer_has_a_native_backend() {
        assert!(eframe::wgpu::Instance::enabled_backend_features()
            .contains(eframe::wgpu::Backends::METAL));
    }

    use super::*;
    use eframe::egui::FontId;

    #[test]
    fn bundled_fonts_cover_the_interface_and_keep_logs_monospace() {
        let ctx = Context::default();
        theme::apply(&ctx);

        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut input = egui::RawInput::default();
            input
                .viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(scale);
            let _ = ctx.run(input, |ctx| {
                assert_eq!(ctx.pixels_per_point(), scale);
                ctx.fonts_mut(|fonts| {
                    for font in [theme::body(13.0), theme::bold(13.0)] {
                        let missing: String = "Process Manager — · … ↑ ↓ ’ “ ” 0–9"
                            .chars()
                            .filter(|&c| !fonts.has_glyph(&font, c))
                            .collect();
                        assert!(missing.is_empty(), "Missing glyphs: {missing}");
                    }
                    let font = FontId::monospace(12.0);
                    let width = fonts.glyph_width(&font, 'i');
                    assert!(width > 0.0);
                    for character in ['W', '0', ' '] {
                        assert_eq!(fonts.glyph_width(&font, character), width);
                    }
                });
            });
        }
    }

    #[test]
    fn dark_appearance_survives_system_theme_changes() {
        for initial_theme in [None, Some(egui::Theme::Light), Some(egui::Theme::Dark)] {
            let ctx = Context::default();
            let _ = ctx.run(
                egui::RawInput {
                    system_theme: initial_theme,
                    ..Default::default()
                },
                |_| {},
            );
            theme::apply(&ctx);
            let expected = ctx.style();

            for system_theme in [egui::Theme::Light, egui::Theme::Dark, egui::Theme::Light] {
                let _ = ctx.run(
                    egui::RawInput {
                        system_theme: Some(system_theme),
                        ..Default::default()
                    },
                    |ctx| {
                        let actual = ctx.style();
                        assert_eq!(ctx.theme(), egui::Theme::Dark);
                        assert_eq!(actual.visuals, expected.visuals);
                        assert_eq!(actual.spacing, expected.spacing);
                    },
                );
            }
        }
    }

    #[test]
    fn layout_tiers_step_down_with_the_window() {
        assert_eq!(Tier::for_width(1440.0), Tier::Wide);
        assert_eq!(Tier::for_width(1000.0), Tier::Full);
        assert_eq!(Tier::for_width(800.0), Tier::Medium);
        assert_eq!(Tier::for_width(600.0), Tier::Compact);
        assert_eq!(Tier::for_width(420.0), Tier::Tiny);
        assert!(Tier::Medium.has_sidebar() && !Tier::Compact.has_sidebar());
    }
}

#[cfg(windows)]
fn apply_windows_caption_theme(hwnd: windows_sys::Win32::Foundation::HWND) {
    use std::ffi::c_void;

    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWA_CAPTION_COLOR: u32 = 35;
    const DWMWA_TEXT_COLOR: u32 = 36;
    const DWMWCP_ROUND: u32 = 2;

    let dark_mode_enabled: i32 = 1;
    let corner_preference = DWMWCP_ROUND;
    let border_color = color_to_colorref(theme::LINE);
    let caption_color = color_to_colorref(theme::BG);
    let text_color = color_to_colorref(theme::MUTED);

    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark_mode_enabled as *const _ as *const c_void,
            std::mem::size_of_val(&dark_mode_enabled) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner_preference as *const _ as *const c_void,
            std::mem::size_of_val(&corner_preference) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_color as *const _ as *const c_void,
            std::mem::size_of_val(&border_color) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption_color as *const _ as *const c_void,
            std::mem::size_of_val(&caption_color) as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text_color as *const _ as *const c_void,
            std::mem::size_of_val(&text_color) as u32,
        );
    }
}

#[cfg(windows)]
fn color_to_colorref(color: egui::Color32) -> u32 {
    (color.r() as u32) | ((color.g() as u32) << 8) | ((color.b() as u32) << 16)
}

fn window_title(stack_name: &str) -> String {
    format!("Process Manager - {}", stack_name)
}

fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|err| format!("Clipboard unavailable: {}", err))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|err| format!("Failed to set clipboard text: {}", err))
}

fn wait_for_process_stop(manager: &ProcessManager, id: &str) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
        match manager.get_status(id) {
            Some(ProcessStatus::Stopped | ProcessStatus::Error(_)) | None => break,
            _ => {}
        }
    }
}

fn append_diagnostics_line(log_path: &mut Option<PathBuf>, line: &str) {
    let path = log_path.get_or_insert_with(|| {
        crate::platform::data_directory().join("process-manager-diagnostics.log")
    });

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(windows)]
fn find_window_by_title(window_title: &str) -> Option<windows_sys::Win32::Foundation::HWND> {
    use std::iter;

    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    let title_wide: Vec<u16> = window_title.encode_utf16().chain(iter::once(0)).collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title_wide.as_ptr()) };
    if hwnd.is_null() {
        None
    } else {
        Some(hwnd)
    }
}

#[cfg(windows)]
fn extract_root_hwnd(
    cc: &eframe::CreationContext<'_>,
) -> Option<windows_sys::Win32::Foundation::HWND> {
    let handle = cc.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => {
            Some(handle.hwnd.get() as windows_sys::Win32::Foundation::HWND)
        }
        _ => None,
    }
}

#[cfg(windows)]
fn load_executable_taskbar_icon_handles() -> (
    Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON>,
    Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON>,
) {
    use std::iter;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::ExtractIconExW;

    let Some(exe_path) = std::env::current_exe().ok() else {
        return (None, None);
    };
    let exe_path_wide: Vec<u16> = exe_path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();

    let mut large_icon = std::ptr::null_mut();
    let mut small_icon = std::ptr::null_mut();
    let extracted = unsafe {
        ExtractIconExW(
            exe_path_wide.as_ptr(),
            0,
            &mut large_icon,
            &mut small_icon,
            1,
        )
    };

    if extracted == 0 {
        (None, None)
    } else {
        (
            (!large_icon.is_null()).then_some(large_icon),
            (!small_icon.is_null()).then_some(small_icon),
        )
    }
}
