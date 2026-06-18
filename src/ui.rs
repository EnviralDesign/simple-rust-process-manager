//! Native desktop shell built with egui/eframe.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Button, CentralPanel, Color32, Context, CornerRadius, FontId, Key, Layout,
    Pos2, RichText, ScrollArea, SidePanel, Stroke, TextEdit, TopBottomPanel, Ui, UiBuilder, Vec2,
    ViewportBuilder, ViewportCommand, Window,
};
#[cfg(windows)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tokio::runtime::Runtime;

use crate::config::{
    weekly_hour_enabled, weekly_hour_index, AppConfig, ManagedRestartSchedule, ProcessConfig,
    ProcessType, ScheduledRun, ScheduledRunMode, DEFAULT_LOG_ROTATION_COUNT,
    DEFAULT_STARTUP_DELAY_SECONDS, WEEKLY_HOUR_COUNT,
};
use crate::log_classification::contains_error_indicator;
use crate::process_manager::{ProcessCounts, ProcessManager, ProcessStatus, UiRuntimeSnapshot};
use crate::rest_api::{build_agent_bootstrap, RestServerController, RestServerSnapshot};

const SHELL_BG: Color32 = Color32::from_rgb(32, 32, 36); // Fixed shell / native caption chrome
const BODY_BG: Color32 = Color32::from_rgb(24, 24, 24); // Content inset — neutral gray like Codex main pane
const PANEL_BG: Color32 = Color32::from_rgb(26, 26, 29); // Dialogs, raised surfaces
const BORDER: Color32 = Color32::from_rgb(45, 45, 48);
const SHELL_HOVER_BG: Color32 = Color32::from_rgb(40, 40, 45);
const SHELL_ACTIVE_BG: Color32 = Color32::from_rgb(47, 47, 53);
const SHELL_STROKE: Color32 = Color32::from_rgb(56, 56, 62);
const SHELL_SUBTLE_STROKE: Color32 = Color32::from_rgb(50, 50, 56);
const PROCESS_ROW_SELECTED_BG: Color32 = Color32::from_rgb(43, 43, 48);
const PROCESS_ROW_HOVER_BG: Color32 = Color32::from_rgb(37, 37, 42);
const TAB_SELECTED_BG: Color32 = Color32::from_rgb(55, 71, 95);
const TAB_SELECTED_HOVER_BG: Color32 = Color32::from_rgb(61, 79, 105);
const TAB_SELECTED_ACTIVE_BG: Color32 = Color32::from_rgb(68, 88, 116);
const TAB_SELECTED_STROKE: Color32 = Color32::from_rgb(112, 150, 204);
const TEXT_MAIN: Color32 = Color32::from_rgb(237, 237, 237); // #EDEDED
const TEXT_MUTED: Color32 = Color32::from_rgb(136, 136, 136); // #888888
const TEXT_SOFT: Color32 = Color32::from_rgb(180, 180, 180);
const RUNNING: Color32 = Color32::from_rgb(85, 184, 122);
const WARNING: Color32 = Color32::from_rgb(214, 153, 77);
const DANGER: Color32 = Color32::from_rgb(210, 95, 95);
const STOPPED: Color32 = Color32::from_rgb(112, 118, 126);
const TOOLBAR_TEXT: Color32 = Color32::from_rgb(186, 186, 186);
const TOOLBAR_GREEN: Color32 = Color32::from_rgb(106, 188, 131);
const TOOLBAR_YELLOW: Color32 = Color32::from_rgb(210, 164, 96);
const TOOLBAR_RED: Color32 = Color32::from_rgb(208, 116, 116);
const TOOLBAR_GRAY: Color32 = Color32::from_rgb(162, 162, 162);
const ACCENT_SOFT: Color32 = Color32::from_rgb(86, 102, 126);
const SIDEBAR_WIDTH: f32 = 240.0;
const SIDEBAR_MIN_WIDTH: f32 = 180.0;
const SIDEBAR_MAX_WIDTH: f32 = 460.0;
const PROCESS_LABEL_HOVER_DELAY_SECONDS: f64 = 2.0;
const UI_LOG_LIMIT: usize = 1000;
const WINDOW_CORNER_RADIUS: u8 = 8;
const CONTENT_GUTTER_X: i8 = 16;
const LOG_STICK_THRESHOLD_PX: f32 = 22.0;
const FIELD_BG: Color32 = Color32::from_rgb(20, 20, 20);
const FIELD_BG_HOVER: Color32 = Color32::from_rgb(24, 24, 24);
const FIELD_BORDER: Color32 = Color32::from_gray(46);
const FIELD_BORDER_FOCUS: Color32 = Color32::from_gray(72);
const FIELD_BORDER_DISABLED: Color32 = Color32::from_gray(58);
const MODAL_FORM_WIDTH: f32 = 398.0;
const MODAL_BUTTON_HEIGHT: f32 = 34.0;
const PROCESS_DIALOG_HEIGHT: f32 = 520.0;
const GLOBAL_SETTINGS_HEIGHT: f32 = PROCESS_DIALOG_HEIGHT;
const BROWSE_BUTTON_WIDTH: f32 = 82.0;
const MODAL_FOOTER_HEIGHT: f32 = 58.0;
const PROJECT_WEBSITE_URL: &str = "https://github.com/EnviralDesign/simple-rust-process-manager";
const PROJECT_GITHUB_ACCOUNT_URL: &str = "https://github.com/EnviralDesign";
const PROJECT_GITHUB_ACCOUNT_HANDLE: &str = "@EnviralDesign";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptionSyncMode {
    Off,
    Startup,
    Continuous,
}

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
    caption_sync: CaptionSyncMode,
    diagnostics: bool,
}

impl RuntimeToggles {
    fn from_env() -> Self {
        let mut toggles = Self {
            renderer: default_renderer_profile(),
            present: default_present_profile(),
            vsync: default_vsync_enabled(),
            run_and_return: false,
            // Screen readback for title-bar sampling is purely cosmetic and can
            // become expensive on some Windows GPU / remote-desktop setups.
            caption_sync: CaptionSyncMode::Off,
            diagnostics: false,
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

        toggles.caption_sync = match std::env::var("PM_CAPTION_SYNC")
            .unwrap_or_else(|_| default_caption_sync_label(toggles.caption_sync).to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "off" => CaptionSyncMode::Off,
            "continuous" => CaptionSyncMode::Continuous,
            _ => CaptionSyncMode::Startup,
        };

        toggles.diagnostics = std::env::var("PM_DIAGNOSTICS")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

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

fn default_caption_sync_label(mode: CaptionSyncMode) -> &'static str {
    match mode {
        CaptionSyncMode::Off => "off",
        CaptionSyncMode::Startup => "startup",
        CaptionSyncMode::Continuous => "continuous",
    }
}

#[derive(Default)]
struct DiagnosticsState {
    start_time: Option<Instant>,
    updates: u64,
    slow_updates: u64,
    max_update_ms: f32,
    max_snapshot_ms: f32,
    max_caption_probe_ms: f32,
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

pub fn run() -> eframe::Result<()> {
    let toggles = RuntimeToggles::from_env();

    let mut viewport = ViewportBuilder::default()
        .with_title("Process Manager")
        .with_inner_size([1180.0, 760.0])
        .with_min_inner_size([920.0, 560.0]);

    if let Some(icon) = load_icon_data() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        renderer: renderer_backend(toggles.renderer),
        vsync: toggles.vsync,
        run_and_return: toggles.run_and_return,
        wgpu_options: wgpu_configuration(toggles.renderer, toggles.present),
        ..Default::default()
    };

    eframe::run_native(
        "Process Manager",
        native_options,
        Box::new(move |cc| Ok(Box::new(ProcessManagerApp::new(cc, toggles.clone())))),
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

    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
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

#[derive(Clone)]
struct ProcessDraft {
    name: String,
    command: String,
    working_directory: String,
    process_type: ProcessType,
    auto_start: bool,
    startup_delay_seconds: String,
    auto_restart: bool,
    restart_schedule: ManagedRestartSchedule,
    scheduled_run: ScheduledRun,
    scheduled_run_hour: String,
    scheduled_run_interval_hours: String,
    restart_schedule_editor_open: bool,
    scheduled_run_editor_open: bool,
    respond_to_start_all: bool,
    respond_to_stop_all: bool,
    respond_to_restart_all: bool,
    log_to_disk: bool,
    log_rotation_count: String,
}

impl Default for ProcessDraft {
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            working_directory: String::new(),
            process_type: ProcessType::Process,
            auto_start: false,
            startup_delay_seconds: DEFAULT_STARTUP_DELAY_SECONDS.to_string(),
            auto_restart: false,
            restart_schedule: ManagedRestartSchedule::default(),
            scheduled_run: ScheduledRun::default(),
            scheduled_run_hour: ScheduledRun::default().hour.to_string(),
            scheduled_run_interval_hours: ScheduledRun::default().interval_hours.to_string(),
            restart_schedule_editor_open: false,
            scheduled_run_editor_open: false,
            respond_to_start_all: true,
            respond_to_stop_all: true,
            respond_to_restart_all: true,
            log_to_disk: false,
            log_rotation_count: DEFAULT_LOG_ROTATION_COUNT.to_string(),
        }
    }
}

impl ProcessDraft {
    fn from_process(process: &ProcessConfig) -> Self {
        Self {
            name: process.name.clone(),
            command: process.command.clone(),
            working_directory: process.working_directory.clone(),
            process_type: process.process_type.clone(),
            auto_start: process.auto_start,
            startup_delay_seconds: process.startup_delay_seconds.to_string(),
            auto_restart: process.auto_restart,
            restart_schedule: process.restart_schedule.clone(),
            scheduled_run: process.scheduled_run.clone(),
            scheduled_run_hour: process.scheduled_run.hour.to_string(),
            scheduled_run_interval_hours: process.scheduled_run.interval_hours.to_string(),
            restart_schedule_editor_open: false,
            scheduled_run_editor_open: false,
            respond_to_start_all: process.respond_to_start_all,
            respond_to_stop_all: process.respond_to_stop_all,
            respond_to_restart_all: process.respond_to_restart_all,
            log_to_disk: process.log_to_disk,
            log_rotation_count: process.log_rotation_count.to_string(),
        }
    }
}

enum ProcessDialog {
    Add(ProcessDraft),
    Edit { id: String, form: ProcessDraft },
}

impl ProcessDialog {
    fn title(&self) -> &'static str {
        match self {
            Self::Add(_) => "Add Process",
            Self::Edit { .. } => "Edit Process",
        }
    }

    fn form_mut(&mut self) -> &mut ProcessDraft {
        match self {
            Self::Add(form) => form,
            Self::Edit { form, .. } => form,
        }
    }
}

#[derive(Clone)]
struct RestSettingsForm {
    enabled: bool,
    port: String,
    log_directory: String,
    process_error_flash_seconds: String,
}

impl RestSettingsForm {
    fn from_config(config: &AppConfig) -> Self {
        Self {
            enabled: config.remote_control.enabled,
            port: config.remote_control.port.to_string(),
            log_directory: config.log_directory.clone(),
            process_error_flash_seconds: config.process_error_flash_seconds.to_string(),
        }
    }
}

struct AboutField {
    label: &'static str,
    value: String,
    link: Option<&'static str>,
}

struct TimedFlash {
    started_at: Instant,
    until: Instant,
}

#[derive(Clone, Debug)]
struct ProcessLabelHover {
    process_id: String,
    started_at: Instant,
}

#[derive(Clone, Debug)]
struct LogSelection {
    process_id: String,
    anchor: usize,
    focus: usize,
}

impl LogSelection {
    fn range(&self) -> (usize, usize) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }
}

#[derive(Clone, Debug)]
struct FrozenLogLine {
    process_id: String,
    index: usize,
}

pub struct ProcessManagerApp {
    toggles: RuntimeToggles,
    runtime: Runtime,
    manager: Arc<ProcessManager>,
    rest_controller: Arc<RestServerController>,
    config: AppConfig,
    selected_process: Option<String>,
    dragged_process: Option<String>,
    process_dialog: Option<ProcessDialog>,
    delete_process_id: Option<String>,
    reload_processes_confirm_open: bool,
    rest_settings_open: bool,
    global_settings_tab: usize,
    rest_settings_form: RestSettingsForm,
    rest_settings_error: Option<String>,
    stack_name_buffer: String,
    banner: Option<(String, Instant)>,
    copy_feedback_until: Option<Instant>,
    stick_logs_to_bottom: bool,
    log_selection: Option<LogSelection>,
    frozen_log_line: Option<FrozenLogLine>,
    last_error_version: u64,
    last_process_error_versions: HashMap<String, u64>,
    process_row_flashes: HashMap<String, TimedFlash>,
    process_label_hover: Option<ProcessLabelHover>,
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
    shell_bg: Color32,
    caption_color_initialized: bool,
    next_caption_probe: Instant,
    last_focus_state: Option<bool>,
    last_viewport_size: Option<Vec2>,
    last_manager_version: u64,
    snapshot_selected_process: Option<String>,
    runtime_snapshot: UiRuntimeSnapshot,
    diagnostics: DiagnosticsState,
}

impl ProcessManagerApp {
    fn new(cc: &eframe::CreationContext<'_>, toggles: RuntimeToggles) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_visuals(&cc.egui_ctx);

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
        let runtime_snapshot = manager.build_ui_snapshot(selected_process.as_deref(), UI_LOG_LIMIT);
        let last_manager_version = manager.current_version();
        let current_title = window_title(&config.stack_name);
        let rest_settings_form = RestSettingsForm::from_config(&config);
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

        Self {
            toggles,
            runtime,
            manager,
            rest_controller,
            config,
            selected_process: selected_process.clone(),
            dragged_process: None,
            process_dialog: None,
            delete_process_id: None,
            reload_processes_confirm_open: false,
            rest_settings_open: false,
            global_settings_tab: 0,
            rest_settings_form,
            rest_settings_error: None,
            stack_name_buffer: String::new(),
            banner: None,
            copy_feedback_until: None,
            stick_logs_to_bottom: true,
            log_selection: None,
            frozen_log_line: None,
            last_error_version: 0,
            last_process_error_versions,
            process_row_flashes: HashMap::new(),
            process_label_hover: None,
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
            shell_bg: SHELL_BG,
            caption_color_initialized: false,
            next_caption_probe: Instant::now(),
            last_focus_state: None,
            last_viewport_size: None,
            last_manager_version,
            snapshot_selected_process: selected_process.clone(),
            runtime_snapshot,
            diagnostics: DiagnosticsState {
                session_label,
                renderer_backend,
                renderer_adapter,
                ..DiagnosticsState::default()
            },
        }
    }

    fn update_title(&mut self, ctx: &Context) {
        let next = window_title(&self.config.stack_name);
        if next != self.current_title {
            self.current_title = next.clone();
            ctx.send_viewport_cmd(ViewportCommand::Title(next));
        }
    }

    fn set_banner(&mut self, message: impl Into<String>) {
        self.banner = Some((message.into(), Instant::now() + Duration::from_secs(3)));
    }

    fn visible_banner(&mut self) -> Option<&str> {
        let expired = self
            .banner
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until);
        if expired {
            self.banner = None;
        }
        self.banner.as_ref().map(|(message, _)| message.as_str())
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

    fn process_row_flash_intensity(&self, ctx: &Context, process_id: &str) -> f32 {
        let Some(fade) = self.process_row_flash_fade(process_id, Instant::now()) else {
            return 0.0;
        };

        let tail_fade = if fade > 0.18 { 1.0 } else { fade / 0.18 };
        let pulse =
            0.6 + ((((ctx.input(|input| input.time) as f32) * 8.5).sin() * 0.5) + 0.5) * 0.4;
        pulse * tail_fade.clamp(0.0, 1.0)
    }

    fn update_process_label_hover(
        &mut self,
        ui: &mut Ui,
        response: &egui::Response,
        process: &ProcessConfig,
    ) {
        if response.hovered() {
            let now = Instant::now();
            let reset_hover = self
                .process_label_hover
                .as_ref()
                .map_or(true, |hover| hover.process_id != process.id);

            if reset_hover {
                self.process_label_hover = Some(ProcessLabelHover {
                    process_id: process.id.clone(),
                    started_at: now,
                });
            }

            let delay = Duration::from_secs_f64(PROCESS_LABEL_HOVER_DELAY_SECONDS);
            let elapsed = self
                .process_label_hover
                .as_ref()
                .filter(|hover| hover.process_id == process.id)
                .map(|hover| now.saturating_duration_since(hover.started_at))
                .unwrap_or_default();

            if elapsed >= delay {
                let label = process_tab_label(process);
                egui::Tooltip::always_open(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    response.id.with("full_process_label"),
                    egui::PopupAnchor::Pointer,
                )
                .gap(12.0)
                .show(|ui| {
                    ui.set_max_width(360.0);
                    ui.label(RichText::new(label).color(TEXT_MAIN).size(13.0));
                });
            } else {
                ui.ctx().request_repaint_after(delay - elapsed);
            }
        } else if self
            .process_label_hover
            .as_ref()
            .is_some_and(|hover| hover.process_id == process.id)
        {
            self.process_label_hover = None;
        }
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

    fn rest_snapshot(&self) -> RestServerSnapshot {
        self.rest_controller.snapshot()
    }

    fn ensure_valid_selection(&mut self) {
        let selected_exists = self
            .selected_process
            .as_ref()
            .and_then(|id| self.config.get_process(id))
            .is_some();
        if !selected_exists {
            self.selected_process = self
                .config
                .processes
                .first()
                .map(|process| process.id.clone());
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

    fn open_add_process(&mut self) {
        self.process_dialog = Some(ProcessDialog::Add(ProcessDraft::default()));
    }

    fn open_edit_process(&mut self, process_id: &str) {
        if let Some(process) = self.process_config(process_id) {
            self.process_dialog = Some(ProcessDialog::Edit {
                id: process.id.clone(),
                form: ProcessDraft::from_process(&process),
            });
        }
    }

    fn open_rest_settings(&mut self) {
        self.stack_name_buffer = self.config.stack_name.clone();
        self.rest_settings_form = RestSettingsForm::from_config(&self.config);
        self.rest_settings_error = None;
        self.rest_settings_open = true;
    }

    fn request_processes_reload(&mut self) {
        self.reload_processes_confirm_open = true;
    }

    fn process_config(&self, process_id: &str) -> Option<ProcessConfig> {
        self.manager
            .get_process_config(process_id)
            .or_else(|| self.config.get_process(process_id).cloned())
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

        let Some(updated) = config.processes.iter().find(|process| process.id == process_id) else {
            self.set_banner(format!(
                "Process id '{}' not found in processes.json.",
                process_id
            ));
            return;
        };

        if !self
            .manager
            .reload_process_from_config(updated.clone())
        {
            self.set_banner(format!(
                "Process '{}' is not currently managed in this session.",
                process_id
            ));
            return;
        }

        self.config.update_process(process_id, updated.clone());
        self.last_process_error_versions = self.manager.error_versions();
        self.process_row_flashes.remove(process_id);
        self.refresh_runtime_snapshot(true);
        self.set_banner(format!("Reloaded '{}' from processes.json.", updated.name));
    }

    fn cancel_reload_processes_confirmation(&mut self) {
        self.reload_processes_confirm_open = false;
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

        self.set_banner("Stopping all processes and reloading from processes.json...");
        self.manager.set_log_directory(config.log_directory.clone());
        self.manager.reload_from_config(&config.processes);
        self.config = config;
        self.apply_rest_config();
        self.last_process_error_versions = self.manager.error_versions();
        self.process_row_flashes.clear();
        self.ensure_valid_selection();
        self.refresh_runtime_snapshot(true);
        self.set_banner("Processes reloaded from processes.json.");
    }

    fn toggle_api_enabled(&mut self) {
        self.config.remote_control.enabled = !self.config.remote_control.enabled;
        self.persist_config();
        self.apply_rest_config();
    }

    fn copy_agent_skill(&mut self) {
        let payload = build_agent_bootstrap(
            &self.config.stack_name,
            &self.config.remote_control,
            &self.rest_snapshot(),
            &self.manager.list_processes(),
        );

        match copy_text_to_clipboard(&payload) {
            Ok(()) => {
                self.copy_feedback_until = Some(Instant::now() + Duration::from_secs(2));
            }
            Err(err) => self.set_banner(err),
        }
    }

    fn apply_process_dialog(&mut self, dialog: ProcessDialog) {
        match dialog {
            ProcessDialog::Add(form) => {
                if form.name.trim().is_empty() || form.command.trim().is_empty() {
                    self.set_banner("Name and command are required.");
                    return;
                }

                let log_rotation_count = match parse_log_rotation_count(&form.log_rotation_count) {
                    Ok(value) => value,
                    Err(err) => {
                        self.set_banner(err);
                        return;
                    }
                };
                let scheduled_run = match build_scheduled_run(&form) {
                    Ok(value) => value,
                    Err(err) => {
                        self.set_banner(err);
                        return;
                    }
                };
                let startup_delay_seconds =
                    match parse_startup_delay_seconds(&form.startup_delay_seconds) {
                        Ok(value) => value,
                        Err(err) => {
                            self.set_banner(err);
                            return;
                        }
                    };
                let restart_schedule = normalize_restart_schedule(form.restart_schedule.clone());

                let mut process = ProcessConfig::new(
                    form.name.trim().to_string(),
                    form.command.trim().to_string(),
                    form.working_directory.trim().to_string(),
                    form.process_type,
                );
                process.auto_start = form.auto_start;
                process.startup_delay_seconds = startup_delay_seconds;
                process.auto_restart = form.auto_restart;
                process.restart_schedule = restart_schedule;
                process.scheduled_run = scheduled_run;
                process.respond_to_start_all = form.respond_to_start_all;
                process.respond_to_stop_all = form.respond_to_stop_all;
                process.respond_to_restart_all = form.respond_to_restart_all;
                process.log_to_disk = form.log_to_disk;
                process.log_rotation_count = log_rotation_count;

                self.manager.add_process(process.clone());
                self.config.add_process(process.clone());
                self.persist_config();
                self.selected_process = Some(process.id);
                self.set_banner("Process added.");
            }
            ProcessDialog::Edit { id, form } => {
                if form.name.trim().is_empty() || form.command.trim().is_empty() {
                    self.set_banner("Name and command are required.");
                    return;
                }

                let log_rotation_count = match parse_log_rotation_count(&form.log_rotation_count) {
                    Ok(value) => value,
                    Err(err) => {
                        self.set_banner(err);
                        return;
                    }
                };
                let scheduled_run = match build_scheduled_run(&form) {
                    Ok(value) => value,
                    Err(err) => {
                        self.set_banner(err);
                        return;
                    }
                };
                let startup_delay_seconds =
                    match parse_startup_delay_seconds(&form.startup_delay_seconds) {
                        Ok(value) => value,
                        Err(err) => {
                            self.set_banner(err);
                            return;
                        }
                    };
                let restart_schedule = normalize_restart_schedule(form.restart_schedule.clone());

                if matches!(
                    self.manager.get_status(&id),
                    Some(
                        ProcessStatus::Running | ProcessStatus::Starting | ProcessStatus::Stopping
                    )
                ) {
                    self.manager.stop_process(&id);
                    wait_for_process_stop(&self.manager, &id);
                }

                let updated = ProcessConfig {
                    id: id.clone(),
                    name: form.name.trim().to_string(),
                    command: form.command.trim().to_string(),
                    working_directory: form.working_directory.trim().to_string(),
                    process_type: form.process_type,
                    auto_start: form.auto_start,
                    startup_delay_seconds,
                    auto_restart: form.auto_restart,
                    restart_schedule,
                    scheduled_run,
                    respond_to_start_all: form.respond_to_start_all,
                    respond_to_stop_all: form.respond_to_stop_all,
                    respond_to_restart_all: form.respond_to_restart_all,
                    log_to_disk: form.log_to_disk,
                    log_rotation_count,
                };

                self.config.update_process(&id, updated.clone());
                self.persist_config();
                let _ = self.manager.update_process_config(updated);
                self.selected_process = Some(id);
                self.set_banner("Process updated.");
            }
        }
    }

    fn save_rest_settings(&mut self) {
        let parsed_port = match self.rest_settings_form.port.trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                self.rest_settings_error =
                    Some("Port must be a number between 1 and 65535.".into());
                return;
            }
        };
        let process_error_flash_seconds = match parse_process_error_flash_seconds(
            &self.rest_settings_form.process_error_flash_seconds,
        ) {
            Ok(seconds) => seconds,
            Err(err) => {
                self.rest_settings_error = Some(err);
                return;
            }
        };

        let trimmed = self.stack_name_buffer.trim();
        if !trimmed.is_empty() && trimmed != self.config.stack_name {
            self.config.stack_name = trimmed.to_string();
        }

        self.config.remote_control.enabled = self.rest_settings_form.enabled;
        self.config.remote_control.port = parsed_port;
        self.config.log_directory = normalize_log_directory(&self.rest_settings_form.log_directory);
        self.config.process_error_flash_seconds = process_error_flash_seconds;
        self.persist_config();
        self.manager
            .set_log_directory(self.config.log_directory.clone());
        self.apply_rest_config();
        self.rest_settings_open = false;
        self.rest_settings_error = None;
        self.set_banner("Global settings saved.");
    }

    fn delete_process(&mut self, process_id: &str) {
        self.manager.remove_process(process_id);
        self.config.remove_process(process_id);
        self.last_process_error_versions.remove(process_id);
        self.process_row_flashes.remove(process_id);
        self.persist_config();
        if self.selected_process.as_deref() == Some(process_id) {
            self.selected_process = None;
        }
        self.ensure_valid_selection();
        self.set_banner("Process deleted.");
    }

    fn move_process_up(&mut self, process_id: &str) {
        if self.config.move_process_up(process_id) {
            self.persist_config();
            self.set_banner("Process moved up.");
        }
    }

    fn move_process_down(&mut self, process_id: &str) {
        if self.config.move_process_down(process_id) {
            self.persist_config();
            self.set_banner("Process moved down.");
        }
    }

    fn move_process_to_index(&mut self, process_id: &str, target_index: usize) {
        if self.config.move_process_to_index(process_id, target_index) {
            self.persist_config();
            self.set_banner("Process reordered.");
        }
    }

    fn draw_drag_insert_marker(
        &self,
        ui: &mut Ui,
        row_bounds: &[egui::Rect],
        insert_index: usize,
    ) {
        if row_bounds.is_empty() {
            return;
        }

        let marker_row = if insert_index >= row_bounds.len() {
            row_bounds.len() - 1
        } else {
            insert_index
        };

        let marker_rect = row_bounds[marker_row];
        let y = if insert_index >= row_bounds.len() {
            marker_rect.max.y
        } else {
            marker_rect.min.y
        };

        let left = marker_rect.left() + 8.0;
        let right = marker_rect.right() - 8.0;

        ui.painter().line_segment(
            [egui::pos2(left, y), egui::pos2(right, y)],
            Stroke::new(2.0, TAB_SELECTED_STROKE),
        );
    }

    fn selected_process_config(&self) -> Option<ProcessConfig> {
        self.selected_process
            .as_ref()
            .and_then(|id| self.process_config(id))
    }

    fn refresh_runtime_snapshot(&mut self, force: bool) {
        let current_version = self.manager.current_version();
        let selected_changed = self.snapshot_selected_process != self.selected_process;
        if !force && !selected_changed && current_version == self.last_manager_version {
            return;
        }

        if selected_changed {
            self.stick_logs_to_bottom = true;
            self.log_selection = None;
            self.frozen_log_line = None;
        }

        let started = Instant::now();
        self.runtime_snapshot = self
            .manager
            .build_ui_snapshot(self.selected_process.as_deref(), UI_LOG_LIMIT);
        self.last_manager_version = current_version;
        self.snapshot_selected_process = self.selected_process.clone();
        self.record_snapshot_refresh(started.elapsed());
    }

    fn select_log_line(&mut self, process_id: &str, log_index: usize, extend_range: bool) {
        self.frozen_log_line = None;

        if extend_range {
            if let Some(selection) = self
                .log_selection
                .as_mut()
                .filter(|selection| selection.process_id == process_id)
            {
                selection.focus = log_index;
                return;
            }
        }

        self.log_selection = Some(LogSelection {
            process_id: process_id.to_string(),
            anchor: log_index,
            focus: log_index,
        });
    }

    fn freeze_log_line(&mut self, process_id: &str, log_index: usize) {
        self.log_selection = Some(LogSelection {
            process_id: process_id.to_string(),
            anchor: log_index,
            focus: log_index,
        });
        self.frozen_log_line = Some(FrozenLogLine {
            process_id: process_id.to_string(),
            index: log_index,
        });
    }

    fn is_frozen_log_line(&self, process_id: &str, log_index: usize) -> bool {
        self.frozen_log_line
            .as_ref()
            .is_some_and(|frozen| frozen.process_id == process_id && frozen.index == log_index)
    }

    fn is_log_text_selection_frozen(&self, process_id: &str) -> bool {
        self.frozen_log_line
            .as_ref()
            .is_some_and(|frozen| frozen.process_id == process_id)
    }

    fn selected_log_range(&self, process_id: &str) -> Option<(usize, usize)> {
        self.log_selection
            .as_ref()
            .filter(|selection| selection.process_id == process_id)
            .map(LogSelection::range)
    }

    fn visible_selected_log_count(
        &self,
        process_id: &str,
        visible_start: usize,
        visible_len: usize,
    ) -> usize {
        let Some((start, end)) = self.selected_log_range(process_id) else {
            return 0;
        };
        let Some(visible_end) = visible_start.checked_add(visible_len.saturating_sub(1)) else {
            return 0;
        };
        if visible_len == 0 || end < visible_start || start > visible_end {
            return 0;
        }
        end.min(visible_end) - start.max(visible_start) + 1
    }

    fn clear_log_selection(&mut self) {
        self.log_selection = None;
        self.frozen_log_line = None;
    }

    fn copy_selected_logs(&mut self) {
        let Some(process_id) = self.selected_process.as_deref() else {
            return;
        };
        let Some((selection_start, selection_end)) = self.selected_log_range(process_id) else {
            return;
        };

        let logs = &self.runtime_snapshot.selected_logs;
        if logs.is_empty() {
            return;
        }

        let visible_start = self
            .runtime_snapshot
            .selected_log_count
            .saturating_sub(logs.len());
        let visible_end = visible_start + logs.len() - 1;
        if selection_end < visible_start || selection_start > visible_end {
            self.set_banner("Selected logs are no longer in memory.");
            self.clear_log_selection();
            return;
        }

        let copy_start = selection_start.max(visible_start);
        let copy_end = selection_end.min(visible_end);
        let start_offset = copy_start - visible_start;
        let end_offset = copy_end - visible_start;
        let payload = logs[start_offset..=end_offset].join("\n");

        match copy_text_to_clipboard(&payload) {
            Ok(()) => self.set_banner(format!(
                "Copied {} log lines.",
                end_offset - start_offset + 1
            )),
            Err(err) => self.set_banner(err),
        }
    }

    fn handle_shortcuts(&mut self, ctx: &Context) {
        if ctx.wants_keyboard_input() {
            return;
        }

        let mut open_add = false;
        let mut start_all = false;
        let mut stop_all = false;
        let mut restart_all = false;
        let mut copy_logs = false;
        let mut clear_logs = false;

        ctx.input(|input| {
            if input.modifiers.ctrl && input.key_pressed(Key::N) {
                open_add = true;
            }
            if input.modifiers.ctrl && input.key_pressed(Key::S) {
                start_all = true;
            }
            if input.modifiers.ctrl && input.key_pressed(Key::X) {
                stop_all = true;
            }
            if input.modifiers.ctrl && input.key_pressed(Key::R) {
                restart_all = true;
            }
            if input.modifiers.ctrl && input.key_pressed(Key::C) && self.frozen_log_line.is_none() {
                copy_logs = true;
            }
            if input.key_pressed(Key::Escape) {
                clear_logs = true;
            }
        });

        if open_add {
            self.open_add_process();
        }
        if copy_logs {
            self.copy_selected_logs();
        }
        if clear_logs {
            self.clear_log_selection();
        }
        if start_all {
            self.manager.start_all();
        }
        if stop_all {
            self.manager.stop_all();
        }
        if restart_all {
            self.manager.restart_all();
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

    fn record_caption_probe(&mut self, elapsed: Duration) {
        if self.toggles.diagnostics {
            self.diagnostics.max_caption_probe_ms = self
                .diagnostics
                .max_caption_probe_ms
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
        let has_active_processes = counts.running > 0 || counts.starting > 0 || counts.stopping > 0;
        if has_active_processes {
            return Some(Duration::from_millis(100));
        }

        let now = Instant::now();
        if self.banner.as_ref().is_some_and(|(_, until)| now < *until) {
            return Some(Duration::from_millis(100));
        }

        if self.copy_feedback_until.is_some_and(|until| now < until) {
            return Some(Duration::from_millis(100));
        }

        if self.any_process_row_flash_active(now) {
            return Some(Duration::from_millis(100));
        }

        if matches!(self.toggles.caption_sync, CaptionSyncMode::Continuous) {
            return Some(Duration::from_secs(2));
        }

        if matches!(self.toggles.caption_sync, CaptionSyncMode::Startup)
            && !self.caption_color_initialized
        {
            return Some(Duration::from_millis(250));
        }

        if self
            .config
            .processes
            .iter()
            .any(|process| matches!(process.process_type, ProcessType::Docker))
        {
            return Some(Duration::from_millis(750));
        }

        None
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
                "session={} uptime={uptime:.1}s updates={} slow_updates={} max_update_ms={:.2} max_snapshot_ms={:.2} max_caption_probe_ms={:.2} renderer={} backend={} adapter={} present={:?} vsync={} run_and_return={} caption_sync={:?} motion_fps={:.1} motion_updates={} move_events={} resize_events={}\n",
                diagnostics
                    .session_label
                    .as_deref()
                    .unwrap_or("-"),
                diagnostics.updates,
                diagnostics.slow_updates,
                diagnostics.max_update_ms,
                diagnostics.max_snapshot_ms,
                diagnostics.max_caption_probe_ms,
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
                self.toggles.caption_sync,
                diagnostics.last_motion_fps,
                diagnostics.last_motion_updates,
                diagnostics.last_motion_move_events,
                diagnostics.last_motion_resize_events,
            );
            append_diagnostics_line(&mut diagnostics.log_path, &line);
            diagnostics.max_update_ms = 0.0;
            diagnostics.max_snapshot_ms = 0.0;
            diagnostics.max_caption_probe_ms = 0.0;
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
                ui.label(format!("caption_sync: {:?}", self.toggles.caption_sync));
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
                    self.runtime_snapshot.selected_log_count
                ));
                if let Some(path) = &self.diagnostics.log_path {
                    ui.label(format!("log: {}", path.display()));
                }
            });
    }

    fn refresh_shell_bg_from_windows_caption(&mut self, focused: bool) -> bool {
        if self.toggles.caption_sync == CaptionSyncMode::Off {
            return false;
        }

        if matches!(self.toggles.caption_sync, CaptionSyncMode::Startup)
            && self.caption_color_initialized
        {
            self.last_focus_state = Some(focused);
            return false;
        }

        let focus_changed = self
            .last_focus_state
            .map(|previous| previous != focused)
            .unwrap_or(true);
        self.last_focus_state = Some(focused);

        if focus_changed && matches!(self.toggles.caption_sync, CaptionSyncMode::Continuous) {
            self.next_caption_probe = Instant::now();
        }

        if Instant::now() < self.next_caption_probe {
            return false;
        }

        let retry_delay = if self.caption_color_initialized {
            match self.toggles.caption_sync {
                CaptionSyncMode::Continuous => Duration::from_secs(2),
                CaptionSyncMode::Startup => Duration::from_secs(60 * 60 * 24),
                CaptionSyncMode::Off => Duration::from_secs(60 * 60 * 24),
            }
        } else {
            Duration::from_millis(16)
        };

        self.next_caption_probe = Instant::now() + retry_delay;

        let started = Instant::now();
        #[cfg(windows)]
        {
            if let Some(color) = sample_windows_title_bar_color(&self.current_title) {
                if !should_accept_caption_color(color) {
                    self.record_caption_probe(started.elapsed());
                    return false;
                }
                let changed = color != self.shell_bg;
                self.shell_bg = color;
                self.caption_color_initialized = true;
                self.record_caption_probe(started.elapsed());
                return changed;
            }
        }

        self.record_caption_probe(started.elapsed());
        false
    }

    fn draw_header(&mut self, ctx: &Context) {
        let counts = self.runtime_snapshot.counts;

        TopBottomPanel::top("header")
            .frame(
                egui::Frame::default()
                    .fill(self.shell_bg)
                    .inner_margin(egui::Margin::symmetric(CONTENT_GUTTER_X, 10))
                    .stroke(Stroke::NONE),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(28.0);
                    ui.spacing_mut().item_spacing.x = 6.0;

                    ui.label(
                        RichText::new(stack_summary(&counts))
                            .color(TEXT_MUTED)
                            .size(11.0),
                    );
                    if let Some(message) = self.visible_banner() {
                        ui.add_space(6.0);
                        ui.label(RichText::new(message).color(TEXT_SOFT).size(11.0));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;

                        // Group 1: Global process controls
                        if chrome_text_button(
                            ui,
                            "⟳ Restart All",
                            TOOLBAR_TEXT,
                            Vec2::new(0.0, 28.0),
                            12.0,
                            false,
                        )
                        .clicked()
                        {
                            self.manager.restart_all();
                        }
                        if chrome_text_button(
                            ui,
                            "■ Stop All",
                            TOOLBAR_TEXT,
                            Vec2::new(0.0, 28.0),
                            12.0,
                            false,
                        )
                        .clicked()
                        {
                            self.manager.stop_all();
                        }
                        if chrome_text_button(
                            ui,
                            "▶ Start All",
                            TOOLBAR_GREEN,
                            Vec2::new(0.0, 28.0),
                            12.0,
                            false,
                        )
                        .clicked()
                        {
                            self.manager.start_all();
                        }

                        if chrome_text_button(
                            ui,
                            "⟳ Reload",
                            TOOLBAR_YELLOW,
                            Vec2::new(0.0, 28.0),
                            12.0,
                            false,
                        )
                        .on_hover_text(
                            "Reload processes.json from disk and reinitialize managed entries.",
                        )
                        .clicked()
                        {
                            self.request_processes_reload();
                        }

                        ui.add_space(4.0);
                        let (sep_rect, _) =
                            ui.allocate_exact_size(Vec2::new(1.0, 18.0), egui::Sense::hover());
                        ui.painter().vline(
                            sep_rect.center().x,
                            sep_rect.y_range(),
                            Stroke::new(1.0, Color32::from_white_alpha(15)),
                        );
                        ui.add_space(4.0);

                        // Group 2: Utilities
                        if chrome_text_button(
                            ui,
                            "📋 Copy Agent Skill",
                            TOOLBAR_TEXT,
                            Vec2::new(0.0, 28.0),
                            12.0,
                            false,
                        )
                        .clicked()
                        {
                            self.copy_agent_skill();
                        }

                        let api_text = format!(
                            "API: {}",
                            if self.config.remote_control.enabled {
                                "ON"
                            } else {
                                "OFF"
                            }
                        );
                        let api_color = if self.config.remote_control.enabled {
                            TOOLBAR_GREEN
                        } else {
                            TOOLBAR_GRAY
                        };
                        if chrome_text_button(
                            ui,
                            &api_text,
                            api_color,
                            Vec2::new(0.0, 28.0),
                            12.0,
                            false,
                        )
                        .on_hover_text("Toggle Local API")
                        .clicked()
                        {
                            self.toggle_api_enabled();
                        }
                    });
                });
            });
    }

    fn draw_sidebar(&mut self, ctx: &Context) {
        SidePanel::left("sidebar")
            .resizable(true)
            .default_width(SIDEBAR_WIDTH)
            .min_width(SIDEBAR_MIN_WIDTH)
            .max_width(SIDEBAR_MAX_WIDTH)
            .frame(
                egui::Frame::default()
                    .fill(self.shell_bg)
                    .inner_margin(egui::Margin::same(12))
                    .stroke(Stroke::NONE),
            )
            .show(ctx, |ui| {
                ui.set_width(ui.available_width());
                TopBottomPanel::bottom("global_settings_panel")
                    .frame(egui::Frame::default().inner_margin(egui::Margin::symmetric(0, 4)))
                    .show_inside(ui, |ui| {
                        if draw_sidebar_footer_button(ui, "Global Settings").clicked() {
                            self.open_rest_settings();
                        }
                    });

                CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.set_height(24.0);
                            ui.label(
                                RichText::new("PROCESSES")
                                    .color(TEXT_MUTED)
                                    .size(11.0)
                                    .strong(),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if chrome_text_button(
                                    ui,
                                    "+ Add",
                                    TEXT_MUTED,
                                    Vec2::new(0.0, 22.0),
                                    11.0,
                                    true,
                                )
                                .clicked()
                                {
                                    self.open_add_process();
                                }
                            });
                        });

                        ui.add_space(10.0);

                        if self.config.processes.is_empty() {
                            ui.add_space(20.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("No processes yet")
                                        .color(TEXT_SOFT)
                                        .size(15.0),
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("Add one with the + button.")
                                        .color(TEXT_MUTED)
                                        .size(12.0),
                                );
                            });
                            return;
                        }

                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                    .show(ui, |ui| {
                                let process_count = self.config.processes.len();
                                let mut move_up_id: Option<String> = None;
                                let mut move_down_id: Option<String> = None;
                                let mut reload_process_id: Option<String> = None;
                                let mut reorder_to: Option<(String, usize)> = None;
                                let mut drag_insert_index: Option<usize> = None;
                                let mut row_bounds: Vec<egui::Rect> = Vec::with_capacity(process_count);

                                for (index, process) in
                                    self.config.processes.clone().into_iter().enumerate()
                                {
                                    let row_process =
                                        self.process_config(&process.id).unwrap_or(process.clone());
                                    let status = self
                                        .runtime_snapshot
                                        .statuses
                                        .get(&row_process.id)
                                        .cloned()
                                        .unwrap_or(ProcessStatus::Stopped);
                                    let is_selected = self.selected_process.as_deref() == Some(process.id.as_str());
                                    let flash_intensity =
                                        self.process_row_flash_intensity(ctx, &row_process.id);
                                    let row_response = draw_process_row(
                                        ui,
                                        &row_process,
                                        &status,
                                        is_selected,
                                        flash_intensity,
                                    );
                                    self.update_process_label_hover(ui, &row_response, &row_process);
                                    let row_clicked = row_response.clicked();
                                    if row_response.drag_started() {
                                        self.dragged_process = Some(process.id.clone());
                                        self.selected_process = Some(process.id.clone());
                                        self.refresh_runtime_snapshot(true);
                                    }
                                    if let Some(dragged_id) = self.dragged_process.clone() {
                                        if dragged_id != process.id
                                            && row_response.hovered()
                                            && ctx.input(|input| input.pointer.any_released())
                                        {
                                            reorder_to = Some((dragged_id.clone(), index));
                                        }
                                        if dragged_id != process.id && row_response.hovered() {
                                            drag_insert_index = Some(index);
                                        }
                                    }
                                    row_bounds.push(row_response.rect);
                                    row_response.context_menu(|ui| {
                                        let can_move_up = index > 0;
                                        let can_move_down = index + 1 < process_count;

                                        if ui
                                            .add_enabled(can_move_up, Button::new("Move up"))
                                            .clicked()
                                        {
                                            move_up_id = Some(process.id.clone());
                                            ui.close();
                                        }
                                        if ui
                                            .add_enabled(can_move_down, Button::new("Move down"))
                                            .clicked()
                                        {
                                            move_down_id = Some(process.id.clone());
                                            ui.close();
                                        }
                                        if ui.button("Reload").clicked() {
                                            reload_process_id = Some(process.id.clone());
                                            ui.close();
                                        }
                                    });
                                    if row_clicked {
                                        self.selected_process = Some(process.id.clone());
                                        self.refresh_runtime_snapshot(true);
                                    }
                                    ui.add_space(2.0);
                                }

                                if self.dragged_process.is_some() && ctx.input(|input| input.pointer.primary_down()) {
                                    let can_place_at_end = !row_bounds.is_empty()
                                        && process_count > 0
                                        && row_bounds.last().is_some_and(|last_rect| {
                                            ctx.input(|input| {
                                                input
                                                    .pointer
                                                    .interact_pos()
                                                    .is_some_and(|pos| pos.y >= last_rect.max.y)
                                            })
                                        });
                                    if can_place_at_end {
                                        drag_insert_index = Some(process_count);
                                    }
                                    if let Some(insert_index) = drag_insert_index {
                                        self.draw_drag_insert_marker(ui, &row_bounds, insert_index);
                                    }
                                }
                                if let Some((process_id, target_index)) = reorder_to {
                                    self.move_process_to_index(&process_id, target_index);
                                    self.dragged_process = None;
                                } else if ctx.input(|input| input.pointer.any_released()) {
                                    self.dragged_process = None;
                                } else if let Some(process_id) = move_up_id {
                                    self.move_process_up(&process_id);
                                } else if let Some(process_id) = move_down_id {
                                    self.move_process_down(&process_id);
                                } else if let Some(process_id) = reload_process_id {
                                    self.reload_process_from_disk(&process_id);
                                }
                            });
                    });
            });
    }

    fn draw_content(&mut self, ctx: &Context) {
        CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(self.shell_bg)
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                let inset_rect = ui.max_rect();
                let inset_radius = CornerRadius {
                    nw: WINDOW_CORNER_RADIUS,
                    ne: 0,
                    sw: WINDOW_CORNER_RADIUS,
                    se: 0,
                };

                ui.painter().rect_filled(inset_rect, inset_radius, BODY_BG);

                ui.scope_builder(UiBuilder::new().max_rect(inset_rect), |ui| {
                    if let Some(process) = self.selected_process_config() {
                        self.draw_process_detail(ui, &process);
                    } else {
                        self.draw_empty_state(ui);
                    }
                });
            });
    }

    fn draw_empty_state(&self, ui: &mut Ui) {
        ui.with_layout(
            Layout::centered_and_justified(egui::Direction::TopDown),
            |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Let's build")
                            .color(TEXT_MAIN)
                            .size(32.0)
                            .strong(),
                    );
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("Select a process or add a new one.")
                            .color(TEXT_MUTED)
                            .size(16.0),
                    );
                });
            },
        );
    }

    fn draw_process_detail(&mut self, ui: &mut Ui, process: &ProcessConfig) {
        let logs = self.runtime_snapshot.selected_logs.clone();
        let visible_log_start = self
            .runtime_snapshot
            .selected_log_count
            .saturating_sub(logs.len());
        let selected_log_count =
            self.visible_selected_log_count(&process.id, visible_log_start, logs.len());
        let auto_start = if process.auto_start { "ON" } else { "OFF" };
        let managed_restart = if process.auto_restart { "ON" } else { "OFF" };
        let global_controls = global_controls_summary(process);
        let metadata = format!(
            "{} | {} | auto-start {} | delay {}s | restart {} | global {}",
            match &process.process_type {
                ProcessType::Process => "Process",
                ProcessType::Docker => "Docker",
            },
            &process.command,
            auto_start,
            process.startup_delay_seconds,
            managed_restart,
            global_controls
        );
        let mut action_start = false;
        let mut action_stop = false;
        let mut action_restart = false;
        let mut action_edit = false;
        let mut action_delete = false;
        let mut action_copy_logs = false;
        let mut action_clear_log_selection = false;

        // Single compact header row: process actions left, metadata uses the remaining space.
        egui::Frame::default()
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE)
            .inner_margin(egui::Margin::symmetric(CONTENT_GUTTER_X, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let action_width = ui.available_width().min(if selected_log_count > 0 {
                        520.0
                    } else {
                        360.0
                    });
                    ui.allocate_ui_with_layout(
                        Vec2::new(action_width, 28.0),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if chrome_text_button(
                                ui,
                                "▶ Start",
                                TOOLBAR_GREEN,
                                Vec2::new(0.0, 28.0),
                                12.0,
                                false,
                            )
                            .clicked()
                            {
                                action_start = true;
                            }
                            if chrome_text_button(
                                ui,
                                "■ Stop",
                                TOOLBAR_GRAY,
                                Vec2::new(0.0, 28.0),
                                12.0,
                                false,
                            )
                            .clicked()
                            {
                                action_stop = true;
                            }
                            if chrome_text_button(
                                ui,
                                "⟳ Restart",
                                TOOLBAR_YELLOW,
                                Vec2::new(0.0, 28.0),
                                12.0,
                                false,
                            )
                            .clicked()
                            {
                                action_restart = true;
                            }
                            if selected_log_count > 0 {
                                if chrome_text_button(
                                    ui,
                                    "📋 Copy Logs",
                                    TOOLBAR_TEXT,
                                    Vec2::new(0.0, 28.0),
                                    12.0,
                                    false,
                                )
                                .on_hover_text(format!(
                                    "Copy {selected_log_count} selected log lines"
                                ))
                                .clicked()
                                {
                                    action_copy_logs = true;
                                }
                                if chrome_text_button(
                                    ui,
                                    "Clear",
                                    TOOLBAR_GRAY,
                                    Vec2::new(0.0, 28.0),
                                    12.0,
                                    false,
                                )
                                .on_hover_text("Clear log selection")
                                .clicked()
                                {
                                    action_clear_log_selection = true;
                                }
                            }
                            ui.add_space(2.0);
                            let (sep_rect, _) =
                                ui.allocate_exact_size(Vec2::new(1.0, 18.0), egui::Sense::hover());
                            ui.painter().vline(
                                sep_rect.center().x,
                                sep_rect.y_range(),
                                Stroke::new(1.0, Color32::from_white_alpha(15)),
                            );
                            ui.add_space(2.0);
                            if chrome_text_button(
                                ui,
                                "⚙ Edit",
                                TOOLBAR_TEXT,
                                Vec2::new(0.0, 28.0),
                                12.0,
                                false,
                            )
                            .clicked()
                            {
                                action_edit = true;
                            }
                            if chrome_text_button(
                                ui,
                                "✕ Delete",
                                TOOLBAR_RED,
                                Vec2::new(0.0, 28.0),
                                12.0,
                                false,
                            )
                            .clicked()
                            {
                                action_delete = true;
                            }
                        },
                    );

                    ui.add_space(10.0);

                    let metadata_response = ui.add_sized(
                        Vec2::new(ui.available_width().max(0.0), 28.0),
                        egui::Label::new(
                            RichText::new(metadata.as_str())
                                .color(TEXT_MUTED)
                                .size(11.5),
                        )
                        .truncate(),
                    );
                    metadata_response.on_hover_text(metadata);
                });

                // Thin separator
                ui.add_space(8.0);
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    Stroke::new(1.0, Color32::from_white_alpha(10)),
                );
            });

        egui::Frame::default()
            .fill(Color32::TRANSPARENT)
            .inner_margin(egui::Margin::symmetric(CONTENT_GUTTER_X, 12))
            .show(ui, |ui| {
                let remaining_height = ui.available_height();
                if logs.is_empty() {
                    ui.set_min_height(remaining_height.max(0.0));
                    ui.label(
                        RichText::new("No output yet. Start the process to see logs.")
                            .color(TEXT_SOFT)
                            .monospace(),
                    );
                } else {
                    let output = ScrollArea::vertical()
                        .id_salt(("process_logs", &process.id))
                        .auto_shrink([false, false])
                        .max_height(remaining_height.max(0.0))
                        .stick_to_bottom(self.stick_logs_to_bottom)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(0.0, 4.0);

                            for (offset, line) in logs.iter().enumerate() {
                                let log_index = visible_log_start + offset;
                                let style = classify_log_line(line);
                                let frozen = self.is_frozen_log_line(&process.id, log_index);
                                let text_selection_frozen =
                                    self.is_log_text_selection_frozen(&process.id);
                                let selected = frozen
                                    || self.selected_log_range(&process.id).is_some_and(
                                        |(start, end)| (start..=end).contains(&log_index),
                                    );
                                let response = draw_log_line(ui, line, style, selected, frozen);
                                if frozen && response.clicked_elsewhere() {
                                    self.clear_log_selection();
                                } else if response.double_clicked() {
                                    self.freeze_log_line(&process.id, log_index);
                                } else if !text_selection_frozen && response.clicked() {
                                    let extend_range = ui.input(|input| input.modifiers.shift);
                                    self.select_log_line(&process.id, log_index, extend_range);
                                }
                            }
                        });

                    let max_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
                    let distance_from_bottom = (max_offset - output.state.offset.y).max(0.0);
                    self.stick_logs_to_bottom = distance_from_bottom <= LOG_STICK_THRESHOLD_PX;
                }
            });

        if action_delete {
            self.delete_process_id = Some(process.id.clone());
        }
        if action_edit {
            self.open_edit_process(&process.id);
        }
        if action_restart {
            self.manager.restart_process(&process.id);
        }
        if action_copy_logs {
            self.copy_selected_logs();
        }
        if action_clear_log_selection {
            self.clear_log_selection();
        }
        if action_stop {
            self.manager.stop_process(&process.id);
        }
        if action_start {
            self.manager.start_process(&process.id);
        }
    }

    fn draw_process_dialog(&mut self, ctx: &Context) {
        let mut close_dialog = false;
        let mut submit_dialog = false;

        if let Some(dialog) = self.process_dialog.as_mut() {
            let mut open = true;
            Window::new(dialog.title())
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .collapsible(false)
                .resizable(false)
                .fixed_size([430.0, PROCESS_DIALOG_HEIGHT])
                .frame(
                    egui::Frame::window(&ctx.style())
                        .fill(PANEL_BG)
                        .stroke(Stroke::new(1.0, BORDER)),
                )
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.set_width(430.0);
                    let form = dialog.form_mut();
                    let content_height = (ui.available_height() - MODAL_FOOTER_HEIGHT).max(140.0);

                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), content_height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.scope(|ui| {
                                let scroll = &mut ui.style_mut().spacing.scroll;
                                *scroll = egui::style::ScrollStyle::solid();
                                scroll.bar_width = 12.0;
                                scroll.handle_min_length = 28.0;
                                scroll.bar_inner_margin = 4.0;
                                scroll.bar_outer_margin = 2.0;
                                scroll.foreground_color = false;
                                scroll.dormant_background_opacity = 1.0;
                                scroll.dormant_handle_opacity = 1.0;
                                scroll.active_background_opacity = 1.0;
                                scroll.active_handle_opacity = 1.0;
                                scroll.interact_background_opacity = 1.0;
                                scroll.interact_handle_opacity = 1.0;
                                let visuals = ui.visuals_mut();
                                visuals.extreme_bg_color = Color32::from_gray(38);
                                visuals.widgets.inactive.bg_fill = Color32::from_gray(92);
                                visuals.widgets.hovered.bg_fill = Color32::from_gray(108);
                                visuals.widgets.active.bg_fill = Color32::from_gray(116);

                                ScrollArea::vertical()
                                    .id_salt("process_dialog_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(content_height)
                                    .scroll_bar_visibility(
                                        egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                                    )
                                    .show(ui, |ui| {
                                        ui.set_width(MODAL_FORM_WIDTH);
                                        ui.label(field_label("Name"));
                                        modal_text_edit(
                                            ui,
                                            &mut form.name,
                                            "Frontend Dev Server",
                                            MODAL_FORM_WIDTH,
                                        );

                                        ui.add_space(14.0);
                                        ui.label(field_label("Type"));
                                        ui.horizontal(|ui| {
                                            modal_tab_button(
                                                ui,
                                                &mut form.process_type,
                                                ProcessType::Process,
                                                "Process",
                                            );
                                            modal_tab_button(
                                                ui,
                                                &mut form.process_type,
                                                ProcessType::Docker,
                                                "Docker",
                                            );
                                        });

                                        ui.add_space(14.0);
                                        ui.label(field_label(if form.process_type == ProcessType::Docker {
                                            "Container Name"
                                        } else {
                                            "Command"
                                        }));
                                        modal_text_edit(
                                            ui,
                                            &mut form.command,
                                            if form.process_type == ProcessType::Docker {
                                                "my-postgres-container"
                                            } else {
                                                "npm run dev"
                                            },
                                            MODAL_FORM_WIDTH,
                                        );

                                        if form.process_type == ProcessType::Process {
                                            ui.add_space(14.0);
                                            ui.label(field_label("Working Directory"));
                                            modal_text_edit(
                                                ui,
                                                &mut form.working_directory,
                                                "C:/projects/my-app",
                                                MODAL_FORM_WIDTH,
                                            );
                                        }

                                        ui.add_space(16.0);
                                        modal_divider(ui);
                                        ui.add_space(16.0);

                                        modal_checkbox_row(
                                            ui,
                                            &mut form.auto_start,
                                            "Auto-start with app launch",
                                            Some("Start this entry automatically whenever Process Manager starts."),
                                        );

                                        ui.add_space(14.0);
                                        ui.label(field_label("Startup Delay (seconds)"));
                                        modal_text_edit(
                                            ui,
                                            &mut form.startup_delay_seconds,
                                            "0",
                                            MODAL_FORM_WIDTH,
                                        );
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new("Wait before launching this entry after any start request. Use 0 to start immediately.")
                                                .color(TEXT_MUTED)
                                                .size(11.5),
                                        );

                                        ui.add_space(14.0);
                                        modal_checkbox_row(
                                            ui,
                                            &mut form.auto_restart,
                                            "Managed restart",
                                            Some("Automatically restart this entry if it goes down."),
                                        );

                                        ui.add_space(14.0);
                                        if form.auto_restart {
                                            draw_restart_schedule_summary(ui, form);
                                            ui.add_space(14.0);
                                        } else {
                                            form.restart_schedule.enabled = false;
                                        }

                                        modal_checkbox_row(
                                            ui,
                                            &mut form.scheduled_run.enabled,
                                            "Scheduled run",
                                            Some("Start this entry from a time-based schedule when it is not already running."),
                                        );
                                        ui.add_space(14.0);
                                        if form.scheduled_run.enabled {
                                            draw_scheduled_run_summary(ui, form);
                                            ui.add_space(14.0);
                                        }

                                        draw_stack_control_group(ui, form);

                                        ui.add_space(14.0);
                                        modal_checkbox_row(
                                            ui,
                                            &mut form.log_to_disk,
                                            "Write logs to disk",
                                            Some(
                                                "Store a full session log under the global Process Manager logs directory.",
                                            ),
                                        );

                                        ui.add_space(14.0);
                                        ui.label(field_label("Logs To Keep"));
                                        if form.log_to_disk {
                                            modal_text_edit(
                                                ui,
                                                &mut form.log_rotation_count,
                                                "10",
                                                MODAL_FORM_WIDTH,
                                            );
                                        } else {
                                            modal_disabled_text_edit(
                                                ui,
                                                &mut form.log_rotation_count,
                                                MODAL_FORM_WIDTH,
                                            );
                                        }
                                        ui.add_space(6.0);
                                        ui.label(
                                            RichText::new("Number of session log files to retain for this process.")
                                                .color(TEXT_MUTED)
                                                .size(11.5),
                                        );
                                    });
                            });
                        },
                    );

                    modal_footer(ui, |ui| {
                        if subtle_action_button(ui, "Save", Some(ACCENT_SOFT)).clicked() {
                            submit_dialog = true;
                        }
                        if shell_button(ui, "Cancel").clicked() {
                            close_dialog = true;
                        }
                    });
                });

            draw_process_schedule_editors(ctx, dialog.form_mut());

            if !open {
                close_dialog = true;
            }
        }

        if submit_dialog {
            if let Some(dialog) = self.process_dialog.take() {
                self.apply_process_dialog(dialog);
            }
        } else if close_dialog {
            self.process_dialog = None;
        }
    }

    fn draw_rest_settings_dialog(&mut self, ctx: &Context) {
        if !self.rest_settings_open {
            return;
        }

        let mut open = true;
        let mut save = false;
        let mut browse_for_logs = false;
        let mut host_text = "127.0.0.1".to_string();

        Window::new("Global Settings")
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .default_size([420.0, GLOBAL_SETTINGS_HEIGHT])
            .min_size([420.0, GLOBAL_SETTINGS_HEIGHT])
            .max_size([420.0, GLOBAL_SETTINGS_HEIGHT])
            .fixed_size([420.0, GLOBAL_SETTINGS_HEIGHT])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(PANEL_BG)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                let content_height = (ui.available_height() - MODAL_FOOTER_HEIGHT).max(140.0);

                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), content_height),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.horizontal(|ui| {
                            modal_tab_button(
                                ui,
                                &mut self.global_settings_tab,
                                0usize,
                                "Process Manager",
                            );
                            modal_tab_button(
                                ui,
                                &mut self.global_settings_tab,
                                1usize,
                                "Local API",
                            );
                            modal_tab_button(ui, &mut self.global_settings_tab, 2usize, "About");
                        });
                        ui.add_space(10.0);
                        let (rect, _) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 1.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().hline(
                            rect.x_range(),
                            rect.center().y,
                            Stroke::new(1.0, Color32::from_white_alpha(10)),
                        );
                        ui.add_space(14.0);

                        ui.vertical(|ui| {
                            if self.global_settings_tab == 0 {
                                ui.label(field_label("Stack Name"));
                                modal_text_edit(
                                    ui,
                                    &mut self.stack_name_buffer,
                                    "My Stack",
                                    MODAL_FORM_WIDTH,
                                );
                                ui.add_space(14.0);
                                ui.label(field_label("Log Directory"));
                                let (_, browse_response) = modal_text_edit_with_button(
                                    ui,
                                    &mut self.rest_settings_form.log_directory,
                                    ".",
                                    MODAL_FORM_WIDTH,
                                    "Browse",
                                );
                                if browse_response.clicked() {
                                    browse_for_logs = true;
                                }
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(
                                        "Process Manager creates 'Process Manager logs' here. '.' resolves next to the executable.",
                                    )
                                    .color(TEXT_MUTED)
                                    .size(11.5),
                                );
                                ui.add_space(14.0);
                                ui.label(field_label("Error Flash Duration (seconds)"));
                                modal_text_edit(
                                    ui,
                                    &mut self.rest_settings_form.process_error_flash_seconds,
                                    "5",
                                    MODAL_FORM_WIDTH,
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new(
                                        "Softly flashes the Processes sidebar when a new error arrives. Use 0 to disable.",
                                    )
                                    .color(TEXT_MUTED)
                                    .size(11.5),
                                );
                            } else if self.global_settings_tab == 1 {
                                modal_checkbox_row(
                                    ui,
                                    &mut self.rest_settings_form.enabled,
                                    "Enable localhost REST control",
                                    Some("Expose a loopback-only API for local tooling."),
                                );
                                ui.add_space(14.0);
                                ui.label(field_label("Host"));
                                modal_disabled_text_edit(ui, &mut host_text, MODAL_FORM_WIDTH);
                                ui.add_space(14.0);
                                ui.label(field_label("Port"));
                                modal_text_edit(
                                    ui,
                                    &mut self.rest_settings_form.port,
                                    "3000",
                                    MODAL_FORM_WIDTH,
                                );
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("The API binds only to 127.0.0.1.")
                                        .color(TEXT_MUTED)
                                        .size(11.5),
                                );
                            } else if self.global_settings_tab == 2 {
                                ui.label(
                                    RichText::new("Project Information")
                                        .color(TEXT_MAIN)
                                        .size(14.0)
                                        .strong(),
                                );
                                ui.add_space(14.0);

                                for field in about_fields() {
                                    draw_about_field(ui, &field);
                                    ui.add_space(10.0);
                                }

                            }

                            if let Some(error) = &self.rest_settings_error {
                                ui.add_space(12.0);
                                ui.label(RichText::new(error).color(DANGER).size(12.0));
                            }
                        });
                    },
                );

                modal_footer(ui, |ui| {
                    if self.global_settings_tab != 2
                        && subtle_action_button(ui, "Save", Some(ACCENT_SOFT)).clicked()
                    {
                        save = true;
                    }
                    let close_label = if self.global_settings_tab == 2 {
                        "Close"
                    } else {
                        "Cancel"
                    };
                    if shell_button(ui, close_label).clicked() {
                        self.rest_settings_open = false;
                    }
                });
            });

        if !open {
            self.rest_settings_open = false;
        }

        if browse_for_logs {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.rest_settings_form.log_directory = path.display().to_string();
            }
        }

        if save {
            self.save_rest_settings();
        }
    }

    fn draw_delete_dialog(&mut self, ctx: &Context) {
        let Some(process_id) = self.delete_process_id.clone() else {
            return;
        };

        let process_name = self
            .config
            .get_process(&process_id)
            .map(|process| process.name.clone())
            .unwrap_or_else(|| "this process".to_string());

        let mut open = true;
        let mut confirm = false;

        Window::new("Delete Process")
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(PANEL_BG)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.set_min_height(180.0);
                ui.label(
                    RichText::new("Delete Process")
                        .color(TEXT_MAIN)
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("Delete {}? This cannot be undone.", process_name))
                        .color(TEXT_SOFT)
                        .size(13.0),
                );

                modal_footer(ui, |ui| {
                    if subtle_action_button(ui, "Delete", Some(DANGER)).clicked() {
                        confirm = true;
                    }
                    if shell_button(ui, "Cancel").clicked() {
                        self.delete_process_id = None;
                    }
                });
            });

        if !open {
            self.delete_process_id = None;
        }

        if confirm {
            self.delete_process(&process_id);
            self.delete_process_id = None;
        }
    }

    fn draw_reload_dialog(&mut self, ctx: &Context) {
        if !self.reload_processes_confirm_open {
            return;
        }

        let mut open = true;
        let mut confirm = false;

        Window::new("Reload processes.json")
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(PANEL_BG)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.set_min_height(220.0);
                ui.label(
                    RichText::new("Reload processes.json")
                        .color(TEXT_MAIN)
                        .size(16.0)
                        .strong(),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new(
                        "This will stop all managed processes first, including those not opted into Stop All."
                            .to_string(),
                    )
                    .color(TEXT_SOFT)
                    .size(13.0),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "After that, the local processes.json file will be reloaded and the process list will be rebuilt."
                            .to_string(),
                    )
                    .color(TEXT_SOFT)
                    .size(13.0),
                );

                modal_footer(ui, |ui| {
                    if subtle_action_button(ui, "Reload", Some(TOOLBAR_YELLOW)).clicked() {
                        confirm = true;
                    }
                    if shell_button(ui, "Cancel").clicked() {
                        self.cancel_reload_processes_confirmation();
                    }
                });
            });

        if !open {
            self.cancel_reload_processes_confirmation();
        }

        if confirm {
            self.reload_processes_from_disk();
            self.reload_processes_confirm_open = false;
        }
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

        apply_windows_caption_theme(hwnd, self.shell_bg);
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
}

impl eframe::App for ProcessManagerApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let update_started = Instant::now();
        self.ensure_valid_selection();
        self.update_title(ctx);
        self.ensure_windows_native_caption();
        self.ensure_windows_taskbar_icon();
        let focused = ctx.input(|input| input.viewport().focused).unwrap_or(true);
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

        let caption_changed = self.refresh_shell_bg_from_windows_caption(focused);
        self.handle_shortcuts(ctx);
        self.maybe_request_attention(ctx);
        self.refresh_runtime_snapshot(false);

        // Keep global panel_fill in sync with the live shell_bg from caption probe
        ctx.style_mut(|style| {
            style.visuals.panel_fill = self.shell_bg;
            style.visuals.window_fill = PANEL_BG;
            style.visuals.faint_bg_color = PANEL_BG;
            style.visuals.extreme_bg_color = BODY_BG;
        });

        if caption_changed || viewport_pos_changed || viewport_size_changed {
            ctx.request_repaint();
        }
        if let Some(delay) = self.next_repaint_delay() {
            ctx.request_repaint_after(delay);
        }

        self.draw_sidebar(ctx);
        self.draw_header(ctx);
        self.draw_content(ctx);
        self.draw_process_dialog(ctx);
        self.draw_rest_settings_dialog(ctx);
        self.draw_delete_dialog(ctx);
        self.draw_reload_dialog(ctx);
        self.draw_diagnostics_overlay(ctx);
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

fn configure_visuals(ctx: &Context) {
    let mut visuals = egui::Visuals::dark();
    // Do NOT set override_text_color — it prevents selected text from being visible
    visuals.panel_fill = SHELL_BG;
    visuals.window_fill = PANEL_BG;
    visuals.extreme_bg_color = BODY_BG;
    visuals.faint_bg_color = PANEL_BG;
    visuals.widgets.noninteractive.bg_fill = SHELL_BG;
    visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_SOFT);
    visuals.widgets.inactive.bg_fill = SHELL_BG;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_MAIN);
    visuals.widgets.hovered.bg_fill = SHELL_HOVER_BG;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_MAIN);
    visuals.widgets.active.bg_fill = SHELL_ACTIVE_BG;
    visuals.widgets.active.bg_stroke = Stroke::NONE;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT_MAIN);
    visuals.selection.bg_fill = Color32::from_rgb(56, 98, 158);
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(86, 136, 198));
    visuals.window_shadow.color = Color32::TRANSPARENT;
    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.button_padding = Vec2::new(10.0, 6.0);
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.indent = 16.0;
    });
}

#[cfg(windows)]
fn apply_windows_caption_theme(hwnd: windows_sys::Win32::Foundation::HWND, color: Color32) {
    use std::ffi::c_void;

    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWA_BORDER_COLOR: u32 = 34;
    const DWMWA_CAPTION_COLOR: u32 = 35;
    const DWMWA_TEXT_COLOR: u32 = 36;
    const DWMWCP_ROUND: u32 = 2;

    let dark_mode_enabled: i32 = (!is_light_color(color)) as i32;
    let corner_preference = DWMWCP_ROUND;
    let border_color = color_to_colorref(BORDER);
    let caption_color = color_to_colorref(color);
    let text_color = color_to_colorref(if is_light_color(color) {
        Color32::from_rgb(24, 24, 24)
    } else {
        Color32::from_rgb(237, 237, 237)
    });

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

fn should_accept_caption_color(color: Color32) -> bool {
    let r = color.r() as f32 / 255.0;
    let g = color.g() as f32 / 255.0;
    let b = color.b() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let saturation = if max <= f32::EPSILON {
        0.0
    } else {
        (max - min) / max
    };

    luminance < 0.24 || saturation > 0.12
}

fn is_light_color(color: Color32) -> bool {
    let r = color.r() as f32 / 255.0;
    let g = color.g() as f32 / 255.0;
    let b = color.b() as f32 / 255.0;
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    luminance >= 0.5
}

#[cfg(windows)]
fn color_to_colorref(color: Color32) -> u32 {
    (color.r() as u32) | ((color.g() as u32) << 8) | ((color.b() as u32) << 16)
}

fn configure_fonts(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    let mut loaded_segoe_ui = false;

    if let Ok(bytes) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
        fonts
            .font_data
            .insert("Segoe UI".into(), egui::FontData::from_owned(bytes).into());
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.insert(0, "Segoe UI".into());
        }
        loaded_segoe_ui = true;
    }

    // Segoe UI Symbol provides Unicode symbols and icons that Segoe UI lacks
    if let Ok(bytes) = std::fs::read("C:/Windows/Fonts/seguisym.ttf") {
        fonts.font_data.insert(
            "Segoe UI Symbol".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.push("Segoe UI Symbol".into());
        }
    }

    let monospace_font = [
        ("C:/Windows/Fonts/CascadiaMono.ttf", "Cascadia Mono"),
        ("C:/Windows/Fonts/consola.ttf", "Consolas"),
        ("C:/Windows/Fonts/lucon.ttf", "Lucida Console"),
    ]
    .into_iter()
    .find_map(|(path, name)| {
        std::fs::read(path)
            .ok()
            .map(|bytes| (name.to_string(), egui::FontData::from_owned(bytes)))
    });

    if let Some((name, font_data)) = monospace_font {
        fonts.font_data.insert(name.clone(), font_data.into());
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            family.insert(0, name);
        }
    } else if loaded_segoe_ui {
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
            family.insert(0, "Segoe UI".into());
        }
    }

    ctx.set_fonts(fonts);
}

fn window_title(stack_name: &str) -> String {
    format!("Process Manager - {}", stack_name)
}

fn about_fields() -> Vec<AboutField> {
    vec![
        AboutField {
            label: "Name",
            value: env!("CARGO_PKG_NAME").to_string(),
            link: None,
        },
        AboutField {
            label: "Version",
            value: env!("CARGO_PKG_VERSION").to_string(),
            link: None,
        },
        AboutField {
            label: "Author",
            value: env!("CARGO_PKG_AUTHORS").to_string(),
            link: Some(PROJECT_GITHUB_ACCOUNT_URL),
        },
        AboutField {
            label: "Website",
            value: PROJECT_WEBSITE_URL.to_string(),
            link: Some(PROJECT_WEBSITE_URL),
        },
        AboutField {
            label: "GitHub",
            value: PROJECT_GITHUB_ACCOUNT_HANDLE.to_string(),
            link: Some(PROJECT_GITHUB_ACCOUNT_URL),
        },
    ]
}

fn draw_about_field(ui: &mut Ui, field: &AboutField) {
    egui::Frame::default()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, FIELD_BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(MODAL_FORM_WIDTH);
            ui.label(field_label(field.label));
            ui.add_space(4.0);

            if let Some(link) = field.link {
                ui.hyperlink_to(
                    RichText::new(&field.value).color(TEXT_MAIN).size(13.0),
                    link,
                );
            } else {
                ui.label(RichText::new(&field.value).color(TEXT_MAIN).size(13.0));
            }
        });
}

fn stack_summary(counts: &ProcessCounts) -> String {
    format!(
        "{} running | {} stopped | {} starting | {} errors",
        counts.running, counts.stopped, counts.starting, counts.error
    )
}

fn shell_button(ui: &mut Ui, label: &str) -> egui::Response {
    chrome_button(ui, label, None, Vec2::new(0.0, 28.0))
}

fn subtle_action_button(ui: &mut Ui, label: &str, accent: Option<Color32>) -> egui::Response {
    toolbar_button(ui, label, accent, Vec2::new(0.0, 28.0))
}

fn toolbar_button(
    ui: &mut Ui,
    label: &str,
    accent: Option<Color32>,
    min_size: Vec2,
) -> egui::Response {
    chrome_text_button(
        ui,
        label,
        accent.unwrap_or(TEXT_MAIN),
        min_size,
        12.0,
        false,
    )
}

fn chrome_button(
    ui: &mut Ui,
    label: &str,
    accent: Option<Color32>,
    min_size: Vec2,
) -> egui::Response {
    chrome_text_button(ui, label, accent.unwrap_or(TEXT_MAIN), min_size, 12.0, true)
}

fn chrome_text_button(
    ui: &mut Ui,
    label: &str,
    text_color: Color32,
    min_size: Vec2,
    font_size: f32,
    show_idle_stroke: bool,
) -> egui::Response {
    ui.scope(|ui| {
        let visuals = &mut ui.style_mut().visuals;
        visuals.widgets.inactive.bg_fill = SHELL_BG;
        visuals.widgets.inactive.bg_stroke = if show_idle_stroke {
            Stroke::new(1.0, SHELL_STROKE)
        } else {
            Stroke::NONE
        };
        visuals.widgets.hovered.bg_fill = SHELL_HOVER_BG;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, SHELL_STROKE);
        visuals.widgets.active.bg_fill = SHELL_ACTIVE_BG;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, SHELL_STROKE);
        visuals.widgets.open.bg_fill = SHELL_ACTIVE_BG;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, SHELL_STROKE);

        ui.add(
            Button::new(RichText::new(label).color(text_color).size(font_size))
                .frame(true)
                .frame_when_inactive(show_idle_stroke)
                .corner_radius(6.0)
                .min_size(min_size),
        )
    })
    .inner
}

fn draw_process_row(
    ui: &mut Ui,
    process: &ProcessConfig,
    status: &ProcessStatus,
    selected: bool,
    flash_intensity: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 32.0),
        egui::Sense::click_and_drag(),
    );

    let is_hovered = response.hovered();

    let base_bg_color = if selected {
        PROCESS_ROW_SELECTED_BG
    } else if is_hovered {
        PROCESS_ROW_HOVER_BG
    } else {
        Color32::TRANSPARENT
    };
    let bg_color = blend_color(base_bg_color, DANGER, 0.62 * flash_intensity);

    if bg_color != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 4.0, bg_color);
    }

    if flash_intensity > 0.0 {
        let stroke_color = blend_color(SHELL_BG, DANGER, 0.92 * flash_intensity);
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.0, stroke_color),
            egui::StrokeKind::Outside,
        );
    }

    if selected {
        let accent_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(2.0, 8.0),
            egui::vec2(2.0, rect.height() - 16.0),
        );
        ui.painter().rect_filled(accent_rect, 1.0, TEXT_MAIN);
    }

    let inner_rect = rect.shrink2(egui::vec2(14.0, 0.0));
    let dot_center = egui::pos2(inner_rect.min.x + 10.0, rect.center().y);
    ui.painter()
        .circle_filled(dot_center, 4.0, status_color(status, ui.ctx()));
    let text_pos = egui::pos2(dot_center.x + 14.0, rect.center().y);
    let font_id = FontId::proportional(13.5);
    let text_color = if selected { TEXT_MAIN } else { TEXT_MUTED };
    ui.painter().text(
        text_pos,
        Align2::LEFT_CENTER,
        &process.name,
        font_id.clone(),
        text_color,
    );

    let markers = process_markers(process);
    if let Some(marker_text) = markers.as_deref() {
        let name_width = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(process.name.clone(), font_id.clone(), text_color)
                .size()
                .x
        });
        let marker_color = if selected { TEXT_SOFT } else { STOPPED };
        ui.painter().text(
            egui::pos2(text_pos.x + name_width, text_pos.y),
            Align2::LEFT_CENTER,
            marker_text,
            font_id,
            marker_color,
        );
    }

    response
}

fn field_label(text: &str) -> RichText {
    RichText::new(text).color(TEXT_SOFT).size(12.0).strong()
}

fn process_markers(process: &ProcessConfig) -> Option<String> {
    let mut markers = Vec::new();
    if process.auto_start {
        markers.push("A");
    }
    if process.auto_restart {
        markers.push("M");
    }

    if markers.is_empty() {
        None
    } else {
        Some(format!(" ({})", markers.join(",")))
    }
}

fn process_tab_label(process: &ProcessConfig) -> String {
    match process_markers(process) {
        Some(markers) => format!("{}{}", process.name, markers),
        None => process.name.clone(),
    }
}

fn draw_log_line(
    ui: &mut Ui,
    line: &str,
    style: LogLineStyle,
    selected: bool,
    text_selectable: bool,
) -> egui::Response {
    let fill = if selected {
        Color32::from_rgb(42, 58, 82)
    } else {
        Color32::TRANSPARENT
    };

    let shown = egui::Frame::default()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut label = egui::Label::new(
                RichText::new(line)
                    .color(style.color)
                    .monospace()
                    .size(12.5),
            )
            .selectable(text_selectable);

            if !text_selectable {
                label = label.sense(egui::Sense::click());
            }

            ui.add(label)
            .on_hover_text(format!(
                "{}\n{}",
                style.hover,
                if text_selectable {
                    "Drag to select text; click outside this row to return to row selection."
                } else {
                    "Click to select; Shift-click another line to select a range; double-click to select text."
                }
            ))
        });

    if text_selectable {
        shown.response.union(shown.inner)
    } else {
        shown
            .response
            .interact(egui::Sense::click())
            .union(shown.inner)
    }
}

fn global_controls_summary(process: &ProcessConfig) -> &'static str {
    match (
        process.respond_to_start_all,
        process.respond_to_stop_all,
        process.respond_to_restart_all,
    ) {
        (true, true, true) => "all",
        (false, false, false) => "ignored",
        _ => "custom",
    }
}

fn draw_sidebar_footer_button(ui: &mut Ui, label: &str) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());

    let bg_color = if response.hovered() {
        PROCESS_ROW_HOVER_BG
    } else {
        Color32::TRANSPARENT
    };

    if bg_color != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, 4.0, bg_color);
    }

    let inner_rect = rect.shrink2(egui::vec2(14.0, 0.0));
    let icon_pos = egui::pos2(inner_rect.min.x + 10.0, rect.center().y);
    ui.painter().text(
        icon_pos,
        Align2::CENTER_CENTER,
        "⚙",
        FontId::proportional(13.0),
        TEXT_MUTED,
    );
    ui.painter().text(
        egui::pos2(icon_pos.x + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        TEXT_MUTED,
    );

    response
}

fn modal_text_edit(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.scope(|ui| {
        let visuals = &mut ui.style_mut().visuals;
        visuals.widgets.inactive.bg_fill = FIELD_BG;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, FIELD_BORDER);
        visuals.widgets.hovered.bg_fill = FIELD_BG_HOVER;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, FIELD_BORDER_FOCUS);
        visuals.widgets.active.bg_fill = FIELD_BG_HOVER;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, FIELD_BORDER_FOCUS);
        visuals.widgets.open.bg_fill = FIELD_BG_HOVER;
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, FIELD_BORDER_FOCUS);

        ui.add_sized(
            [width, 34.0],
            TextEdit::singleline(value)
                .hint_text(hint)
                .frame(true)
                .margin(egui::Margin::symmetric(8, 7)),
        )
    })
    .inner
}

fn modal_disabled_text_edit(ui: &mut Ui, value: &mut String, width: f32) -> egui::Response {
    ui.scope(|ui| {
        let visuals = &mut ui.style_mut().visuals;
        visuals.widgets.noninteractive.bg_fill = FIELD_BG;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, FIELD_BORDER_DISABLED);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_MUTED);

        ui.add_enabled_ui(false, |ui| {
            ui.add_sized(
                [width, 34.0],
                egui::widgets::TextEdit::singleline(value)
                    .frame(true)
                    .margin(egui::Margin::symmetric(8, 7))
                    .text_color(TEXT_MUTED),
            )
        })
        .inner
    })
    .inner
}

fn modal_text_edit_with_button(
    ui: &mut Ui,
    value: &mut String,
    hint: &str,
    total_width: f32,
    button_label: &str,
) -> (egui::Response, egui::Response) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let field_width = (total_width - BROWSE_BUTTON_WIDTH - 8.0).max(0.0);
        let field_response = modal_text_edit(ui, value, hint, field_width);
        let button_response = chrome_text_button(
            ui,
            button_label,
            TEXT_MAIN,
            Vec2::new(BROWSE_BUTTON_WIDTH, MODAL_BUTTON_HEIGHT),
            12.0,
            true,
        );
        (field_response, button_response)
    })
    .inner
}

fn modal_divider(ui: &mut Ui) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, SHELL_SUBTLE_STROKE),
    );
}

fn draw_restart_schedule_summary(ui: &mut Ui, form: &mut ProcessDraft) {
    egui::Frame::default()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, FIELD_BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(MODAL_FORM_WIDTH - 24.0);
            ui.checkbox(&mut form.restart_schedule.enabled, "Enable active schedule");
            ui.add_space(5.0);
            ui.checkbox(
                &mut form.restart_schedule.stop_when_inactive,
                "Stop when window ends",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(restart_schedule_summary(&form.restart_schedule))
                        .color(TEXT_MUTED)
                        .size(11.5),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if subtle_action_button(ui, "Edit hours...", Some(ACCENT_SOFT)).clicked() {
                        form.restart_schedule_editor_open = true;
                    }
                });
            });
        });
}

fn draw_scheduled_run_summary(ui: &mut Ui, form: &mut ProcessDraft) {
    egui::Frame::default()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, FIELD_BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(MODAL_FORM_WIDTH - 24.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(scheduled_run_summary(form))
                        .color(TEXT_MUTED)
                        .size(11.5),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if subtle_action_button(ui, "Edit schedule...", Some(ACCENT_SOFT)).clicked() {
                        form.scheduled_run_editor_open = true;
                    }
                });
            });
        });
}

fn draw_stack_control_group(ui: &mut Ui, form: &mut ProcessDraft) {
    egui::Frame::default()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, FIELD_BORDER))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(MODAL_FORM_WIDTH - 24.0);
            ui.label(field_label("Respond to stack controls"));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut form.respond_to_start_all, "Start All");
                ui.checkbox(&mut form.respond_to_stop_all, "Stop All");
                ui.checkbox(&mut form.respond_to_restart_all, "Restart All");
            });
        });
}

fn draw_process_schedule_editors(ctx: &Context, form: &mut ProcessDraft) {
    if form.restart_schedule_editor_open {
        let mut open = true;
        Window::new("Managed Restart Active Hours")
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .fixed_size([690.0, 430.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(PANEL_BG)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                normalize_weekly_hours(&mut form.restart_schedule.hours);
                ui.label(
                    RichText::new(
                        "Toggle the hours when managed restart should actively start and repeat this process.",
                    )
                    .color(TEXT_MUTED)
                    .size(12.0),
                );
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if shell_button(ui, "Clear").clicked() {
                        set_weekly_hours(&mut form.restart_schedule.hours, false);
                    }
                    if shell_button(ui, "All").clicked() {
                        set_weekly_hours(&mut form.restart_schedule.hours, true);
                    }
                    if shell_button(ui, "Weekdays").clicked() {
                        set_weekday_hours(&mut form.restart_schedule.hours, 0..5, 0..24);
                    }
                    if shell_button(ui, "Weekends").clicked() {
                        set_weekday_hours(&mut form.restart_schedule.hours, 5..7, 0..24);
                    }
                    if shell_button(ui, "Business Hours").clicked() {
                        set_weekday_hours(&mut form.restart_schedule.hours, 0..5, 9..17);
                    }
                    if shell_button(ui, "Nights").clicked() {
                        set_night_hours(&mut form.restart_schedule.hours);
                    }
                });

                ui.add_space(12.0);
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(292.0)
                    .show(ui, |ui| {
                        egui::Grid::new("restart_schedule_hours_grid")
                            .spacing([4.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("");
                                for hour in 0..24 {
                                    ui.label(
                                        RichText::new(format!("{hour:02}"))
                                            .color(TEXT_MUTED)
                                            .size(10.5),
                                    );
                                }
                                ui.end_row();

                                for day in 0..7 {
                                    ui.label(
                                        RichText::new(day_label(day))
                                            .color(TEXT_SOFT)
                                            .size(11.0),
                                    );
                                    for hour in 0..24 {
                                        let enabled = weekly_hour_enabled(
                                            &form.restart_schedule.hours,
                                            day,
                                            hour,
                                        );
                                        let label = if enabled { "ON" } else { "" };
                                        let response = ui.add(
                                            Button::new(RichText::new(label).size(9.0))
                                                .fill(if enabled {
                                                    TAB_SELECTED_BG
                                                } else {
                                                    FIELD_BG
                                                })
                                                .stroke(Stroke::new(
                                                    1.0,
                                                    if enabled {
                                                        TAB_SELECTED_STROKE
                                                    } else {
                                                        FIELD_BORDER
                                                    },
                                                ))
                                                .min_size(Vec2::new(22.0, 22.0)),
                                        );
                                        if response.clicked() {
                                            if let Some(index) = weekly_hour_index(day, hour) {
                                                form.restart_schedule.hours[index] = !enabled;
                                            }
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            });
        form.restart_schedule_editor_open = open;
    }

    if form.scheduled_run_editor_open {
        let mut open = true;
        Window::new("Scheduled Run")
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .collapsible(false)
            .resizable(false)
            .fixed_size([430.0, 340.0])
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(PANEL_BG)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(430.0);
                ui.label(
                    RichText::new("Choose when this process should receive a start request.")
                        .color(TEXT_MUTED)
                        .size(12.0),
                );
                ui.add_space(14.0);
                ui.label(field_label("Cadence"));
                ui.horizontal_wrapped(|ui| {
                    modal_tab_button(
                        ui,
                        &mut form.scheduled_run.mode,
                        ScheduledRunMode::Hourly,
                        "Hourly",
                    );
                    modal_tab_button(
                        ui,
                        &mut form.scheduled_run.mode,
                        ScheduledRunMode::EveryNHours,
                        "Every N hours",
                    );
                    modal_tab_button(
                        ui,
                        &mut form.scheduled_run.mode,
                        ScheduledRunMode::Daily,
                        "Daily",
                    );
                    modal_tab_button(
                        ui,
                        &mut form.scheduled_run.mode,
                        ScheduledRunMode::SelectedWeekdays,
                        "Weekdays",
                    );
                });

                ui.add_space(14.0);
                match form.scheduled_run.mode {
                    ScheduledRunMode::Hourly => {
                        ui.label(
                            RichText::new("Runs at the top of every hour.")
                                .color(TEXT_MUTED)
                                .size(11.5),
                        );
                    }
                    ScheduledRunMode::EveryNHours => {
                        ui.label(field_label("Every N Hours"));
                        modal_text_edit(
                            ui,
                            &mut form.scheduled_run_interval_hours,
                            "1",
                            MODAL_FORM_WIDTH,
                        );
                    }
                    ScheduledRunMode::Daily => {
                        ui.label(field_label("Local Hour"));
                        modal_text_edit(ui, &mut form.scheduled_run_hour, "9", MODAL_FORM_WIDTH);
                    }
                    ScheduledRunMode::SelectedWeekdays => {
                        ui.label(field_label("Local Hour"));
                        modal_text_edit(ui, &mut form.scheduled_run_hour, "9", MODAL_FORM_WIDTH);
                        ui.add_space(12.0);
                        ui.label(field_label("Days"));
                        normalize_weekdays(&mut form.scheduled_run.weekdays);
                        ui.horizontal_wrapped(|ui| {
                            for day in 0..7 {
                                ui.checkbox(&mut form.scheduled_run.weekdays[day], day_label(day));
                            }
                        });
                    }
                }
            });
        form.scheduled_run_editor_open = open;
    }
}

fn modal_tab_button<T>(ui: &mut Ui, current: &mut T, value: T, label: &str) -> egui::Response
where
    T: PartialEq + Clone,
{
    let selected = *current == value;
    let response = ui
        .scope(|ui| {
            let visuals = &mut ui.style_mut().visuals;
            visuals.widgets.inactive.bg_fill = if selected { TAB_SELECTED_BG } else { SHELL_BG };
            visuals.widgets.inactive.bg_stroke = Stroke::new(
                1.0,
                if selected {
                    TAB_SELECTED_STROKE
                } else {
                    SHELL_SUBTLE_STROKE
                },
            );
            visuals.widgets.hovered.bg_fill = if selected {
                TAB_SELECTED_HOVER_BG
            } else {
                SHELL_HOVER_BG
            };
            visuals.widgets.hovered.bg_stroke = Stroke::new(
                1.0,
                if selected {
                    TAB_SELECTED_STROKE
                } else {
                    SHELL_STROKE
                },
            );
            visuals.widgets.active.bg_fill = TAB_SELECTED_ACTIVE_BG;
            visuals.widgets.active.bg_stroke = Stroke::new(1.0, TAB_SELECTED_STROKE);
            visuals.widgets.open.bg_fill = TAB_SELECTED_ACTIVE_BG;
            visuals.widgets.open.bg_stroke = Stroke::new(1.0, TAB_SELECTED_STROKE);

            ui.add(
                Button::new(
                    RichText::new(label)
                        .color(if selected { TEXT_MAIN } else { TEXT_MUTED })
                        .size(12.0),
                )
                .frame(true)
                .corner_radius(6.0)
                .min_size(Vec2::new(0.0, 30.0)),
            )
        })
        .inner;

    if response.clicked() {
        *current = value.clone();
    }

    response
}

fn modal_checkbox_row(
    ui: &mut Ui,
    checked: &mut bool,
    label: &str,
    description: Option<&str>,
) -> egui::Response {
    let response = ui
        .scope(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(MODAL_FORM_WIDTH, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    let visuals = &mut ui.style_mut().visuals;
                    visuals.widgets.inactive.bg_fill = FIELD_BG;
                    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, FIELD_BORDER);
                    visuals.widgets.hovered.bg_fill = FIELD_BG_HOVER;
                    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, FIELD_BORDER_FOCUS);
                    visuals.widgets.active.bg_fill = FIELD_BG_HOVER;
                    visuals.widgets.active.bg_stroke = Stroke::new(1.0, FIELD_BORDER_FOCUS);

                    egui::Frame::default()
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::new(1.0, FIELD_BORDER))
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let response = ui.checkbox(checked, label);
                            if let Some(description) = description {
                                ui.add_space(4.0);
                                ui.label(RichText::new(description).color(TEXT_MUTED).size(11.5));
                            }
                            response
                        })
                        .inner
                },
            )
            .inner
        })
        .inner;

    response
}

fn modal_footer(ui: &mut Ui, add_actions: impl FnOnce(&mut Ui)) {
    let (footer_rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), MODAL_FOOTER_HEIGHT),
        egui::Sense::hover(),
    );
    ui.painter().hline(
        footer_rect.x_range(),
        footer_rect.top(),
        Stroke::new(1.0, SHELL_SUBTLE_STROKE),
    );

    let actions_rect = footer_rect.shrink2(egui::vec2(12.0, 14.0));
    ui.scope_builder(UiBuilder::new().max_rect(actions_rect), |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), add_actions);
    });
}

fn status_color(status: &ProcessStatus, ctx: &Context) -> Color32 {
    match status {
        ProcessStatus::Running => RUNNING,
        ProcessStatus::Stopped => STOPPED,
        ProcessStatus::Starting | ProcessStatus::Stopping => pulse_color(ctx, WARNING),
        ProcessStatus::Error(_) => DANGER,
    }
}

fn pulse_color(ctx: &Context, base: Color32) -> Color32 {
    let wave = ((ctx.input(|input| input.time) * 3.0).sin() * 0.18 + 0.82) as f32;
    Color32::from_rgba_premultiplied(
        (base.r() as f32 * wave) as u8,
        (base.g() as f32 * wave) as u8,
        (base.b() as f32 * wave) as u8,
        255,
    )
}

struct LogLineStyle {
    color: Color32,
    hover: &'static str,
}

fn classify_log_line(line: &str) -> LogLineStyle {
    let trimmed = line.trim();
    let content = trimmed
        .strip_prefix("[stderr] ")
        .or_else(|| trimmed.strip_prefix("[stderr]"))
        .unwrap_or(trimmed);
    let lower = content.to_ascii_lowercase();
    let is_system = trimmed.starts_with('[') && trimmed.ends_with(']');

    if is_system {
        return LogLineStyle {
            color: Color32::from_rgb(126, 147, 172),
            hover: "System event",
        };
    }

    if contains_error_indicator(content) {
        return LogLineStyle {
            color: DANGER,
            hover: "Likely error output",
        };
    }

    if lower.contains("warn") {
        return LogLineStyle {
            color: WARNING,
            hover: "Warning output",
        };
    }

    if trimmed.starts_with("[stderr]") {
        return LogLineStyle {
            color: TEXT_SOFT,
            hover: "stderr output",
        };
    }

    LogLineStyle {
        color: TEXT_SOFT,
        hover: "stdout output",
    }
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

fn parse_log_rotation_count(value: &str) -> Result<usize, String> {
    match value.trim().parse::<usize>() {
        Ok(count) if count > 0 => Ok(count),
        _ => Err("Logs to keep must be a whole number greater than 0.".to_string()),
    }
}

fn parse_startup_delay_seconds(value: &str) -> Result<u64, String> {
    match value.trim().parse::<u64>() {
        Ok(seconds) => Ok(seconds),
        Err(_) => Err("Startup delay must be a whole number of seconds.".to_string()),
    }
}

fn build_scheduled_run(form: &ProcessDraft) -> Result<ScheduledRun, String> {
    let mut scheduled_run = form.scheduled_run.clone();
    scheduled_run.hour = parse_hour(&form.scheduled_run_hour)?;
    scheduled_run.interval_hours = parse_interval_hours(&form.scheduled_run_interval_hours)?;
    normalize_weekdays(&mut scheduled_run.weekdays);
    Ok(scheduled_run)
}

fn normalize_restart_schedule(mut schedule: ManagedRestartSchedule) -> ManagedRestartSchedule {
    normalize_weekly_hours(&mut schedule.hours);
    schedule
}

fn parse_hour(value: &str) -> Result<u8, String> {
    match value.trim().parse::<u8>() {
        Ok(hour) if hour <= 23 => Ok(hour),
        _ => Err("Scheduled run hour must be a whole number from 0 to 23.".to_string()),
    }
}

fn parse_interval_hours(value: &str) -> Result<u8, String> {
    match value.trim().parse::<u8>() {
        Ok(hours) if (1..=24).contains(&hours) => Ok(hours),
        _ => Err("Scheduled run interval must be a whole number from 1 to 24.".to_string()),
    }
}

fn normalize_weekly_hours(hours: &mut Vec<bool>) {
    if hours.len() < WEEKLY_HOUR_COUNT {
        hours.resize(WEEKLY_HOUR_COUNT, false);
    } else if hours.len() > WEEKLY_HOUR_COUNT {
        hours.truncate(WEEKLY_HOUR_COUNT);
    }
}

fn normalize_weekdays(days: &mut Vec<bool>) {
    if days.len() < 7 {
        days.resize(7, false);
    } else if days.len() > 7 {
        days.truncate(7);
    }
}

fn set_weekly_hours(hours: &mut Vec<bool>, value: bool) {
    normalize_weekly_hours(hours);
    for enabled in hours.iter_mut() {
        *enabled = value;
    }
}

fn set_weekday_hours(
    hours: &mut Vec<bool>,
    days: impl Iterator<Item = usize>,
    hour_range: impl Iterator<Item = usize> + Clone,
) {
    set_weekly_hours(hours, false);
    for day in days {
        for hour in hour_range.clone() {
            if let Some(index) = weekly_hour_index(day, hour as u32) {
                hours[index] = true;
            }
        }
    }
}

fn set_night_hours(hours: &mut Vec<bool>) {
    set_weekly_hours(hours, false);
    for day in 0..7 {
        for hour in 0..24 {
            if (0..6).contains(&hour) || hour >= 20 {
                if let Some(index) = weekly_hour_index(day, hour as u32) {
                    hours[index] = true;
                }
            }
        }
    }
}

fn restart_schedule_summary(schedule: &ManagedRestartSchedule) -> String {
    if !schedule.enabled {
        return "No active-hours gate. Managed restart can run anytime.".to_string();
    }

    let active_hours = schedule.hours.iter().filter(|enabled| **enabled).count();
    let stop_note = if schedule.stop_when_inactive {
        " Stops when inactive."
    } else {
        " Lets active runs finish."
    };
    format!("{active_hours} active hours per week.{stop_note}")
}

fn scheduled_run_summary(form: &ProcessDraft) -> String {
    match form.scheduled_run.mode {
        ScheduledRunMode::Hourly => "Starts at the top of every hour.".to_string(),
        ScheduledRunMode::EveryNHours => {
            format!(
                "Starts every {} hour(s).",
                form.scheduled_run_interval_hours.trim()
            )
        }
        ScheduledRunMode::Daily => {
            format!("Starts daily at {:0>2}:00.", form.scheduled_run_hour.trim())
        }
        ScheduledRunMode::SelectedWeekdays => {
            let selected = form
                .scheduled_run
                .weekdays
                .iter()
                .enumerate()
                .filter_map(|(index, enabled)| enabled.then_some(day_label(index)))
                .collect::<Vec<_>>()
                .join(", ");
            let selected = if selected.is_empty() {
                "no days".to_string()
            } else {
                selected
            };
            format!(
                "Starts on {selected} at {:0>2}:00.",
                form.scheduled_run_hour.trim()
            )
        }
    }
}

fn day_label(index: usize) -> &'static str {
    match index {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        _ => "Sun",
    }
}

fn parse_process_error_flash_seconds(value: &str) -> Result<u64, String> {
    match value.trim().parse::<u64>() {
        Ok(seconds) => Ok(seconds),
        Err(_) => Err(
            "Error flash duration must be a whole number of seconds (0 disables it).".to_string(),
        ),
    }
}

fn normalize_log_directory(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    }
}

fn blend_color(base: Color32, overlay: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let inverse = 1.0 - amount;
    Color32::from_rgba_unmultiplied(
        ((base.r() as f32 * inverse) + (overlay.r() as f32 * amount)).round() as u8,
        ((base.g() as f32 * inverse) + (overlay.g() as f32 * amount)).round() as u8,
        ((base.b() as f32 * inverse) + (overlay.b() as f32 * amount)).round() as u8,
        ((base.a() as f32 * inverse) + (overlay.a() as f32 * amount)).round() as u8,
    )
}

fn append_diagnostics_line(log_path: &mut Option<PathBuf>, line: &str) {
    let path = log_path.get_or_insert_with(|| {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        exe_dir.join("process-manager-diagnostics.log")
    });

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(windows)]
fn sample_windows_title_bar_color(window_title: &str) -> Option<Color32> {
    use std::collections::HashMap;

    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let hwnd = find_window_by_title(window_title)?;

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
    if ok == 0 {
        return None;
    }

    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width < 320 || height < 80 {
        return None;
    }

    let sample_y = rect.top + 12;
    let sample_start = rect.left + 150;
    let sample_end = rect.right - 190;
    if sample_end <= sample_start {
        return None;
    }

    let hdc = unsafe { GetDC(std::ptr::null_mut()) };
    if hdc.is_null() {
        return None;
    }

    let mut counts = HashMap::<u32, usize>::new();
    let step = 12usize;

    for x in (sample_start..sample_end).step_by(step) {
        let pixel = unsafe { GetPixel(hdc, x, sample_y) };
        if pixel == u32::MAX {
            continue;
        }

        let r = (pixel & 0x0000_00FF) as u8;
        let g = ((pixel & 0x0000_FF00) >> 8) as u8;
        let b = ((pixel & 0x00FF_0000) >> 16) as u8;
        let packed = ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
        *counts.entry(packed).or_insert(0) += 1;
    }

    unsafe {
        ReleaseDC(std::ptr::null_mut(), hdc);
    }

    let packed = counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(packed, _)| packed)?;

    let r = ((packed >> 16) & 0xFF) as u8;
    let g = ((packed >> 8) & 0xFF) as u8;
    let b = (packed & 0xFF) as u8;

    Some(Color32::from_rgb(r, g, b))
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
