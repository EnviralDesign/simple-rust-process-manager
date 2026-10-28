//! The console: one process's header and actions, its live numbers, and the Logs, Settings and
//! Activity tabs.

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Color32, CornerRadius, Layout, Popup, Rect,
    RectAlign, ScrollArea, Sense, Stroke, StrokeKind, TextFormat, Ui,
};

use crate::config::{ProcessConfig, ProcessType};
use crate::process_manager::{ActivityKind, ProcessStatus};

use super::format::{self, LineKind, Stream, Tone};
use super::icons::{self, Icon};
use super::theme::{self, Tier, *};
use super::widgets::{self, Btn, Light};
use super::{LevelFilter, ProcessManagerApp, ProcessTab};

/// Width of the line-number column in the log.
const GUTTER_NUMBER: f32 = 56.0;
const GUTTER_TIME: f32 = 96.0;
const GUTTER_STREAM: f32 = 30.0;

enum HeadAction {
    Start,
    Stop,
    Restart,
    Edit,
    Duplicate,
    Reload,
    OpenFolder,
    OpenLogs,
    CopyCommand,
    MoveUp,
    MoveDown,
    ClearLog,
    Delete,
}

impl ProcessManagerApp {
    pub(super) fn draw_console(&mut self, ui: &mut Ui, process: &ProcessConfig, tier: Tier) {
        let full = ui.max_rect();
        let gutter = if tier <= Tier::Compact { 14.0 } else { GUTTER };
        let short = full.height() < 560.0;
        let mut ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(Layout::top_down(Align::Min)),
        );
        ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

        self.draw_process_head(&mut ui, process, tier, gutter, short);
        if tier >= Tier::Medium && full.height() >= 600.0 {
            self.draw_stats_strip(&mut ui, process, gutter);
        }
        self.draw_tabs(&mut ui, process, gutter);

        match self.tab {
            ProcessTab::Logs => self.draw_logs(&mut ui, process, tier, gutter),
            ProcessTab::Settings => self.draw_process_settings(&mut ui, process, tier, gutter),
            ProcessTab::Activity => self.draw_activity(&mut ui, process, gutter),
        }
    }

    fn draw_process_head(
        &mut self,
        ui: &mut Ui,
        process: &ProcessConfig,
        tier: Tier,
        gutter: f32,
        short: bool,
    ) {
        let view = self.status_view(process);
        let status = self.status_of(&process.id);
        let live = matches!(status, ProcessStatus::Running | ProcessStatus::Starting);
        let stopping = status == ProcessStatus::Stopping;
        let compact = tier <= Tier::Compact || short;
        let pad_top = if compact { 12.0 } else { 18.0 };
        let mut action = None;

        let outer = ui.available_rect_before_wrap();
        let inner_width = outer.width() - gutter * 2.0;
        let mut head = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_size(
                    pos2(outer.left() + gutter, outer.top() + pad_top),
                    vec2(inner_width, outer.height()),
                ))
                .layout(Layout::top_down(Align::Min)),
        );
        head.spacing_mut().item_spacing = vec2(8.0, 0.0);

        // Actions, measured so the title knows its room.
        let actions_width = {
            let mut w = Btn::new("Restart").with_icon(Icon::Restart).width(&head) + 30.0 + 12.0;
            w += if live {
                Btn::new("Stop").with_icon(Icon::Stop).width(&head)
            } else {
                Btn::new("Start").with_icon(Icon::Play).width(&head)
            };
            w
        };
        let icon_actions = tier == Tier::Tiny;
        let actions_width = if icon_actions {
            30.0 * 3.0 + 12.0
        } else {
            actions_width
        };
        let title_room = (inner_width - actions_width - 20.0).max(120.0);

        let title_top = head.cursor().top();
        // Title and status. Without the sidebar the header's switcher already names the
        // process, so the status leads here, with Details beside it.
        let tone_color = tone_color(view.tone);
        let show_name = tier.has_sidebar();
        let title_size = if compact { 20.0 } else { 24.0 };
        let (title_rect, _) = head.allocate_exact_size(vec2(title_room, 30.0), Sense::hover());
        let painter = head.painter().clone();
        let mut status_rect = title_rect;
        let mut details_inline_x = None;
        if show_name {
            let name_g =
                widgets::galley_truncated(&head, &process.name, bold(title_size), TEXT, title_room);
            let status_g = widgets::galley_truncated(
                &head,
                &view.word,
                bold(13.0),
                tone_color,
                (title_room - name_g.size().x - 30.0).max(60.0),
            );
            let status_inline = name_g.size().x + 14.0 + 16.0 + status_g.size().x <= title_room;
            let name_w = name_g.size().x;
            widgets::paint_galley_left(
                &painter,
                pos2(title_rect.left(), title_rect.center().y),
                name_g,
                TEXT,
            );
            if status_inline {
                let x = title_rect.left() + name_w + 14.0;
                widgets::paint_light(
                    &head,
                    &painter,
                    pos2(x + 4.0, title_rect.center().y + 1.0),
                    view.light,
                );
                widgets::paint_galley_left(
                    &painter,
                    pos2(x + 16.0, title_rect.center().y + 1.0),
                    status_g,
                    tone_color,
                );
            } else {
                let (r, _) = head.allocate_exact_size(vec2(title_room, 20.0), Sense::hover());
                widgets::paint_light(
                    &head,
                    &painter,
                    pos2(r.left() + 4.0, r.center().y),
                    view.light,
                );
                widgets::paint_galley_left(
                    &painter,
                    pos2(r.left() + 16.0, r.center().y),
                    status_g,
                    tone_color,
                );
                status_rect = r;
            }
        } else {
            let details_w = 80.0;
            let status_g = widgets::galley_truncated(
                &head,
                &view.word,
                bold(15.0),
                tone_color,
                (title_room - 16.0 - details_w - 8.0).max(60.0),
            );
            let status_w = status_g.size().x;
            widgets::paint_light(
                &head,
                &painter,
                pos2(title_rect.left() + 4.0, title_rect.center().y),
                view.light,
            );
            widgets::paint_galley_left(
                &painter,
                pos2(title_rect.left() + 16.0, title_rect.center().y),
                status_g,
                tone_color,
            );
            details_inline_x = Some(title_rect.left() + 16.0 + status_w + 10.0);
        }
        if let ProcessStatus::Error(message) = &status {
            let _ = head
                .interact(status_rect, head.id().with("status_detail"), Sense::hover())
                .on_hover_text(message.clone());
        }

        // Command line and badges: always on a roomy window, behind Details on a small one.
        let show_details = !compact || self.details_open;
        if compact {
            let label = if self.details_open {
                "Hide details"
            } else {
                "Details"
            };
            let icon = if self.details_open {
                Icon::CaretDown
            } else {
                Icon::CaretRight
            };
            let button = Btn::new(label).with_icon(icon).ghost().xsmall();
            let details_id = head.id().with("details");
            let clicked = match details_inline_x {
                Some(x) => {
                    let w = button.width(&head);
                    let r =
                        Rect::from_min_size(pos2(x, title_rect.center().y - 11.0), vec2(w, 22.0));
                    button.show_in(&mut head, r, details_id).clicked()
                }
                None => {
                    head.add_space(6.0);
                    button.show(&mut head).clicked()
                }
            };
            if clicked {
                self.details_open = !self.details_open;
            }
        }
        if show_details {
            head.add_space(10.0);
            if let Some(copy) = self.draw_command_line(
                &mut head,
                process,
                title_room.max(inner_width.min(title_room + actions_width)),
            ) {
                if copy {
                    action = Some(HeadAction::CopyCommand);
                }
            }
            head.add_space(10.0);
            head.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                ui.set_max_width(inner_width);
                for badge in format::badges(process) {
                    widgets::badge(ui, &badge, false);
                }
            });
        }
        let head_bottom = head.min_rect().bottom();

        // The actions, top right.
        let actions_rect = Rect::from_min_size(
            pos2(
                outer.left() + gutter + inner_width - actions_width,
                title_top,
            ),
            vec2(actions_width, CONTROL_HEIGHT),
        );
        let mut actions_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(actions_rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        actions_ui.spacing_mut().item_spacing.x = 6.0;
        let more = Btn::icon(Icon::More)
            .show(&mut actions_ui)
            .on_hover_text("More actions");
        Popup::menu(&more)
            .frame(widgets::popup_frame())
            .align(RectAlign::BOTTOM_END)
            .gap(6.0)
            .show(|ui| {
                ui.set_min_width(240.0);
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut item = |ui: &mut Ui,
                                label: &str,
                                kbd: Option<&str>,
                                danger: bool,
                                enabled: bool,
                                a: HeadAction| {
                    if widgets::menu_item(ui, label, kbd, danger, enabled).clicked() {
                        action = Some(a);
                        ui.close();
                    }
                };
                item(
                    ui,
                    "Edit settings",
                    Some("Ctrl ,"),
                    false,
                    true,
                    HeadAction::Edit,
                );
                item(ui, "Duplicate", None, false, true, HeadAction::Duplicate);
                item(
                    ui,
                    "Reload from processes.json",
                    None,
                    false,
                    true,
                    HeadAction::Reload,
                );
                widgets::menu_separator(ui);
                item(
                    ui,
                    "Copy the command",
                    None,
                    false,
                    true,
                    HeadAction::CopyCommand,
                );
                item(
                    ui,
                    "Open the working folder",
                    None,
                    false,
                    process.process_type == ProcessType::Process,
                    HeadAction::OpenFolder,
                );
                item(
                    ui,
                    "Open the log folder",
                    None,
                    false,
                    process.log_to_disk,
                    HeadAction::OpenLogs,
                );
                item(
                    ui,
                    "Clear the log view",
                    None,
                    false,
                    true,
                    HeadAction::ClearLog,
                );
                widgets::menu_separator(ui);
                item(ui, "Move up", None, false, true, HeadAction::MoveUp);
                item(ui, "Move down", None, false, true, HeadAction::MoveDown);
                widgets::menu_separator(ui);
                item(ui, "Delete…", None, true, true, HeadAction::Delete);
            });
        let restart = if icon_actions {
            Btn::icon(Icon::Restart)
        } else {
            Btn::new("Restart").with_icon(Icon::Restart)
        };
        if restart
            .show(&mut actions_ui)
            .on_hover_text("Restart · Ctrl R")
            .clicked()
        {
            action = Some(HeadAction::Restart);
        }
        if stopping {
            Btn::new("Stopping…").enabled(false).show(&mut actions_ui);
        } else if live {
            let stop = if icon_actions {
                Btn::icon(Icon::Stop)
            } else {
                Btn::new("Stop").with_icon(Icon::Stop)
            };
            if stop
                .show(&mut actions_ui)
                .on_hover_text("Stop · Ctrl Enter")
                .clicked()
            {
                action = Some(HeadAction::Stop);
            }
        } else {
            let start = if icon_actions {
                Btn::icon(Icon::Play).primary()
            } else {
                Btn::new("Start").with_icon(Icon::Play).primary()
            };
            if start
                .show(&mut actions_ui)
                .on_hover_text("Start · Ctrl Enter")
                .clicked()
            {
                action = Some(HeadAction::Start);
            }
        }

        let bottom_pad = if compact { 4.0 } else { 14.0 };
        let used = (head_bottom.max(actions_rect.bottom()) + bottom_pad) - outer.top();
        ui.allocate_exact_size(vec2(outer.width(), used), Sense::hover());

        if let Some(action) = action {
            let id = process.id.clone();
            match action {
                HeadAction::Start => self.start_process(&id),
                HeadAction::Stop => self.stop_process(&id),
                HeadAction::Restart => self.restart_process(&id),
                HeadAction::Edit => self.set_tab(ProcessTab::Settings),
                HeadAction::Duplicate => self.duplicate_process(&id),
                HeadAction::Reload => self.reload_process_from_disk(&id),
                HeadAction::OpenFolder => {
                    self.open_folder(std::path::PathBuf::from(&process.working_directory))
                }
                HeadAction::OpenLogs => {
                    let folder = self.process_log_folder(process);
                    self.open_folder(folder);
                }
                HeadAction::CopyCommand => {
                    let command = process.command.clone();
                    self.copy_text(&command, format!("Copied: {command}"));
                }
                HeadAction::MoveUp => self.move_process_up(&id),
                HeadAction::MoveDown => self.move_process_down(&id),
                HeadAction::ClearLog => {
                    self.manager.clear_logs(&id);
                    self.log_selection = None;
                    self.set_banner(format!(
                        "Cleared the log view of {}; logs on disk are untouched",
                        process.name
                    ));
                }
                HeadAction::Delete => self.delete_process_id = Some(id),
            }
        }
    }

    /// `command  in  folder  [copy]`. Returns whether copy was clicked.
    fn draw_command_line(
        &self,
        ui: &mut Ui,
        process: &ProcessConfig,
        max_width: f32,
    ) -> Option<bool> {
        let (command, joiner, place) = match process.process_type {
            ProcessType::Process => (
                process.command.as_str(),
                "in",
                if process.working_directory.trim().is_empty() {
                    "the executable’s folder".to_string()
                } else {
                    process.working_directory.clone()
                },
            ),
            ProcessType::Docker => (
                process.command.as_str(),
                "via",
                format!("docker start -a {}", process.command),
            ),
        };
        let joiner_g = widgets::galley(ui, joiner, body(12.0), FAINT);
        let room = (max_width - joiner_g.size().x - 24.0 - 26.0 - 16.0).max(80.0);
        let command_g = widgets::galley_truncated(
            ui,
            if command.is_empty() {
                "(no command)"
            } else {
                command
            },
            mono(12.0),
            TEXT,
            room * 0.55,
        );
        let place_g =
            widgets::galley_truncated(ui, &place, mono(12.0), MUTED, room - command_g.size().x);
        let height = 24.0;
        let width = command_g.size().x
            + 16.0
            + 8.0
            + joiner_g.size().x
            + 8.0
            + place_g.size().x
            + 16.0
            + 8.0
            + 24.0;
        let (rect, _) = ui.allocate_exact_size(vec2(width.min(max_width), height), Sense::hover());
        let painter = ui.painter();
        let chip = |painter: &egui::Painter,
                    x: f32,
                    g: std::sync::Arc<egui::Galley>,
                    color: Color32|
         -> f32 {
            let r = Rect::from_min_size(pos2(x, rect.top()), vec2(g.size().x + 16.0, height));
            painter.rect(
                r,
                CornerRadius::same(RADIUS),
                PANEL,
                Stroke::new(1.0_f32, LINE),
                StrokeKind::Inside,
            );
            widgets::paint_galley_left(painter, pos2(r.left() + 8.0, r.center().y), g, color);
            r.right()
        };
        let mut x = chip(painter, rect.left(), command_g, TEXT) + 8.0;
        let jw = joiner_g.size().x;
        widgets::paint_galley_left(painter, pos2(x, rect.center().y), joiner_g, FAINT);
        x += jw + 8.0;
        x = chip(painter, x, place_g, MUTED) + 6.0;
        let copy_rect = Rect::from_min_size(pos2(x, rect.top()), vec2(24.0, height));
        let copy = Btn::icon(Icon::Copy).ghost().xsmall().show_in(
            ui,
            copy_rect,
            ui.id().with("copy_command"),
        );
        let full = format!("{command} {joiner} {place}");
        let _ = ui
            .interact(rect, ui.id().with("command_hover"), Sense::hover())
            .on_hover_text(full);
        Some(copy.on_hover_text("Copy the command").clicked())
    }

    fn draw_stats_strip(&mut self, ui: &mut Ui, process: &ProcessConfig, gutter: f32) {
        let outer = ui.available_rect_before_wrap();
        let rect = Rect::from_min_size(
            pos2(outer.left() + gutter, outer.top()),
            vec2(outer.width() - gutter * 2.0, 58.0),
        );
        ui.allocate_exact_size(vec2(outer.width(), 58.0), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect(
            rect,
            CornerRadius::same(RADIUS_CARD),
            PANEL,
            Stroke::new(1.0_f32, LINE),
            StrokeKind::Inside,
        );

        let status = self.status_of(&process.id);
        let running = status == ProcessStatus::Running;
        let docker = process.process_type == ProcessType::Docker;
        let usage = self.usage_of(&process.id).unwrap_or_default();
        let info = self.info_of(&process.id).cloned().unwrap_or_default();
        let history = &self.runtime_snapshot.selected_history;
        let cpu_values: Vec<f32> = history
            .iter()
            .map(|u| u.cpu_percent.unwrap_or(0.0))
            .collect();
        let mem_values: Vec<f32> = history
            .iter()
            .map(|u| u.memory_bytes.unwrap_or(0) as f32 / 1_048_576.0)
            .collect();

        let (cpu, cpu_unit) = if docker {
            ("—".to_string(), "not sampled")
        } else if running {
            let text = format::format_cpu(usage.cpu_percent.unwrap_or(0.0));
            (text.trim_end_matches('%').to_string(), "%")
        } else {
            ("0.0".to_string(), "%")
        };
        let (mem, mem_unit) = if docker {
            ("—".to_string(), "not sampled")
        } else if running {
            let (value, unit) = format::memory_parts(usage.memory_bytes.unwrap_or(0));
            (value, unit)
        } else {
            ("0".to_string(), "MB")
        };
        let uptime = if running {
            format::uptime_seconds(Some(&info))
                .map(format::duration_text)
                .unwrap_or_else(|| "—".into())
        } else {
            "—".to_string()
        };
        let pid = info
            .pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "—".into());
        let restarts = info.start_count.saturating_sub(1).to_string();

        let cells: [(&str, String, &str, Option<(&[f32], bool)>); 5] = [
            (
                "CPU",
                cpu,
                cpu_unit,
                (!docker).then_some((cpu_values.as_slice(), true)),
            ),
            (
                "Memory",
                mem,
                mem_unit,
                (!docker).then_some((mem_values.as_slice(), false)),
            ),
            ("Uptime", uptime, "", None),
            ("PID", pid, "", None),
            ("Restarts", restarts, "this session", None),
        ];
        let cell_w = rect.width() / cells.len() as f32;
        for (index, (label, value, unit, spark)) in cells.into_iter().enumerate() {
            let cell = Rect::from_min_size(
                pos2(rect.left() + cell_w * index as f32, rect.top()),
                vec2(cell_w, rect.height()),
            );
            if index > 0 {
                painter.vline(cell.left(), cell.y_range(), Stroke::new(1.0_f32, LINE));
            }
            let mut job = LayoutJob::default();
            job.append(
                &label.to_uppercase(),
                0.0,
                TextFormat {
                    font_id: bold(10.5),
                    color: FAINT,
                    extra_letter_spacing: 0.9,
                    ..Default::default()
                },
            );
            let label_g = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(pos2(cell.left() + 14.0, cell.top() + 11.0), label_g, FAINT);
            let value_g = widgets::galley(ui, &value, bold(18.0), TEXT);
            let vx = cell.left() + 14.0;
            let vy = cell.top() + 26.0;
            let value_w = value_g.size().x;
            painter.galley(pos2(vx, vy), value_g, TEXT);
            if !unit.is_empty() {
                let unit_g = widgets::galley(ui, unit, body(11.5), FAINT);
                painter.galley(pos2(vx + value_w + 3.0, vy + 6.0), unit_g, FAINT);
            }
            if let Some((values, area)) = spark {
                if cell_w >= 170.0 && values.len() >= 2 {
                    let spark_rect = Rect::from_min_size(
                        pos2(cell.right() - 14.0 - 84.0, cell.bottom() - 10.0 - 26.0),
                        vec2(84.0, 26.0),
                    );
                    if area {
                        widgets::sparkline(&painter, spark_rect, values, 5.0, ACCENT, true);
                    } else {
                        widgets::sparkline_range(&painter, spark_rect, values, MUTED);
                    }
                }
            }
        }
    }

    fn draw_tabs(&mut self, ui: &mut Ui, process: &ProcessConfig, gutter: f32) {
        let outer = ui.available_rect_before_wrap();
        let height = 48.0;
        ui.allocate_exact_size(vec2(outer.width(), height), Sense::hover());
        let painter = ui.painter().clone();
        let base_y = outer.top() + height;
        painter.hline(outer.x_range(), base_y - 0.5, Stroke::new(1.0_f32, LINE));
        let logs_count = self
            .info_of(&process.id)
            .map(|info| format::thousands(info.log_total))
            .unwrap_or_default();
        let dirty = self
            .editor
            .as_ref()
            .is_some_and(|e| e.process_id.as_deref() == Some(process.id.as_str()) && e.is_dirty());
        let activity = self.runtime_snapshot.selected_activity.len().to_string();
        let tabs = [
            (ProcessTab::Logs, "Logs", logs_count),
            (
                ProcessTab::Settings,
                "Settings",
                if dirty {
                    "unsaved".to_string()
                } else {
                    String::new()
                },
            ),
            (ProcessTab::Activity, "Activity", activity),
        ];
        let mut x = outer.left() + gutter;
        for (tab, label, count) in tabs {
            let label_g = widgets::galley(ui, label, bold(13.0), MUTED);
            let count_g = (!count.is_empty()).then(|| {
                widgets::galley(
                    ui,
                    &count,
                    bold(11.0),
                    if count == "unsaved" { AMBER } else { FAINT },
                )
            });
            let w = 24.0 + label_g.size().x + count_g.as_ref().map_or(0.0, |g| g.size().x + 7.0);
            let rect = Rect::from_min_max(pos2(x, base_y - 36.0), pos2(x + w, base_y));
            let response = ui.interact(rect, ui.id().with(("tab", label)), Sense::click());
            let on = self.tab == tab;
            let color = if on || response.hovered() {
                TEXT
            } else {
                MUTED
            };
            let lw = label_g.size().x;
            widgets::paint_galley_left(
                &painter,
                pos2(rect.left() + 12.0, rect.center().y),
                label_g,
                color,
            );
            if let Some(g) = count_g {
                let c = if count == "unsaved" { AMBER } else { FAINT };
                widgets::paint_galley_left(
                    &painter,
                    pos2(rect.left() + 12.0 + lw + 7.0, rect.center().y + 0.5),
                    g,
                    c,
                );
            }
            if on {
                painter.rect_filled(
                    Rect::from_min_max(pos2(rect.left(), base_y - 2.0), pos2(rect.right(), base_y)),
                    CornerRadius::ZERO,
                    ACCENT,
                );
            }
            if response.clicked() {
                self.set_tab(tab);
            }
            x += w + 4.0;
        }
    }

    fn draw_logs(&mut self, ui: &mut Ui, process: &ProcessConfig, tier: Tier, gutter: f32) {
        let snapshot_lines = self.runtime_snapshot.selected_logs.clone();
        let first_line = self.runtime_snapshot.selected_first_line;
        let words: Vec<String> = self
            .log_filter
            .to_ascii_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();

        // Classify once per frame.
        let classified: Vec<(
            u64,
            &crate::process_manager::LogLine,
            Stream,
            LineKind,
            &str,
        )> = snapshot_lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                let (stream, kind, text) = format::classify(&line.text);
                (first_line + i as u64, line, stream, kind, text)
            })
            .collect();
        let count = |k: LineKind| classified.iter().filter(|c| c.3 == k).count();
        let (errors, warnings, events) = (
            count(LineKind::Error),
            count(LineKind::Warning),
            count(LineKind::Event),
        );
        let filtered: Vec<_> = classified
            .iter()
            .filter(|(_, _, _, kind, _)| match self.log_level {
                LevelFilter::All => true,
                LevelFilter::Errors => *kind == LineKind::Error,
                LevelFilter::Warnings => *kind == LineKind::Warning,
                LevelFilter::Events => *kind == LineKind::Event,
            })
            .filter(|(_, _, _, _, text)| {
                if words.is_empty() {
                    return true;
                }
                let lower = text.to_ascii_lowercase();
                words.iter().all(|w| lower.contains(w.as_str()))
            })
            .collect();

        // Toolbar.
        let outer = ui.available_rect_before_wrap();
        let toolbar_rect = Rect::from_min_size(
            pos2(outer.left() + gutter, outer.top() + 10.0),
            vec2(outer.width() - gutter * 2.0, SMALL_CONTROL_HEIGHT + 4.0),
        );
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(toolbar_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        bar.spacing_mut().item_spacing.x = 8.0;
        let total = classified.len();
        let hits = (!words.is_empty()).then(|| {
            format!(
                "{} {}",
                format::thousands(filtered.len() as u64),
                if filtered.len() == 1 { "line" } else { "lines" }
            )
        });
        let search_w = if tier >= Tier::Full {
            260.0
        } else {
            (toolbar_rect.width() * 0.45).clamp(140.0, 240.0)
        };
        let search = widgets::search_field(
            &mut bar,
            "log_filter",
            &mut self.log_filter,
            "Filter lines",
            search_w,
            hits.as_deref(),
        );
        if self.focus_log_filter {
            search.request_focus();
            self.focus_log_filter = false;
        }
        if search.has_focus() && bar.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.log_filter.clear();
        }
        let levels = [
            (LevelFilter::All, "All", total, Light::Scheduled),
            (LevelFilter::Errors, "Errors", errors, Light::Error),
            (LevelFilter::Warnings, "Warnings", warnings, Light::Busy),
            (
                LevelFilter::Events,
                "Process events",
                events,
                Light::Running,
            ),
        ];
        // Fold the chips into one dropdown, then the toggles into a menu, as room runs out.
        let chips_w: f32 = levels
            .iter()
            .map(|(_, label, n, _)| {
                let label = widgets::galley(&bar, *label, bold(12.0), MUTED).size().x;
                let count = widgets::galley(&bar, format::thousands(*n as u64), body(12.0), FAINT)
                    .size()
                    .x;
                20.0 + 14.0 + label + 6.0 + count + 4.0
            })
            .sum();
        let toggles_w = 52.0 + 54.0 + 82.0 + 26.0 + 6.0 * 4.0;
        let selection_w = if self.selected_log_range(&process.id).is_some() {
            150.0
        } else {
            0.0
        };
        let room = toolbar_rect.width() - search_w - 24.0 - selection_w;
        let chips_inline = room >= chips_w + toggles_w;
        let wide_toolbar = room >= 110.0 + toggles_w;
        if chips_inline {
            for (level, label, n, light) in levels {
                let n = format::thousands(n as u64);
                if widgets::chip(
                    &mut bar,
                    label,
                    Some(&n),
                    self.log_level == level,
                    Some(light),
                )
                .clicked()
                {
                    self.log_level = level;
                    self.follow_logs = true;
                }
            }
        } else {
            let current = levels
                .iter()
                .find(|l| l.0 == self.log_level)
                .copied()
                .unwrap_or(levels[0]);
            let n = format::thousands(current.2 as u64);
            let chip = widgets::chip(&mut bar, current.1, Some(&n), true, Some(current.3));
            Popup::menu(&chip)
                .frame(widgets::popup_frame())
                .gap(6.0)
                .show(|ui| {
                    ui.set_min_width(200.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (level, label, n, _) in levels {
                        let text = format!("{label}  ·  {}", format::thousands(n as u64));
                        if widgets::menu_item(ui, &text, None, false, true).clicked() {
                            self.log_level = level;
                            self.follow_logs = true;
                            ui.close();
                        }
                    }
                });
        }

        // Right side of the toolbar.
        let mut right = bar.new_child(
            egui::UiBuilder::new()
                .max_rect(toolbar_rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        right.spacing_mut().item_spacing.x = 6.0;
        let selected = self.selected_log_range(&process.id);
        if wide_toolbar {
            if Btn::icon(Icon::Folder)
                .small()
                .enabled(process.log_to_disk)
                .show(&mut right)
                .on_hover_text(if process.log_to_disk {
                    "Open the log folder"
                } else {
                    "Logs are not written to disk; turn it on in Settings"
                })
                .clicked()
            {
                let folder = self.process_log_folder(process);
                self.open_folder(folder);
            }
            if widgets::toggle(
                &mut right,
                "Follow",
                Some(Icon::ArrowDown),
                self.follow_logs,
            )
            .on_hover_text("Keep the newest line in view")
            .clicked()
            {
                self.follow_logs = !self.follow_logs;
                self.jump_to_latest = self.follow_logs;
                self.unfollowed_at = self.info_of(&process.id).map_or(0, |i| i.log_total);
            }
            if widgets::toggle(&mut right, "Wrap", None, self.wrap_logs).clicked() {
                self.wrap_logs = !self.wrap_logs;
            }
            if widgets::toggle(&mut right, "Time", None, self.show_log_time).clicked() {
                self.show_log_time = !self.show_log_time;
            }
        } else {
            let more = Btn::icon(Icon::More)
                .small()
                .show(&mut right)
                .on_hover_text("View options");
            Popup::menu(&more)
                .frame(widgets::popup_frame())
                .align(RectAlign::BOTTOM_END)
                .gap(6.0)
                .show(|ui| {
                    ui.set_min_width(220.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let check = |on: bool| if on { "✓  " } else { "    " };
                    if widgets::menu_item(
                        ui,
                        &format!("{}Follow the newest line", check(self.follow_logs)),
                        None,
                        false,
                        true,
                    )
                    .clicked()
                    {
                        self.follow_logs = !self.follow_logs;
                        self.jump_to_latest = self.follow_logs;
                        self.unfollowed_at = self.info_of(&process.id).map_or(0, |i| i.log_total);
                        ui.close();
                    }
                    if widgets::menu_item(
                        ui,
                        &format!("{}Wrap long lines", check(self.wrap_logs)),
                        None,
                        false,
                        true,
                    )
                    .clicked()
                    {
                        self.wrap_logs = !self.wrap_logs;
                        ui.close();
                    }
                    if widgets::menu_item(
                        ui,
                        &format!("{}Show times", check(self.show_log_time)),
                        None,
                        false,
                        true,
                    )
                    .clicked()
                    {
                        self.show_log_time = !self.show_log_time;
                        ui.close();
                    }
                    if process.log_to_disk
                        && widgets::menu_item(ui, "Open the log folder", None, false, true)
                            .clicked()
                    {
                        let folder = self.process_log_folder(process);
                        self.open_folder(folder);
                        ui.close();
                    }
                });
        }
        if let Some((start, end)) = selected {
            if Btn::new("Clear").small().ghost().show(&mut right).clicked() {
                self.log_selection = None;
            }
            if Btn::new("Copy")
                .with_icon(Icon::Copy)
                .small()
                .show(&mut right)
                .on_hover_text("Ctrl C")
                .clicked()
            {
                self.copy_selected_logs();
            }
            if wide_toolbar {
                let n = end - start + 1;
                right.label(
                    egui::RichText::new(format!(
                        "{n} {} selected",
                        if n == 1 { "line" } else { "lines" }
                    ))
                    .font(body(12.0))
                    .color(MUTED),
                );
            }
        }
        ui.allocate_exact_size(
            vec2(outer.width(), 10.0 + SMALL_CONTROL_HEIGHT + 4.0 + 10.0),
            Sense::hover(),
        );

        // The log panel.
        let outer = ui.available_rect_before_wrap();
        let panel = Rect::from_min_max(
            pos2(outer.left() + gutter, outer.top()),
            pos2(
                outer.right() - gutter,
                outer.bottom() - if tier <= Tier::Compact { 10.0 } else { 14.0 },
            ),
        );
        ui.allocate_rect(outer, Sense::hover());
        let painter = ui.painter().clone();
        painter.rect(
            panel,
            CornerRadius::same(RADIUS_CARD),
            LOG_BG,
            Stroke::new(1.0_f32, LINE),
            StrokeKind::Inside,
        );
        let inner = panel.shrink(1.0);
        let mut log_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Min)),
        );
        log_ui.set_clip_rect(inner);

        if filtered.is_empty() {
            let text = if classified.is_empty() {
                match process.process_type {
                    _ if self.is_live(&process.id) => {
                        format!("{} has not written anything yet.", process.name)
                    }
                    _ => format!("No output yet. Start {} to see its log here.", process.name),
                }
            } else {
                "No line matches this filter.".to_string()
            };
            let g = widgets::galley(&log_ui, &text, body(13.0), MUTED);
            log_ui.painter().galley(
                pos2(inner.center().x - g.size().x * 0.5, inner.top() + 40.0),
                g,
                MUTED,
            );
            return;
        }

        let show_time = self.show_log_time && tier > Tier::Tiny;
        let number_w = if tier <= Tier::Compact {
            44.0
        } else {
            GUTTER_NUMBER
        };
        let text_left = number_w + if show_time { GUTTER_TIME } else { 0.0 } + GUTTER_STREAM;
        let frozen = self
            .frozen_log_line
            .clone()
            .filter(|(id, _)| id == &process.id)
            .map(|(_, n)| n);
        let jump = std::mem::take(&mut self.jump_to_latest);
        let wrap = self.wrap_logs;
        let wrap_width = (inner.width() - text_left - 14.0).max(80.0);

        // Wrapped lines need their layout for their height, so all are laid out; unwrapped
        // lines are one row each, so only the rows in view are laid out (the width comes from
        // the longest line, the font being monospaced).
        let layout = |ui: &Ui, text: &str, kind: LineKind| {
            let job = line_job(
                text,
                kind,
                &words,
                if wrap { Some(wrap_width) } else { None },
            );
            ui.fonts_mut(|f| f.layout_job(job))
        };
        let wrapped: Vec<std::sync::Arc<egui::Galley>> = if wrap {
            filtered
                .iter()
                .map(|(_, _, _, kind, text)| layout(&log_ui, text, *kind))
                .collect()
        } else {
            Vec::new()
        };
        let char_w = widgets::galley(&log_ui, "0", mono(12.0), LOG_TEXT).size().x;
        let content_w = if wrap {
            inner.width()
        } else {
            let longest = filtered
                .iter()
                .map(|c| c.4.chars().count())
                .max()
                .unwrap_or(0);
            longest as f32 * char_w + text_left + 24.0
        };

        let mut clicked: Option<(u64, bool)> = None;
        let mut double: Option<u64> = None;
        let mut scroll = ScrollArea::both()
            .id_salt(("log", &process.id))
            .auto_shrink([false, false])
            .stick_to_bottom(self.follow_logs);
        if !wrap {
            scroll = scroll
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded);
        } else {
            scroll = scroll.hscroll(false);
        }
        if jump {
            scroll = scroll.vertical_scroll_offset(f32::MAX);
        }
        let output = scroll.show(&mut log_ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
            ui.add_space(6.0);
            let (lo, hi) = selected.unwrap_or((u64::MAX, 0));
            for (index, (number, line, stream, kind, text)) in filtered.iter().enumerate() {
                let height = match wrapped.get(index) {
                    Some(galley) => galley.size().y.max(LOG_ROW_HEIGHT - 4.0) + 4.0,
                    None => LOG_ROW_HEIGHT,
                };
                let (rect, response) = ui.allocate_exact_size(
                    vec2(content_w.max(ui.available_width()), height),
                    Sense::click(),
                );
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                let galley = match wrapped.get(index) {
                    Some(galley) => galley.clone(),
                    None => layout(ui, text, *kind),
                };
                let painter = ui.painter();
                let is_selected = *number >= lo && *number <= hi;
                if is_selected {
                    painter.rect_filled(rect, CornerRadius::ZERO, theme::accent_soft());
                    painter.rect_filled(
                        Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
                        CornerRadius::ZERO,
                        ACCENT,
                    );
                } else if *kind == LineKind::Error {
                    painter.rect_filled(rect, CornerRadius::ZERO, theme::red_soft());
                    painter.rect_filled(
                        Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
                        CornerRadius::ZERO,
                        RED,
                    );
                } else if response.hovered() {
                    painter.rect_filled(rect, CornerRadius::ZERO, HOVER);
                }
                let top = rect.top() + 10.0;
                let number_color = if is_selected {
                    TEXT
                } else if *kind == LineKind::Error {
                    RED
                } else {
                    FAINT.gamma_multiply(0.8)
                };
                painter.text(
                    pos2(rect.left() + number_w - 12.0, top),
                    egui::Align2::RIGHT_CENTER,
                    number.to_string(),
                    mono(11.0),
                    number_color,
                );
                let mut x = rect.left() + number_w;
                if show_time {
                    painter.text(
                        pos2(x, top),
                        egui::Align2::LEFT_CENTER,
                        line.at.format("%H:%M:%S%.3f").to_string(),
                        mono(11.0),
                        FAINT,
                    );
                    x += GUTTER_TIME;
                }
                let (tag, tag_color) = match stream {
                    Stream::Err => ("err", FAINT),
                    Stream::Event => ("›", ACCENT),
                    Stream::Out => ("", FAINT),
                };
                if !tag.is_empty() {
                    painter.text(
                        pos2(x + GUTTER_STREAM * 0.5, top),
                        egui::Align2::CENTER_CENTER,
                        tag,
                        mono(10.5),
                        tag_color,
                    );
                }
                x += GUTTER_STREAM;
                let text_pos = pos2(
                    x,
                    rect.top()
                        + (LOG_ROW_HEIGHT - galley.rows.first().map_or(14.0, |r| r.height())) * 0.5,
                );
                if frozen == Some(*number) {
                    let text_rect = Rect::from_min_size(text_pos, galley.size());
                    let mut child = ui.new_child(
                        egui::UiBuilder::new().max_rect(text_rect.expand2(vec2(0.0, 2.0))),
                    );
                    child.add(
                        egui::Label::new(egui::WidgetText::LayoutJob(
                            galley.job.as_ref().clone().into(),
                        ))
                        .selectable(true),
                    );
                } else {
                    ui.painter().galley(text_pos, galley, LOG_TEXT);
                }
                if response.double_clicked() {
                    double = Some(*number);
                } else if response.clicked() {
                    clicked = Some((*number, ui.input(|i| i.modifiers.shift)));
                }
            }
            ui.add_space(10.0);
        });

        let max_offset = (output.content_size.y - output.inner_rect.height()).max(0.0);
        let at_bottom = max_offset - output.state.offset.y <= 24.0;
        let log_total = self.info_of(&process.id).map_or(0, |i| i.log_total);
        if self.follow_logs
            && !at_bottom
            && !jump
            && max_offset > 0.0
            && ui.input(|i| i.smooth_scroll_delta.y > 0.0 || i.pointer.is_decidedly_dragging())
        {
            self.follow_logs = false;
            self.unfollowed_at = log_total;
        } else if !self.follow_logs && at_bottom && ui.input(|i| i.smooth_scroll_delta.y < 0.0) {
            self.follow_logs = true;
        }

        if let Some(number) = double {
            self.frozen_log_line = Some((process.id.clone(), number));
            self.select_log_line(&process.id, number, false);
        } else if let Some((number, shift)) = clicked {
            self.frozen_log_line = None;
            self.select_log_line(&process.id, number, shift);
        }

        // "↓ 12 new lines" while reading back.
        let new_lines = log_total.saturating_sub(self.unfollowed_at);
        if !self.follow_logs && new_lines > 0 {
            let label = format!(
                "{} new {}",
                format::thousands(new_lines),
                if new_lines == 1 { "line" } else { "lines" }
            );
            let g = widgets::galley(ui, &label, bold(12.0), ON_ACCENT);
            let w = g.size().x + 14.0 + 13.0 + 7.0 + 14.0;
            let r = Rect::from_center_size(
                pos2(panel.center().x, panel.bottom() - 14.0 - 15.0),
                vec2(w, 30.0),
            );
            let response = ui.interact(r, ui.id().with("new_lines"), Sense::click());
            let fill = if response.hovered() {
                ACCENT_HOVER
            } else {
                ACCENT
            };
            let painter = ui.painter();
            painter.add(
                egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    color: Color32::from_black_alpha(120),
                }
                .as_shape(r, CornerRadius::same(15)),
            );
            painter.rect_filled(r, CornerRadius::same(15), fill);
            icons::paint(
                painter,
                icons::square(pos2(r.left() + 14.0 + 6.5, r.center().y), 13.0),
                13.0,
                Icon::ArrowDown,
                ON_ACCENT,
            );
            widgets::paint_galley_left(
                painter,
                pos2(r.left() + 14.0 + 13.0 + 7.0, r.center().y),
                g,
                ON_ACCENT,
            );
            if response.clicked() {
                self.follow_logs = true;
                self.jump_to_latest = true;
            }
        }
    }

    fn draw_activity(&mut self, ui: &mut Ui, process: &ProcessConfig, gutter: f32) {
        let outer = ui.available_rect_before_wrap();
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(outer.shrink2(vec2(gutter, 0.0)))
                .layout(Layout::top_down(Align::Min)),
        );
        inner.add_space(14.0);
        inner.label(
            egui::RichText::new(format!(
                "What happened to {} this session, and what caused it.",
                process.name
            ))
            .font(body(12.5))
            .color(MUTED),
        );
        inner.add_space(10.0);
        let events = self.runtime_snapshot.selected_activity.clone();
        let now = chrono::Local::now();
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(&mut inner, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if events.is_empty() {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new(
                            "Nothing yet. Starts, stops, exits and restarts show up here.",
                        )
                        .font(body(13.0))
                        .color(MUTED),
                    );
                    return;
                }
                for event in events.iter().rev() {
                    let (rect, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
                    let painter = ui.painter();
                    painter.hline(
                        rect.x_range(),
                        rect.bottom() - 0.5,
                        Stroke::new(1.0_f32, ROW_LINE),
                    );
                    let when = format::when(event.at, now);
                    painter.text(
                        pos2(rect.left(), rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &when,
                        mono(11.5),
                        FAINT,
                    );
                    let light = match event.kind {
                        ActivityKind::Started => Light::Running,
                        ActivityKind::Stopped => Light::Stopped,
                        ActivityKind::Exited => {
                            if event.text.ends_with(" 0") {
                                Light::Stopped
                            } else {
                                Light::Error
                            }
                        }
                        ActivityKind::Failed => Light::Error,
                        ActivityKind::Settings => Light::Scheduled,
                    };
                    widgets::paint_light(
                        ui,
                        painter,
                        pos2(rect.left() + 142.0 + 4.0, rect.center().y),
                        light,
                    );
                    let cause_g = (!event.cause.is_empty())
                        .then(|| widgets::galley(ui, &event.cause, body(12.0), MUTED));
                    let cause_w = cause_g.as_ref().map_or(0.0, |g| g.size().x + 16.0);
                    let text_left = rect.left() + 158.0;
                    widgets::text_in(
                        ui,
                        Rect::from_min_max(
                            pos2(text_left, rect.top()),
                            pos2(rect.right() - cause_w, rect.bottom()),
                        ),
                        &event.text,
                        bold(13.0),
                        TEXT,
                    );
                    if let Some(g) = cause_g {
                        widgets::paint_galley_left(
                            painter,
                            pos2(rect.right() - g.size().x, rect.center().y),
                            g,
                            MUTED,
                        );
                    }
                }
            });
    }

    pub(super) fn draw_empty_state(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Center)),
        );
        child.add_space((rect.height() * 0.32).max(40.0));
        let (mark, _) = child.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        super::header::paint_mark(child.painter(), mark);
        child.add_space(18.0);
        child.label(
            egui::RichText::new("Your stack starts here")
                .font(bold(22.0))
                .color(TEXT),
        );
        child.add_space(8.0);
        child.label(egui::RichText::new("Add a command or a Docker container; Process Manager runs it, keeps its log and restarts it if you like.").font(body(13.0)).color(MUTED));
        child.add_space(18.0);
        if Btn::new("Add a process")
            .with_icon(Icon::Plus)
            .primary()
            .show(&mut child)
            .clicked()
        {
            self.open_new_process();
        }
    }
}

pub(super) fn tone_color(tone: Tone) -> Color32 {
    match tone {
        Tone::Good => GREEN,
        Tone::Busy => AMBER,
        Tone::Bad => RED,
        Tone::Quiet => MUTED,
    }
}

/// One log line's text as a layout job: coloured by kind, with search matches highlighted.
fn line_job(text: &str, kind: LineKind, words: &[String], wrap: Option<f32>) -> LayoutJob {
    let color = match kind {
        LineKind::Normal => LOG_TEXT,
        LineKind::Warning => AMBER,
        LineKind::Error => LOG_ERROR_TEXT,
        LineKind::Event => ACCENT,
    };
    let font = mono(12.0);
    let mut job = LayoutJob::default();
    if let Some(width) = wrap {
        job.wrap.max_width = width;
    }
    let plain = TextFormat {
        font_id: font.clone(),
        color,
        ..Default::default()
    };
    if words.is_empty() {
        job.append(text, 0.0, plain);
        return job;
    }
    // ASCII lowering keeps byte offsets aligned with `text`.
    let lower = text.to_ascii_lowercase();
    let mut marks = vec![false; text.len()];
    for word in words {
        let mut start = 0;
        while let Some(found) = lower[start..].find(word.as_str()) {
            let at = start + found;
            for mark in marks.iter_mut().skip(at).take(word.len()) {
                *mark = true;
            }
            start = at + word.len().max(1);
            if start >= lower.len() {
                break;
            }
        }
    }
    let hit = TextFormat {
        font_id: font,
        color: Color32::WHITE,
        background: theme::hit_fill(),
        ..Default::default()
    };
    let mut run_start = 0;
    let mut run_on = marks.first().copied().unwrap_or(false);
    let boundary = |i: usize| text.is_char_boundary(i);
    for i in 1..=text.len() {
        let on = if i < text.len() { marks[i] } else { !run_on };
        if (on != run_on || i == text.len()) && boundary(i) {
            let slice = &text[run_start..i];
            job.append(slice, 0.0, if run_on { hit.clone() } else { plain.clone() });
            run_start = i;
            run_on = on;
        }
    }
    job
}
