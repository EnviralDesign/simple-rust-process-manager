//! Process settings, edited in place: the Settings tab of a process and the New process page share
//! one form. Changes collect in a draft; a bar at the bottom saves or discards them.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use eframe::egui::{
    self, pos2, vec2, Align, Context, CornerRadius, Layout, Popup, Rect, RectAlign, RichText,
    ScrollArea, Sense, Stroke, StrokeKind, Ui,
};

use crate::config::{
    weekly_hour_index, ManagedRestartSchedule, ProcessConfig, ProcessType, ScheduledRun,
    ScheduledRunMode, DEFAULT_LOG_ROTATION_COUNT, DEFAULT_STARTUP_DELAY_SECONDS, WEEKLY_HOUR_COUNT,
};
use crate::process_manager::{ActivityKind, ProcessStatus};

use super::format;
use super::icons::Icon;
use super::theme::{Tier, *};
use super::widgets::{self, Btn};
use super::{Nav, ProcessManagerApp, ProcessTab, View};

/// The width of a form field column, for tests that draw a path editor alone.
#[cfg(test)]
const FORM_WIDTH: f32 = 398.0;

#[derive(Clone, PartialEq)]
pub(super) struct ProcessDraft {
    name: String,
    command: String,
    working_directory: String,
    process_type: ProcessType,
    auto_start: bool,
    startup_delay_seconds: String,
    auto_restart: bool,
    restart_schedule: ManagedRestartSchedule,
    scheduled_run: ScheduledRun,
    scheduled_run_interval_hours: String,
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
            scheduled_run_interval_hours: ScheduledRun::default().interval_hours.to_string(),
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
            scheduled_run_interval_hours: process.scheduled_run.interval_hours.to_string(),
            respond_to_start_all: process.respond_to_start_all,
            respond_to_stop_all: process.respond_to_stop_all,
            respond_to_restart_all: process.respond_to_restart_all,
            log_to_disk: process.log_to_disk,
            log_rotation_count: process.log_rotation_count.to_string(),
        }
    }

    /// The config this draft describes, or the fields that are wrong, keyed by field.
    fn build(&self, id: Option<&str>) -> Result<ProcessConfig, HashMap<&'static str, String>> {
        let mut errors = HashMap::new();
        if self.name.trim().is_empty() {
            errors.insert("name", "Give it a name.".to_string());
        }
        if self.command.trim().is_empty() {
            errors.insert(
                "command",
                match self.process_type {
                    ProcessType::Process => "Give it a command to run.".to_string(),
                    ProcessType::Docker => "Give it the container's name.".to_string(),
                },
            );
        }
        let delay = self
            .startup_delay_seconds
            .trim()
            .parse::<u64>()
            .map_err(|_| {
                errors.insert("delay", "A whole number of seconds, 0 or more.".to_string());
            });
        let keep = match self.log_rotation_count.trim().parse::<usize>() {
            Ok(count) if count > 0 => Ok(count),
            _ => {
                errors.insert("keep", "A whole number, 1 or more.".to_string());
                Err(())
            }
        };
        let interval = match self.scheduled_run_interval_hours.trim().parse::<u8>() {
            Ok(hours) if (1..=24).contains(&hours) => Ok(hours),
            _ if self.scheduled_run.enabled
                && self.scheduled_run.mode == ScheduledRunMode::EveryNHours =>
            {
                errors.insert("interval", "From 1 to 24 hours.".to_string());
                Err(())
            }
            _ => Ok(self.scheduled_run.interval_hours.clamp(1, 24)),
        };
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut process = ProcessConfig::new(
            self.name.trim().to_string(),
            self.command.trim().to_string(),
            self.working_directory.trim().to_string(),
            self.process_type.clone(),
        );
        if let Some(id) = id {
            process.id = id.to_string();
        }
        let mut restart_schedule = self.restart_schedule.clone();
        restart_schedule.hours.resize(WEEKLY_HOUR_COUNT, false);
        let mut scheduled_run = self.scheduled_run.clone();
        scheduled_run.interval_hours = interval.unwrap_or(1);
        scheduled_run.weekdays.resize(7, false);
        process.auto_start = self.auto_start;
        process.startup_delay_seconds = delay.unwrap_or(0);
        process.auto_restart = self.auto_restart;
        process.restart_schedule = restart_schedule;
        process.scheduled_run = scheduled_run;
        process.respond_to_start_all = self.respond_to_start_all;
        process.respond_to_stop_all = self.respond_to_stop_all;
        process.respond_to_restart_all = self.respond_to_restart_all;
        process.log_to_disk = self.log_to_disk;
        process.log_rotation_count = keep.unwrap_or(DEFAULT_LOG_ROTATION_COUNT);
        Ok(process)
    }

    /// Whether a change needs a restart to take effect on a running process.
    fn launch_differs(&self, other: &Self) -> bool {
        self.command.trim() != other.command.trim()
            || self.working_directory.trim() != other.working_directory.trim()
            || self.process_type != other.process_type
            || self.log_to_disk != other.log_to_disk
    }
}

pub(super) struct Editor {
    /// `None` while the process does not exist yet.
    pub process_id: Option<String>,
    draft: ProcessDraft,
    base: ProcessDraft,
    cwd_suggestion: RelativePathSuggestion,
    errors: HashMap<&'static str, String>,
}

impl Editor {
    pub fn new_process() -> Self {
        let draft = ProcessDraft {
            name: String::new(),
            ..ProcessDraft::default()
        };
        Self {
            process_id: None,
            base: draft.clone(),
            draft,
            cwd_suggestion: RelativePathSuggestion::default(),
            errors: HashMap::new(),
        }
    }

    pub fn for_process(process: &ProcessConfig) -> Self {
        let draft = ProcessDraft::from_process(process);
        Self {
            process_id: Some(process.id.clone()),
            base: draft.clone(),
            draft,
            cwd_suggestion: RelativePathSuggestion::default(),
            errors: HashMap::new(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.process_id.is_none() || self.draft != self.base
    }

    pub fn name(&self) -> String {
        if self.draft.name.trim().is_empty() {
            "the new process".to_string()
        } else {
            self.draft.name.trim().to_string()
        }
    }
}

enum BarAction {
    Save { restart: bool },
    Discard,
}

impl ProcessManagerApp {
    pub(super) fn draw_process_settings(
        &mut self,
        ui: &mut Ui,
        process: &ProcessConfig,
        tier: Tier,
        gutter: f32,
    ) {
        self.ensure_editor();
        // Another draft is waiting on the unsaved-changes question; draw nothing under it.
        if self
            .editor
            .as_ref()
            .is_some_and(|e| e.process_id.as_deref() != Some(process.id.as_str()))
        {
            return;
        }
        let Some(mut editor) = self.editor.take() else {
            return;
        };
        let running = matches!(
            self.status_of(&process.id),
            ProcessStatus::Running | ProcessStatus::Starting
        );
        let log_folder = self.process_log_folder(process);
        let action = draw_form_page(
            ui,
            &mut editor,
            tier,
            gutter,
            running,
            Some(log_folder),
            false,
        );
        self.editor = Some(editor);
        match action {
            Some(BarAction::Save { restart }) => self.save_editor(restart),
            Some(BarAction::Discard) => {
                if let Some(process) = self.process_config(&process.id) {
                    self.editor = Some(Editor::for_process(&process));
                }
                self.set_banner("Discarded the changes");
            }
            None => {}
        }
    }

    pub(super) fn draw_new_process(&mut self, ui: &mut Ui, tier: Tier) {
        if self.editor.as_ref().is_none_or(|e| e.process_id.is_some()) {
            self.editor = Some(Editor::new_process());
        }
        let gutter = if tier <= Tier::Compact { 14.0 } else { GUTTER };
        let rect = ui.max_rect();
        let mut page = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        page.spacing_mut().item_spacing = vec2(0.0, 0.0);
        page.add_space(18.0);
        page.horizontal(|ui| {
            ui.add_space(gutter);
            ui.label(RichText::new("New process").font(bold(24.0)).color(TEXT));
        });
        page.add_space(6.0);
        page.horizontal(|ui| {
            ui.add_space(gutter);
            ui.label(
                RichText::new("Give it a name and a command. Everything else can wait.")
                    .font(body(13.0))
                    .color(MUTED),
            );
        });
        page.add_space(4.0);
        let mut editor = self.editor.take().unwrap_or_else(Editor::new_process);
        let action = draw_form_page(&mut page, &mut editor, tier, gutter, false, None, true);
        self.editor = Some(editor);
        match action {
            Some(BarAction::Save { .. }) => self.save_editor(false),
            Some(BarAction::Discard) => {
                self.editor = None;
                let back = self
                    .selected_process
                    .clone()
                    .map(Nav::Process)
                    .unwrap_or(Nav::Overview);
                self.go(back);
            }
            None => {}
        }
    }

    pub(super) fn save_editor(&mut self, restart: bool) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let built = editor.draft.build(editor.process_id.as_deref());
        let process = match built {
            Ok(process) => process,
            Err(errors) => {
                editor.errors = errors;
                self.set_banner("Some fields need a fix before saving");
                return;
            }
        };
        editor.errors.clear();
        match editor.process_id.clone() {
            Some(id) => {
                let launch_changed = editor.draft.launch_differs(&editor.base);
                let kind_changed = editor.draft.process_type != editor.base.process_type;
                // Stopping goes by the kind it was started as, so a running entry whose kind
                // changes stops first; otherwise it would be orphaned.
                let stopped_for_kind = kind_changed && self.is_live(&id);
                if stopped_for_kind {
                    self.manager.stop_process_by(&id, "Kind changed");
                }
                self.config.update_process(&id, process.clone());
                self.persist_config();
                let _ = self.manager.update_process_config(process.clone());
                self.manager
                    .record_activity(&id, ActivityKind::Settings, "Settings saved", "You");
                self.editor = Some(Editor::for_process(&process));
                let running = self.is_live(&id);
                if stopped_for_kind {
                    self.set_banner(format!(
                        "Saved {}. It was stopped to change its kind; start it again.",
                        process.name
                    ));
                } else if restart {
                    self.restart_process(&id);
                    self.set_banner(format!("Saved {} and restarted it", process.name));
                } else if running && launch_changed {
                    self.set_banner(format!(
                        "Saved {}. The new command and folder apply at its next start.",
                        process.name
                    ));
                } else {
                    self.set_banner(format!("Saved {}", process.name));
                }
            }
            None => {
                self.manager.add_process(process.clone());
                self.config.add_process(process.clone());
                self.persist_config();
                self.last_process_error_versions = self.manager.error_versions();
                self.editor = None;
                self.go(Nav::Process(process.id.clone()));
                self.tab = ProcessTab::Logs;
                self.set_banner(format!(
                    "Added {}. Start it when you're ready.",
                    process.name
                ));
            }
        }
        self.refresh_runtime_snapshot(true);
        let _ = View::Process;
    }
}

/// The form in a scroll area, with the save bar pinned under it when there is something to save.
fn draw_form_page(
    ui: &mut Ui,
    editor: &mut Editor,
    tier: Tier,
    gutter: f32,
    running: bool,
    log_folder: Option<PathBuf>,
    is_new: bool,
) -> Option<BarAction> {
    let outer = ui.available_rect_before_wrap();
    let show_bar = editor.is_dirty();
    let bar_h = if show_bar { 64.0 } else { 0.0 };
    let scroll_rect = Rect::from_min_max(outer.min, pegui(outer.right(), outer.bottom() - bar_h));
    let mut scroll_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(scroll_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    ScrollArea::vertical()
        .id_salt(("settings_form", editor.process_id.clone()))
        .auto_shrink([false, false])
        .show(&mut scroll_ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: gutter as i8,
                    right: gutter as i8,
                    top: 16,
                    bottom: 24,
                })
                .show(ui, |ui| draw_form(ui, editor, tier, log_folder.as_deref()));
        });
    ui.allocate_rect(outer, Sense::hover());

    if !show_bar {
        return None;
    }
    let bar = Rect::from_min_max(
        pos2(outer.left() + gutter, outer.bottom() - bar_h + 8.0),
        pos2(outer.right() - gutter, outer.bottom() - 12.0),
    );
    let painter = ui.painter();
    painter.add(
        egui::epaint::Shadow {
            offset: [0, 10],
            blur: 32,
            spread: 0,
            color: egui::Color32::from_black_alpha(140),
        }
        .as_shape(bar, CornerRadius::same(RADIUS_CARD)),
    );
    painter.rect(
        bar,
        CornerRadius::same(RADIUS_CARD),
        PANEL2,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    let mut bar_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(14.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    bar_ui.spacing_mut().item_spacing.x = 8.0;
    let (dot, _) = bar_ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
    bar_ui.painter().circle_filled(dot.center(), 4.0, AMBER);
    let title = if is_new {
        "New process".to_string()
    } else {
        format!("Unsaved changes to {}", editor.base.name)
    };
    let launch_changed = !is_new && editor.draft.launch_differs(&editor.base);
    let note = if !editor.errors.is_empty() {
        "Fix the fields marked in red".to_string()
    } else if is_new {
        "Saved to processes.json; it starts when you start it".to_string()
    } else if running && launch_changed {
        "The command applies at the next start".to_string()
    } else {
        "Saved to processes.json".to_string()
    };
    let mut action = None;
    let mut right = bar_ui.new_child(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(10.0, 0.0)))
            .layout(Layout::right_to_left(Align::Center)),
    );
    right.spacing_mut().item_spacing.x = 8.0;
    let primary_label = if is_new { "Add process" } else { "Save" };
    if Btn::new(primary_label).primary().show(&mut right).clicked() {
        action = Some(BarAction::Save { restart: false });
    }
    if running
        && launch_changed
        && tier > Tier::Tiny
        && Btn::new("Save and restart").show(&mut right).clicked()
    {
        action = Some(BarAction::Save { restart: true });
    }
    if Btn::new(if is_new { "Cancel" } else { "Discard" })
        .show(&mut right)
        .clicked()
    {
        action = Some(BarAction::Discard);
    }
    let room = right.min_rect().left() - bar_ui.cursor().left() - 12.0;
    let title_g = widgets::galley_truncated(&bar_ui, &title, bold(13.0), TEXT, room.max(0.0));
    let title_w = title_g.size().x;
    let (r, _) = bar_ui.allocate_exact_size(vec2(title_w, 20.0), Sense::hover());
    widgets::paint_galley_left(
        bar_ui.painter(),
        pos2(r.left(), r.center().y),
        title_g,
        TEXT,
    );
    if tier >= Tier::Full && room - title_w > 120.0 {
        let color = if editor.errors.is_empty() { FAINT } else { RED };
        let g = widgets::galley_truncated(&bar_ui, &note, body(12.0), color, room - title_w - 12.0);
        let (r, _) = bar_ui.allocate_exact_size(vec2(g.size().x + 4.0, 20.0), Sense::hover());
        widgets::paint_galley_left(
            bar_ui.painter(),
            pos2(r.left() + 4.0, r.center().y),
            g,
            color,
        );
    }
    action
}

fn pegui(x: f32, y: f32) -> egui::Pos2 {
    pos2(x, y)
}

fn draw_form(ui: &mut Ui, editor: &mut Editor, tier: Tier, log_folder: Option<&Path>) {
    let width = ui.available_width();
    let two_columns = width >= 880.0 && tier >= Tier::Full;
    ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
    if two_columns {
        let column_w = (width - 14.0) / 2.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(vec2(column_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(column_w);
                general_card(ui, editor);
                stack_controls_card(ui, editor);
                logs_card(ui, editor, log_folder);
            });
            ui.allocate_ui_with_layout(vec2(column_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(column_w);
                start_restart_card(ui, editor);
                schedule_card(ui, editor);
            });
        });
    } else {
        general_card(ui, editor);
        start_restart_card(ui, editor);
        schedule_card(ui, editor);
        stack_controls_card(ui, editor);
        logs_card(ui, editor, log_folder);
    }
}

fn field_label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(bold(12.0)).color(MUTED));
    ui.add_space(-8.0);
}

fn field_hint(ui: &mut Ui, text: &str) {
    ui.add_space(-8.0);
    ui.label(RichText::new(text).font(body(11.5)).color(FAINT));
}

fn field_error(ui: &mut Ui, errors: &HashMap<&'static str, String>, key: &str) {
    if let Some(message) = errors.get(key) {
        ui.add_space(-8.0);
        ui.label(RichText::new(message).font(body(11.5)).color(RED));
    }
}

/// A row with a title, a hint and a switch. Returns whether the switch changed.
pub(super) fn switch_row(ui: &mut Ui, title: &str, hint: &str, on: &mut bool, first: bool) -> bool {
    let width = ui.available_width();
    if !first {
        let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
        ui.painter().hline(
            line.x_range(),
            line.center().y,
            Stroke::new(1.0_f32, ROW_LINE),
        );
    }
    let mut changed = false;
    ui.allocate_ui_with_layout(
        vec2(width, 0.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(width);
            let text_w = width - 36.0 - 14.0;
            ui.allocate_ui_with_layout(vec2(text_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(title).font(bold(13.0)).color(TEXT));
                if !hint.is_empty() {
                    ui.add(
                        egui::Label::new(RichText::new(hint).font(body(11.5)).color(FAINT)).wrap(),
                    );
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                changed = widgets::switch(ui, on).changed();
            });
        },
    );
    changed
}

fn general_card(ui: &mut Ui, editor: &mut Editor) {
    let errors = editor.errors.clone();
    let draft = &mut editor.draft;
    widgets::card(ui, "General", |ui| {
        let width = ui.available_width();
        field_label(ui, "Name");
        widgets::text_field(
            ui,
            "name",
            &mut draft.name,
            "Frontend dev server",
            width,
            false,
        );
        field_error(ui, &errors, "name");

        field_label(ui, "Kind");
        let selected = if draft.process_type == ProcessType::Docker {
            1
        } else {
            0
        };
        if let Some(index) = widgets::segmented(ui, &["Process", "Docker container"], selected) {
            draft.process_type = if index == 1 {
                ProcessType::Docker
            } else {
                ProcessType::Process
            };
        }

        let docker = draft.process_type == ProcessType::Docker;
        field_label(ui, if docker { "Container name" } else { "Command" });
        widgets::text_field(
            ui,
            "command",
            &mut draft.command,
            if docker {
                "postgres-dev"
            } else {
                "npm run dev"
            },
            width,
            true,
        );
        field_error(ui, &errors, "command");
        field_hint(
            ui,
            if docker {
                "Started with docker start -a. Containers keep running when Process Manager closes."
            } else {
                "Runs without a console window. Stop ends its whole process tree."
            },
        );

        if !docker {
            field_label(ui, "Working folder");
            path_edit(
                ui,
                "cwd",
                &mut draft.working_directory,
                &mut editor.cwd_suggestion,
                "C:/projects/my-app",
                width,
            );
            field_hint(
                ui,
                "Empty means the executable's folder. A relative path moves with the stack.",
            );
        }
    });
}

fn start_restart_card(ui: &mut Ui, editor: &mut Editor) {
    let errors = editor.errors.clone();
    let draft = &mut editor.draft;
    widgets::card(ui, "Start and restart", |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        switch_row(
            ui,
            "Auto-start",
            "Start this entry when Process Manager opens.",
            &mut draft.auto_start,
            true,
        );
        let width = ui.available_width();
        let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
        ui.painter().hline(
            line.x_range(),
            line.center().y,
            Stroke::new(1.0_f32, ROW_LINE),
        );
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new("Wait").font(bold(13.0)).color(TEXT));
            widgets::text_field(
                ui,
                "delay",
                &mut draft.startup_delay_seconds,
                "0",
                64.0,
                true,
            );
            ui.label(
                RichText::new("seconds before every start")
                    .font(body(12.0))
                    .color(FAINT),
            );
        });
        field_error(ui, &errors, "delay");
        switch_row(
            ui,
            "Managed restart",
            "Start it again whenever it exits, even with code 0.",
            &mut draft.auto_restart,
            false,
        );
        if draft.auto_restart {
            sub_panel(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 10.0;
                switch_row(
                    ui,
                    "Only during active hours",
                    "Outside these hours an exit is left alone.",
                    &mut draft.restart_schedule.enabled,
                    true,
                );
                if draft.restart_schedule.enabled {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Active hours").font(bold(12.0)).color(MUTED));
                        ui.label(
                            RichText::new(format::hours_text(&draft.restart_schedule.hours))
                                .font(body(11.5))
                                .color(FAINT),
                        );
                    });
                    hours_grid(ui, &mut draft.restart_schedule.hours);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        ui.label(RichText::new("Presets").font(body(11.5)).color(FAINT));
                        let hours = &mut draft.restart_schedule.hours;
                        if Btn::new("Always").xsmall().show(ui).clicked() {
                            set_hours(hours, |_, _| true);
                        }
                        if Btn::new("Weekdays 08–20").xsmall().show(ui).clicked() {
                            set_hours(hours, |d, h| d < 5 && (8..20).contains(&h));
                        }
                        if Btn::new("Business hours").xsmall().show(ui).clicked() {
                            set_hours(hours, |d, h| d < 5 && (9..17).contains(&h));
                        }
                        if Btn::new("Nights").xsmall().show(ui).clicked() {
                            set_hours(hours, |_, h| !(6..20).contains(&h));
                        }
                        if Btn::new("Clear").xsmall().show(ui).clicked() {
                            set_hours(hours, |_, _| false);
                        }
                    });
                    switch_row(
                        ui,
                        "Stop it when active hours end",
                        "Otherwise it keeps running and is only no longer restarted.",
                        &mut draft.restart_schedule.stop_when_inactive,
                        false,
                    );
                }
            });
        }
    });
}

fn schedule_card(ui: &mut Ui, editor: &mut Editor) {
    let errors = editor.errors.clone();
    let draft = &mut editor.draft;
    widgets::card(ui, "Schedule", |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        switch_row(
            ui,
            "Scheduled start",
            "Start this entry on a schedule while it is stopped.",
            &mut draft.scheduled_run.enabled,
            true,
        );
        if !draft.scheduled_run.enabled {
            return;
        }
        let interval_text = &mut draft.scheduled_run_interval_hours;
        let run = &mut draft.scheduled_run;
        sub_panel(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 10.0;
            let modes = [
                ScheduledRunMode::Hourly,
                ScheduledRunMode::EveryNHours,
                ScheduledRunMode::Daily,
                ScheduledRunMode::SelectedWeekdays,
            ];
            let selected = modes.iter().position(|m| *m == run.mode).unwrap_or(2);
            if let Some(index) = widgets::segmented(
                ui,
                &["Hourly", "Every N hours", "Daily", "Chosen days"],
                selected,
            ) {
                run.mode = modes[index].clone();
            }
            match run.mode {
                ScheduledRunMode::EveryNHours => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(RichText::new("Every").font(bold(13.0)).color(TEXT));
                        widgets::text_field(ui, "interval", interval_text, "4", 56.0, true);
                        ui.label(
                            RichText::new("hours, on the hour")
                                .font(body(12.0))
                                .color(FAINT),
                        );
                    });
                    field_error(ui, &errors, "interval");
                    if let Ok(hours) = interval_text.trim().parse::<u8>() {
                        run.interval_hours = hours.clamp(1, 24);
                    }
                }
                ScheduledRunMode::SelectedWeekdays => {
                    run.weekdays.resize(7, false);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for day in 0..7 {
                            if widgets::chip(
                                ui,
                                format::day_label(day),
                                None,
                                run.weekdays[day],
                                None,
                            )
                            .clicked()
                            {
                                run.weekdays[day] = !run.weekdays[day];
                            }
                        }
                    });
                    hour_picker(ui, run);
                }
                ScheduledRunMode::Daily => hour_picker(ui, run),
                ScheduledRunMode::Hourly => {}
            }
            let run = &*run;
            let next = format::next_scheduled_run(run, chrono::Local::now());
            ui.label(
                RichText::new(format!(
                    "{}, only while it is stopped · next {next}",
                    format::schedule_text(run)
                ))
                .font(body(11.5))
                .color(FAINT),
            );
        });
    });
}

fn hour_picker(ui: &mut Ui, run: &mut ScheduledRun) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new("At").font(bold(13.0)).color(TEXT));
        let label = format!("{:02}:00", run.hour.min(23));
        let button = Btn::new(&label).with_icon(Icon::CaretDown).small().show(ui);
        Popup::menu(&button)
            .frame(widgets::popup_frame())
            .align(RectAlign::BOTTOM_START)
            .gap(4.0)
            .show(|ui| {
                ui.set_min_width(110.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    for hour in 0..24u8 {
                        let text = format!("{hour:02}:00");
                        if widgets::menu_item(ui, &text, None, false, true).clicked() {
                            run.hour = hour;
                            ui.close();
                        }
                    }
                });
            });
        ui.label(RichText::new("local time").font(body(12.0)).color(FAINT));
    });
}

fn stack_controls_card(ui: &mut Ui, editor: &mut Editor) {
    let draft = &mut editor.draft;
    widgets::card(ui, "Stack controls", |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        switch_row(
            ui,
            "Answers Start all",
            "",
            &mut draft.respond_to_start_all,
            true,
        );
        switch_row(
            ui,
            "Answers Stop all",
            "",
            &mut draft.respond_to_stop_all,
            false,
        );
        switch_row(
            ui,
            "Answers Restart all",
            "Its own buttons, auto-start and managed restart work either way. Groups follow the same three.",
            &mut draft.respond_to_restart_all,
            false,
        );
    });
}

fn logs_card(ui: &mut Ui, editor: &mut Editor, log_folder: Option<&Path>) {
    let errors = editor.errors.clone();
    let draft = &mut editor.draft;
    widgets::card(ui, "Logs", |ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        let hint = match log_folder {
            Some(folder) => format!("A full log per session in {}.", folder.display()),
            None => "A full log per session in the stack's log folder.".to_string(),
        };
        switch_row(
            ui,
            "Write logs to disk",
            &hint,
            &mut draft.log_to_disk,
            true,
        );
        if draft.log_to_disk {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("Keep").font(bold(13.0)).color(TEXT));
                widgets::text_field(ui, "keep", &mut draft.log_rotation_count, "10", 64.0, true);
                ui.label(
                    RichText::new("session logs, oldest removed first")
                        .font(body(12.0))
                        .color(FAINT),
                );
            });
            field_error(ui, &errors, "keep");
        }
        ui.label(
            RichText::new(
                "The app keeps the last 1,000 lines of every process in memory, across restarts.",
            )
            .font(body(11.5))
            .color(FAINT),
        );
    });
}

fn sub_panel(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(BG)
        .stroke(Stroke::new(1.0_f32, ROW_LINE))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        });
}

fn set_hours(hours: &mut Vec<bool>, on: impl Fn(usize, usize) -> bool) {
    hours.resize(WEEKLY_HOUR_COUNT, false);
    for day in 0..7 {
        for hour in 0..24 {
            hours[day * 24 + hour] = on(day, hour);
        }
    }
}

/// The weekly active-hours grid: a click flips an hour, a drag paints the value the first
/// cell took, and a day's label flips the whole day.
fn hours_grid(ui: &mut Ui, hours: &mut Vec<bool>) {
    hours.resize(WEEKLY_HOUR_COUNT, false);
    let width = ui.available_width();
    let label_w = 38.0;
    let gap = 2.0;
    let cell_w = ((width - label_w - gap * 23.0) / 24.0).max(6.0);
    let cell_h = 16.0;
    let head_h = 14.0;
    let height = head_h + 4.0 + 7.0 * cell_h + 6.0 * gap;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    let painter = ui.painter().clone();
    let grid_left = rect.left() + label_w;
    let cell_rect = |day: usize, hour: usize| {
        Rect::from_min_size(
            pos2(
                grid_left + hour as f32 * (cell_w + gap),
                rect.top() + head_h + 4.0 + day as f32 * (cell_h + gap),
            ),
            vec2(cell_w, cell_h),
        )
    };
    for hour in (0..24).step_by(3) {
        painter.text(
            pos2(grid_left + hour as f32 * (cell_w + gap), rect.top()),
            egui::Align2::LEFT_TOP,
            format!("{hour:02}"),
            mono(9.5),
            FAINT,
        );
    }
    let hover = response.hover_pos();
    let paint_id = response.id.with("paint");
    let cell_at = |pos: egui::Pos2| -> Option<(usize, usize)> {
        for day in 0..7 {
            for hour in 0..24 {
                if cell_rect(day, hour).expand(gap * 0.5).contains(pos) {
                    return Some((day, hour));
                }
            }
        }
        None
    };
    if response.drag_started() || response.clicked() {
        if let Some((day, hour)) = response.interact_pointer_pos().and_then(cell_at) {
            let index = weekly_hour_index(day, hour as u32).unwrap_or(0);
            let value = !hours[index];
            ui.data_mut(|d| d.insert_temp(paint_id, value));
            hours[index] = value;
        } else if let Some(pos) = response.interact_pointer_pos() {
            // A day label.
            if pos.x < grid_left {
                for day in 0..7 {
                    let row = cell_rect(day, 0);
                    if pos.y >= row.top() && pos.y <= row.bottom() {
                        let all = (0..24).all(|h| hours[day * 24 + h]);
                        for h in 0..24 {
                            hours[day * 24 + h] = !all;
                        }
                    }
                }
            }
        }
    }
    if response.dragged() {
        if let (Some(value), Some((day, hour))) = (
            ui.data(|d| d.get_temp::<bool>(paint_id)),
            response.interact_pointer_pos().and_then(cell_at),
        ) {
            if let Some(index) = weekly_hour_index(day, hour as u32) {
                hours[index] = value;
            }
        }
    }
    for day in 0..7 {
        let row = cell_rect(day, 0);
        let label_hover =
            hover.is_some_and(|p| p.x < grid_left && p.y >= row.top() && p.y <= row.bottom());
        painter.text(
            pos2(rect.left(), row.center().y),
            egui::Align2::LEFT_CENTER,
            format::day_label(day),
            bold(11.0),
            if label_hover { TEXT } else { MUTED },
        );
        for hour in 0..24 {
            let r = cell_rect(day, hour);
            let on = hours[day * 24 + hour];
            let fill = if on {
                GREEN.gamma_multiply(0.85)
            } else {
                PANEL2
            };
            painter.rect_filled(r, CornerRadius::same(2), fill);
            if hover.is_some_and(|p| r.contains(p)) {
                painter.rect_stroke(
                    r,
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, BORDER_HI),
                    StrokeKind::Inside,
                );
            }
        }
    }
    let _ = response
        .on_hover_text("Click an hour to flip it, drag to paint, click a day to flip the day");
}

/// A folder field with Browse, and a quiet offer to switch an absolute path to one relative to
/// the executable's folder.
pub(super) fn path_edit(
    ui: &mut Ui,
    id_salt: &str,
    value: &mut String,
    suggestion: &mut RelativePathSuggestion,
    hint: &str,
    total_width: f32,
) {
    let browse_w = Btn::new("Browse…").width(ui);
    let (field, browse) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let field = widgets::text_field(
                ui,
                id_salt,
                value,
                hint,
                (total_width - browse_w - 8.0).max(80.0),
                true,
            );
            let browse = Btn::new("Browse…").show(ui);
            (field, browse)
        })
        .inner;

    let mut committed = !field.has_focus();
    if browse.clicked() {
        if let Some(path) = rfd::FileDialog::new()
            .set_directory(value.trim().trim_matches('"'))
            .pick_folder()
        {
            *value = path.display().to_string();
            committed = true;
        }
    }
    suggestion.update(value, committed, ui.ctx());

    if let Some(relative) = suggestion.relative.clone() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if Btn::new("Use relative path")
                .small()
                .show(ui)
                .on_hover_text(format!("Replace with this relative path:\n{relative}"))
                .clicked()
            {
                *value = relative.clone();
                suggestion.update(value, true, ui.ctx());
            }
            if Btn::new("Dismiss").small().ghost().show(ui).clicked() {
                suggestion.relative = None;
            }
            ui.add(
                egui::Label::new(RichText::new(&relative).font(mono(11.5)).color(MUTED)).truncate(),
            )
            .on_hover_text(&relative);
        });
    }
}

#[derive(Clone, Default)]
pub(super) struct RelativePathSuggestion {
    pub base_directory: Option<PathBuf>,
    pub(super) checked_value: Option<String>,
    pub(super) pending: Option<Arc<OnceLock<Option<String>>>>,
    pub(super) relative: Option<String>,
}

impl RelativePathSuggestion {
    fn update(&mut self, value: &str, committed: bool, ctx: &Context) {
        if self.checked_value.as_deref() != Some(value) {
            self.pending = None;
            self.relative = None;
            self.checked_value = None;
            if committed {
                self.checked_value = Some(value.to_string());
                let path = value.trim().trim_matches('"');
                if Path::new(path).is_absolute() {
                    let path = PathBuf::from(path);
                    let result = Arc::new(OnceLock::new());
                    self.pending = Some(result.clone());
                    let ctx = ctx.clone();
                    let base_directory = self.base_directory.clone();
                    std::thread::spawn(move || {
                        let relative = base_directory
                            .or_else(|| {
                                std::env::current_exe()
                                    .ok()?
                                    .parent()
                                    .map(Path::to_path_buf)
                            })
                            .and_then(|base| relative_path(&base, &path))
                            .and_then(|path| path.into_os_string().into_string().ok());
                        let _ = result.set(relative);
                        ctx.request_repaint();
                    });
                }
            }
        }

        if let Some(relative) = self.pending.as_ref().and_then(|result| result.get()) {
            self.relative = relative.clone();
            self.pending = None;
        }
    }
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    if !base.is_absolute() || !target.is_absolute() {
        return None;
    }

    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = target.components().collect();
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(a, b)| {
            a == b
                || (cfg!(windows)
                    && a.as_os_str()
                        .to_str()
                        .zip(b.as_os_str().to_str())
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b)))
        })
        .count();

    let mut relative = PathBuf::new();
    for component in &base[common..] {
        if !matches!(component, Component::Normal(_)) {
            return None;
        }
        relative.push("..");
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Some(relative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::CentralPanel;
    use std::time::{Duration, Instant};

    #[test]
    fn relative_routes_cover_children_siblings_parents_and_same_directory() {
        let base = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let parent = base.parent().unwrap();
        for (target, expected) in [
            (base.clone(), PathBuf::from(".")),
            (base.join("service files"), PathBuf::from("service files")),
            (parent.to_path_buf(), PathBuf::from("..")),
            (parent.join("service"), PathBuf::from("..").join("service")),
            (
                base.join("new").join("logs"),
                PathBuf::from("new").join("logs"),
            ),
        ] {
            assert_eq!(relative_path(&base, &target), Some(expected));
        }
        assert!(relative_path(&base, Path::new("already-relative")).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn windows_routes_respect_drives_shares_and_case() {
        for (base, target, expected) in [
            (
                r"C:\Apps\Manager",
                r"c:\apps\Services\Worker",
                Some(r"..\Services\Worker"),
            ),
            (r"C:\Apps\Manager", r"D:\Services", None),
            (r"C:\Apps\Manager", r"C:Services", None),
            (r"C:\Apps\Manager", r"\Services", None),
            (
                r"\\server\share\Manager",
                r"\\server\share\Worker",
                Some(r"..\Worker"),
            ),
            (r"\\server\share\Manager", r"\\server\other\Worker", None),
            (
                r"C:\Apps\Manager",
                r"C:\Apps\Manager\..\Worker",
                Some(r"..\Worker"),
            ),
        ] {
            assert_eq!(
                relative_path(Path::new(base), Path::new(target)),
                expected.map(PathBuf::from)
            );
        }
    }

    #[test]
    fn checks_wait_for_commit_and_ignore_results_for_old_values() {
        let ctx = Context::default();
        let base = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let value = format!("\"{}\"", base.join("service files").display());
        for (base_directory, expected) in [
            (None, PathBuf::from("service files")),
            (
                Some(base.join("config")),
                PathBuf::from("..").join("service files"),
            ),
        ] {
            let mut suggestion = RelativePathSuggestion {
                base_directory,
                ..Default::default()
            };
            suggestion.update(&value, false, &ctx);
            assert!(suggestion.pending.is_none());
            suggestion.update(&value, true, &ctx);
            let pending = suggestion.pending.clone().unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while pending.get().is_none() {
                assert!(Instant::now() < deadline, "Path check did not finish");
                std::thread::sleep(Duration::from_millis(5));
            }
            suggestion.update(&value, true, &ctx);
            assert_eq!(suggestion.relative.as_deref(), expected.to_str());

            suggestion.relative = None;
            suggestion.update(&value, true, &ctx);
            assert!(
                suggestion.relative.is_none(),
                "Dismissed suggestion returned"
            );
            assert!(suggestion.pending.is_none());

            suggestion.pending = Some(pending);
            suggestion.update("different value being typed", false, &ctx);
            assert!(suggestion.relative.is_none());
            assert!(suggestion.pending.is_none());
        }
    }

    #[test]
    fn editor_buttons_convert_only_on_click_and_dismiss_without_changing_value() {
        for button in ["Use relative path", "Dismiss"] {
            let ctx = Context::default();
            super::super::theme::apply(&ctx);
            let absolute = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("service");
            let mut value = absolute.display().to_string();
            let mut suggestion = RelativePathSuggestion {
                checked_value: Some(value.clone()),
                relative: Some("service".to_string()),
                ..Default::default()
            };
            let mut frame = |events: Vec<egui::Event>| {
                ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        CentralPanel::default().show(ctx, |ui| {
                            ui.set_width(FORM_WIDTH);
                            path_edit(ui, "test", &mut value, &mut suggestion, ".", FORM_WIDTH);
                        });
                    },
                )
            };
            let output = frame(Vec::new());
            let position = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == button => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("Suggestion button should be visible");
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ]);
            }
            let output = frame(Vec::new());
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Use relative path")));
            assert_eq!(
                value,
                if button == "Use relative path" {
                    "service".to_string()
                } else {
                    absolute.display().to_string()
                }
            );
        }
    }

    #[test]
    fn drafts_validate_each_field_and_build_the_config() {
        let mut draft = ProcessDraft::default();
        let errors = draft.build(None).unwrap_err();
        assert!(errors.contains_key("name") && errors.contains_key("command"));
        draft.name = "api".into();
        draft.command = "uv run dev".into();
        draft.startup_delay_seconds = "soon".into();
        assert!(draft.build(None).unwrap_err().contains_key("delay"));
        draft.startup_delay_seconds = "5".into();
        draft.log_rotation_count = "0".into();
        assert!(draft.build(None).unwrap_err().contains_key("keep"));
        draft.log_rotation_count = "3".into();
        let process = draft.build(Some("fixed-id")).unwrap();
        assert_eq!(
            (
                process.id.as_str(),
                process.startup_delay_seconds,
                process.log_rotation_count
            ),
            ("fixed-id", 5, 3)
        );
    }

    #[test]
    fn a_running_process_only_needs_a_restart_for_launch_changes() {
        let base = ProcessDraft {
            command: "npm run dev".into(),
            ..Default::default()
        };
        let mut changed = base.clone();
        changed.auto_restart = true;
        assert!(!changed.launch_differs(&base));
        changed.command = "npm run start".into();
        assert!(changed.launch_differs(&base));
    }
}
