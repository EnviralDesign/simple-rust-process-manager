//! The overview: every process in one table under its group, with a filter, state chips, bulk
//! actions for checked rows, and the stack log (every process's output merged by time).

use eframe::egui::{
    self, pos2, vec2, Align, Align2, CornerRadius, Layout, Popup, Rect, RectAlign, ScrollArea,
    Sense, Stroke, StrokeKind, Ui,
};

use crate::config::{ProcessConfig, ProcessType};
use crate::process_manager::ProcessStatus;

use super::console::tone_color;
use super::format::{self, LineKind};
use super::icons::{self, Icon};
use super::theme::{self, Tier, *};
use super::widgets::{self, Btn, Light};
use super::{Nav, OverviewChip, ProcessManagerApp, StackAction};

const UNGROUPED: &str = "\u{0}ungrouped";

#[derive(Clone, Copy)]
struct Columns {
    resources: bool,
    memory: bool,
    uptime: bool,
    flags: bool,
    command: bool,
}

impl Columns {
    fn for_width(width: f32) -> Self {
        Self {
            resources: width >= 560.0,
            memory: width >= 760.0,
            uptime: width >= 860.0,
            flags: width >= 1060.0,
            command: width >= 1160.0,
        }
    }
}

enum TableAction {
    Open(String),
    Toggle(String),
    Restart(String),
    Check(String),
    CheckGroup(Vec<String>, bool),
    Collapse(String),
    Group(String, StackAction),
    Rename(String),
    Ungroup(String),
}

impl ProcessManagerApp {
    pub(super) fn draw_overview(&mut self, ui: &mut Ui, tier: Tier) {
        let full = ui.max_rect();
        let gutter = if tier <= Tier::Compact { 14.0 } else { GUTTER };
        let mut page = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(Layout::top_down(Align::Min)),
        );
        page.spacing_mut().item_spacing = vec2(0.0, 0.0);
        let rollup = self.stack_rollup();

        // Title.
        let group = self
            .overview_group
            .clone()
            .and_then(|id| self.config.get_group(&id).cloned());
        page.add_space(18.0);
        page.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.add_space(gutter);
            let title = group
                .as_ref()
                .map_or("Overview".to_string(), |g| g.name.clone());
            ui.label(egui::RichText::new(title).font(bold(24.0)).color(TEXT));
            let text = match &group {
                Some(g) => {
                    let r = self.rollup(g.process_ids.iter());
                    format!("{} processes · {} running", r.total, r.running)
                }
                None => {
                    let mut text =
                        format!("{} processes · {} running", rollup.total, rollup.running);
                    if rollup.memory > 0 && tier >= Tier::Medium {
                        text.push_str(&format!(
                            " · {} CPU · {}",
                            format::format_cpu(rollup.cpu),
                            format::format_memory(rollup.memory)
                        ));
                    }
                    text
                }
            };
            ui.label(egui::RichText::new(text).font(bold(13.0)).color(MUTED));
            if group.is_some()
                && Btn::new("Show every process")
                    .small()
                    .ghost()
                    .show(ui)
                    .clicked()
            {
                self.overview_group = None;
            }
        });
        page.add_space(10.0);

        // Toolbar.
        page.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            ui.add_space(gutter);
            let width = if tier >= Tier::Full { 260.0 } else { 200.0 };
            widgets::search_field(
                ui,
                "overview_filter",
                &mut self.overview_filter,
                "Filter: name, command, group, status",
                width,
                None,
            );
            let counts = self.chip_counts();
            let chips = [
                (OverviewChip::All, "All", counts[0], None),
                (
                    OverviewChip::Running,
                    "Running",
                    counts[1],
                    Some(Light::Running),
                ),
                (
                    OverviewChip::Stopped,
                    "Stopped",
                    counts[2],
                    Some(Light::Stopped),
                ),
                (
                    OverviewChip::Attention,
                    "Needs a look",
                    counts[3],
                    Some(Light::Error),
                ),
                (
                    OverviewChip::Scheduled,
                    "Scheduled",
                    counts[4],
                    Some(Light::Scheduled),
                ),
            ];
            for (chip, label, count, light) in chips {
                if chip != OverviewChip::All && count == 0 && self.overview_chip != chip {
                    continue;
                }
                let n = count.to_string();
                if widgets::chip(ui, label, Some(&n), self.overview_chip == chip, light).clicked() {
                    self.overview_chip = chip;
                }
            }
        });
        page.add_space(10.0);

        // Selection bar.
        if !self.checked.is_empty() {
            let outer = page.available_rect_before_wrap();
            let bar = Rect::from_min_size(
                pos2(outer.left() + gutter, outer.top()),
                vec2(outer.width() - gutter * 2.0, 38.0),
            );
            page.allocate_exact_size(vec2(outer.width(), 46.0), Sense::hover());
            page.painter()
                .rect_filled(bar, CornerRadius::same(RADIUS), ACCENT);
            let mut bar_ui = page.new_child(
                egui::UiBuilder::new()
                    .max_rect(bar.shrink2(vec2(14.0, 0.0)))
                    .layout(Layout::left_to_right(Align::Center)),
            );
            bar_ui.spacing_mut().item_spacing.x = 8.0;
            bar_ui.label(
                egui::RichText::new(format!("{} selected", self.checked.len()))
                    .font(bold(13.0))
                    .color(ON_ACCENT),
            );
            bar_ui.add_space(6.0);
            for (label, action) in [
                ("Start", StackAction::Start),
                ("Stop", StackAction::Stop),
                ("Restart", StackAction::Restart),
            ] {
                if dark_button(&mut bar_ui, label).clicked() {
                    self.checked_action(action);
                }
            }
            if self.checked.len() > 1 && dark_button(&mut bar_ui, "Make a group").clicked() {
                self.group_checked_processes();
            }
            bar_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if dark_button(ui, "Clear").clicked() {
                    self.checked.clear();
                }
            });
        }

        // Table and stack log share the height that is left.
        let outer = page.available_rect_before_wrap();
        let show_log = tier >= Tier::Medium && outer.height() >= 460.0;
        let log_h = if show_log { 232.0 } else { 0.0 };
        let table = Rect::from_min_max(
            pos2(outer.left() + gutter, outer.top()),
            pos2(
                outer.right() - gutter,
                outer.bottom() - log_h - if show_log { 12.0 } else { 0.0 } - 14.0,
            ),
        );
        let actions = self.draw_table(&mut page, table);
        if show_log {
            let log = Rect::from_min_max(
                pos2(table.left(), table.bottom() + 12.0),
                pos2(table.right(), outer.bottom() - 14.0),
            );
            self.draw_stack_log(&mut page, log);
        }
        page.allocate_rect(outer, Sense::hover());

        for action in actions {
            match action {
                TableAction::Open(id) => self.navigate(Nav::Process(id)),
                TableAction::Toggle(id) => self.toggle_process(&id),
                TableAction::Restart(id) => self.restart_process(&id),
                TableAction::Check(id) => {
                    if let Some(i) = self.checked.iter().position(|c| *c == id) {
                        self.checked.remove(i);
                    } else {
                        self.checked.push(id);
                    }
                }
                TableAction::CheckGroup(ids, on) => {
                    self.checked.retain(|id| !ids.contains(id));
                    if on {
                        self.checked.extend(ids);
                    }
                }
                TableAction::Collapse(id) => {
                    if !self.overview_collapsed.remove(&id) {
                        self.overview_collapsed.insert(id);
                    }
                }
                TableAction::Group(id, action) => self.group_action(&id, action),
                TableAction::Rename(id) => {
                    let name = self
                        .config
                        .get_group(&id)
                        .map(|g| g.name.clone())
                        .unwrap_or_default();
                    self.rename_group = Some((id, name));
                }
                TableAction::Ungroup(id) => self.ungroup(&id),
            }
        }
    }

    fn chip_counts(&self) -> [usize; 5] {
        let mut counts = [self.config.processes.len(), 0, 0, 0, 0];
        for process in &self.config.processes {
            for (index, chip) in [
                OverviewChip::Running,
                OverviewChip::Stopped,
                OverviewChip::Attention,
                OverviewChip::Scheduled,
            ]
            .into_iter()
            .enumerate()
            {
                if self.chip_matches(process, chip) {
                    counts[index + 1] += 1;
                }
            }
        }
        counts
    }

    fn chip_matches(&self, process: &ProcessConfig, chip: OverviewChip) -> bool {
        let status = self.status_of(&process.id);
        let view = self.status_view(process);
        match chip {
            OverviewChip::All => true,
            OverviewChip::Running => status == ProcessStatus::Running,
            OverviewChip::Stopped => {
                status == ProcessStatus::Stopped && !view.alert && !process.scheduled_run.enabled
            }
            OverviewChip::Attention => view.alert,
            OverviewChip::Scheduled => process.scheduled_run.enabled,
        }
    }

    fn row_matches(&self, process: &ProcessConfig, group_name: &str) -> bool {
        if !self.chip_matches(process, self.overview_chip) {
            return false;
        }
        let words: Vec<String> = self
            .overview_filter
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        if words.is_empty() {
            return true;
        }
        let view = self.status_view(process);
        let hay = format!(
            "{} {} {} {} {}",
            process.name,
            process.command,
            group_name,
            view.word,
            format::flags(process).join(" ")
        )
        .to_lowercase();
        words.iter().all(|w| hay.contains(w.as_str()))
    }

    fn draw_table(&mut self, page: &mut Ui, table: Rect) -> Vec<TableAction> {
        let mut actions = Vec::new();
        let painter = page.painter().clone();
        painter.rect(
            table,
            CornerRadius::same(RADIUS_CARD),
            BG,
            Stroke::new(1.0_f32, LINE),
            StrokeKind::Inside,
        );
        let columns = Columns::for_width(table.width());
        let header =
            Rect::from_min_size(table.min + vec2(1.0, 1.0), vec2(table.width() - 2.0, 30.0));
        painter.rect_filled(
            header,
            CornerRadius {
                nw: RADIUS_CARD - 1,
                ne: RADIUS_CARD - 1,
                sw: 0,
                se: 0,
            },
            PANEL,
        );
        painter.hline(
            header.x_range(),
            header.bottom(),
            Stroke::new(1.0_f32, LINE),
        );
        let layout = column_layout(header.left(), header.right(), columns);
        for (label, x) in [
            ("Name", Some(layout.name)),
            ("Status", Some(layout.status)),
            ("CPU", layout.cpu),
            ("Memory", layout.memory),
            ("Uptime", layout.uptime),
            ("Behaviour", layout.flags),
            ("Command", layout.command),
        ] {
            if let Some(x) = x {
                let mut job = egui::text::LayoutJob::default();
                job.append(
                    &label.to_uppercase(),
                    0.0,
                    egui::TextFormat {
                        font_id: bold(10.5),
                        color: FAINT,
                        extra_letter_spacing: 0.9,
                        ..Default::default()
                    },
                );
                let g = page.fonts_mut(|f| f.layout_job(job));
                widgets::paint_galley_left(&painter, pos2(x, header.center().y), g, FAINT);
            }
        }

        let body_rect = Rect::from_min_max(
            pos2(table.left() + 1.0, header.bottom() + 1.0),
            pos2(table.right() - 1.0, table.bottom() - 1.0),
        );
        let mut body_ui = page.new_child(
            egui::UiBuilder::new()
                .max_rect(body_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        body_ui.set_clip_rect(body_rect);

        // Groups in sidebar order, then the loose processes.
        let mut sections: Vec<(String, String, Vec<ProcessConfig>, bool)> = Vec::new();
        for item in self.sidebar_items() {
            if let super::SidebarRenderItem::Group(group) = item {
                if self
                    .overview_group
                    .as_ref()
                    .is_some_and(|only| *only != group.id)
                {
                    continue;
                }
                let members = group
                    .process_ids
                    .iter()
                    .filter_map(|id| self.process_config(id))
                    .collect();
                sections.push((group.id.clone(), group.name.clone(), members, true));
            }
        }
        if self.overview_group.is_none() {
            let loose: Vec<ProcessConfig> = self
                .config
                .processes
                .iter()
                .filter(|p| self.config.group_for_process(&p.id).is_none())
                .filter_map(|p| self.process_config(&p.id))
                .collect();
            if !loose.is_empty() {
                sections.push((UNGROUPED.to_string(), "Ungrouped".to_string(), loose, false));
            }
        }

        let mut shown = 0;
        ScrollArea::vertical()
            .id_salt("overview_table")
            .auto_shrink([false, false])
            .show(&mut body_ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (id, name, members, real) in &sections {
                    let rows: Vec<&ProcessConfig> = members
                        .iter()
                        .filter(|p| self.row_matches(p, name))
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    shown += rows.len();
                    let collapsed = self.overview_collapsed.contains(id);
                    self.group_row(ui, id, name, &rows, *real, collapsed, &mut actions);
                    if collapsed {
                        continue;
                    }
                    for process in rows {
                        self.table_row(ui, process, columns, &mut actions);
                    }
                }
                if shown == 0 {
                    ui.add_space(28.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            egui::RichText::new("No process matches.")
                                .font(body(13.0))
                                .color(MUTED),
                        );
                        ui.add_space(8.0);
                        if Btn::new("Clear the filter").small().show(ui).clicked() {
                            self.overview_filter.clear();
                            self.overview_chip = OverviewChip::All;
                        }
                    });
                }
            });
        actions
    }

    #[allow(clippy::too_many_arguments)]
    fn group_row(
        &self,
        ui: &mut Ui,
        id: &str,
        name: &str,
        rows: &[&ProcessConfig],
        real: bool,
        collapsed: bool,
        actions: &mut Vec<TableAction>,
    ) {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::ZERO, PANEL);
        painter.hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0_f32, LINE),
        );
        let ids: Vec<String> = rows.iter().map(|p| p.id.clone()).collect();
        let all_checked = ids.iter().all(|id| self.is_checked(id));
        let check_rect = Rect::from_center_size(
            pos2(rect.left() + 12.0 + 7.0, rect.center().y),
            vec2(14.0, 14.0),
        );
        if checkbox(
            ui,
            check_rect,
            ui.id().with(("group_check", id)),
            all_checked,
        )
        .clicked()
        {
            actions.push(TableAction::CheckGroup(ids.clone(), !all_checked));
        }
        let toggle_rect = Rect::from_min_max(
            pos2(rect.left() + 34.0, rect.top()),
            pos2(rect.left() + 34.0 + 260.0, rect.bottom()),
        );
        let toggle = ui.interact(
            toggle_rect,
            ui.id().with(("group_toggle", id)),
            Sense::click(),
        );
        icons::paint(
            &painter,
            icons::square(pos2(toggle_rect.left() + 7.0, rect.center().y), 13.0),
            13.0,
            if collapsed {
                Icon::CaretRight
            } else {
                Icon::CaretDown
            },
            FAINT,
        );
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &name.to_uppercase(),
            0.0,
            egui::TextFormat {
                font_id: bold(11.0),
                color: TEXT,
                extra_letter_spacing: 1.0,
                ..Default::default()
            },
        );
        let g = ui.fonts_mut(|f| f.layout_job(job));
        let gw = g.size().x;
        widgets::paint_galley_left(
            &painter,
            pos2(toggle_rect.left() + 22.0, rect.center().y),
            g,
            TEXT,
        );
        let count = widgets::galley(ui, rows.len().to_string(), body(11.5), FAINT);
        let cw = count.size().x;
        widgets::paint_galley_left(
            &painter,
            pos2(toggle_rect.left() + 22.0 + gw + 8.0, rect.center().y),
            count,
            FAINT,
        );
        if toggle.clicked() {
            actions.push(TableAction::Collapse(id.to_string()));
        }
        let ids_ref: Vec<String> = rows.iter().map(|p| p.id.clone()).collect();
        let r = self.rollup(ids_ref.iter());
        let mut roll = vec![format!("{} running", r.running)];
        if r.attention > 0 {
            roll.push(format!("{} need a look", r.attention));
        }
        if r.memory > 0 {
            roll.push(format::format_memory(r.memory));
        }
        let roll_x = toggle_rect.left() + 22.0 + gw + 8.0 + cw + 12.0;
        let actions_w = if real { 3.0 * 64.0 + 30.0 } else { 0.0 };
        let roll_g = widgets::galley_truncated(
            ui,
            &roll.join(" · "),
            body(11.5),
            FAINT,
            (rect.right() - 12.0 - actions_w - roll_x).max(0.0),
        );
        widgets::paint_galley_left(&painter, pos2(roll_x, rect.center().y), roll_g, FAINT);

        if real {
            let mut right = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect.shrink2(vec2(8.0, 0.0)))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            right.spacing_mut().item_spacing.x = 4.0;
            let more = Btn::icon(Icon::More).ghost().xsmall().show(&mut right);
            Popup::menu(&more)
                .frame(widgets::popup_frame())
                .align(RectAlign::BOTTOM_END)
                .gap(4.0)
                .show(|ui| {
                    ui.set_min_width(180.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if widgets::menu_item(ui, "Rename…", None, false, true).clicked() {
                        actions.push(TableAction::Rename(id.to_string()));
                        ui.close();
                    }
                    if widgets::menu_item(ui, "Ungroup", None, false, true).clicked() {
                        actions.push(TableAction::Ungroup(id.to_string()));
                        ui.close();
                    }
                });
            for (label, action, tip) in [
                (
                    "Restart",
                    StackAction::Restart,
                    "Restart the members that answer Restart all",
                ),
                (
                    "Stop",
                    StackAction::Stop,
                    "Stop the members that answer Stop all",
                ),
                (
                    "Start",
                    StackAction::Start,
                    "Start the members that answer Start all",
                ),
            ] {
                if Btn::new(label)
                    .ghost()
                    .xsmall()
                    .show(&mut right)
                    .on_hover_text(tip)
                    .clicked()
                {
                    actions.push(TableAction::Group(id.to_string(), action));
                }
            }
        }
    }

    fn table_row(
        &self,
        ui: &mut Ui,
        process: &ProcessConfig,
        columns: Columns,
        actions: &mut Vec<TableAction>,
    ) {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
        let painter = ui.painter().clone();
        let view = self.status_view(process);
        let checked = self.is_checked(&process.id);
        let status = self.status_of(&process.id);
        let stopped_quiet =
            status == ProcessStatus::Stopped && !view.alert && !process.scheduled_run.enabled;
        if checked {
            painter.rect_filled(rect, CornerRadius::ZERO, theme::accent_soft());
        } else if view.alert {
            painter.rect_filled(rect, CornerRadius::ZERO, theme::red_soft());
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::ZERO, HOVER);
        }
        painter.hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            Stroke::new(1.0_f32, ROW_LINE),
        );
        let dim = |c: egui::Color32| {
            if stopped_quiet {
                c.gamma_multiply(0.72)
            } else {
                c
            }
        };
        let layout = column_layout(rect.left(), rect.right(), columns);
        let cy = rect.center().y;

        let check_rect =
            Rect::from_center_size(pos2(rect.left() + 12.0 + 7.0, cy), vec2(14.0, 14.0));
        if checkbox(
            ui,
            check_rect,
            ui.id().with(("check", &process.id)),
            checked,
        )
        .clicked()
        {
            actions.push(TableAction::Check(process.id.clone()));
        }

        // Name: a link to the console.
        let name_rect = Rect::from_min_max(
            pos2(layout.name, rect.top()),
            pos2(layout.status - 12.0, rect.bottom()),
        );
        let name_g =
            widgets::galley_truncated(ui, &process.name, bold(13.0), TEXT, name_rect.width());
        let name_hit = Rect::from_min_size(
            pos2(name_rect.left(), cy - 10.0),
            vec2(name_g.size().x, 20.0),
        );
        let name_response = ui.interact(
            name_hit,
            ui.id().with(("open", &process.id)),
            Sense::click(),
        );
        widgets::paint_galley_left(&painter, pos2(name_rect.left(), cy), name_g, dim(TEXT));
        if name_response.hovered() {
            painter.hline(name_hit.x_range(), cy + 8.5, Stroke::new(1.0_f32, MUTED));
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if name_response.clicked() {
            actions.push(TableAction::Open(process.id.clone()));
        }

        // Status.
        let status_end = layout
            .cpu
            .or(layout.actions_left())
            .unwrap_or(rect.right() - 70.0)
            - 12.0;
        widgets::paint_light(ui, &painter, pos2(layout.status + 4.0, cy), view.light);
        widgets::text_in(
            ui,
            Rect::from_min_max(
                pos2(layout.status + 16.0, rect.top()),
                pos2(status_end, rect.bottom()),
            ),
            &view.word,
            bold(12.5),
            dim(tone_color(view.tone)),
        );

        let docker = process.process_type == ProcessType::Docker;
        let running = status == ProcessStatus::Running;
        let usage = self.usage_of(&process.id).unwrap_or_default();
        if let Some(x) = layout.cpu {
            if running && !docker {
                let cpu = usage.cpu_percent.unwrap_or(0.0);
                let bar = Rect::from_min_size(pos2(x, cy - 2.5), vec2(46.0, 5.0));
                painter.rect_filled(bar, CornerRadius::same(3), PANEL2);
                let fill_w = (cpu / 25.0).clamp(0.02, 1.0) * bar.width();
                painter.rect_filled(
                    Rect::from_min_size(bar.min, vec2(fill_w, bar.height())),
                    CornerRadius::same(3),
                    ACCENT,
                );
                painter.text(
                    pos2(x + 54.0, cy),
                    Align2::LEFT_CENTER,
                    format::format_cpu(cpu),
                    mono(11.5),
                    TEXT,
                );
            } else {
                painter.text(pos2(x, cy), Align2::LEFT_CENTER, "—", mono(11.5), FAINT);
            }
        }
        if let Some(x) = layout.memory {
            let text = if running && !docker {
                format::format_memory(usage.memory_bytes.unwrap_or(0))
            } else {
                "—".into()
            };
            painter.text(
                pos2(x, cy),
                Align2::LEFT_CENTER,
                text,
                mono(11.5),
                if running { TEXT } else { FAINT },
            );
        }
        if let Some(x) = layout.uptime {
            let text = if running {
                format::uptime_seconds(self.info_of(&process.id))
                    .map(format::duration_text)
                    .unwrap_or_else(|| "—".into())
            } else {
                "—".into()
            };
            painter.text(pos2(x, cy), Align2::LEFT_CENTER, text, mono(11.5), MUTED);
        }
        if let (Some(x), Some(end)) = (layout.flags, layout.command.or(layout.actions_left())) {
            let mut fx = x;
            for flag in format::flags(process) {
                let g = widgets::galley(ui, &flag, bold(10.5), MUTED);
                let w = g.size().x + 12.0;
                if fx + w > end - 12.0 {
                    break;
                }
                let r = Rect::from_min_size(pos2(fx, cy - 9.5), vec2(w, 19.0));
                painter.rect(
                    r,
                    CornerRadius::same(RADIUS),
                    PANEL,
                    Stroke::new(1.0_f32, LINE),
                    StrokeKind::Inside,
                );
                painter.galley(r.center() - g.size() * 0.5, g, MUTED);
                fx += w + 4.0;
            }
        }
        if let (Some(x), Some(end)) = (layout.command, layout.actions_left()) {
            widgets::text_in(
                ui,
                Rect::from_min_max(pos2(x, rect.top()), pos2(end - 12.0, rect.bottom())),
                &process.command,
                mono(11.5),
                MUTED,
            );
        }

        // Actions.
        let live = self.is_live(&process.id);
        let restart_rect =
            Rect::from_center_size(pos2(rect.right() - 12.0 - 12.0, cy), vec2(24.0, 22.0));
        let toggle_rect =
            Rect::from_center_size(pos2(restart_rect.left() - 4.0 - 12.0, cy), vec2(24.0, 22.0));
        let toggle = Btn::icon(if live { Icon::Stop } else { Icon::Play })
            .xsmall()
            .show_in(ui, toggle_rect, ui.id().with(("row_toggle", &process.id)));
        if toggle
            .on_hover_text(if live {
                format!("Stop {}", process.name)
            } else {
                format!("Start {}", process.name)
            })
            .clicked()
        {
            actions.push(TableAction::Toggle(process.id.clone()));
        }
        let restart = Btn::icon(Icon::Restart).xsmall().show_in(
            ui,
            restart_rect,
            ui.id().with(("row_restart", &process.id)),
        );
        if restart
            .on_hover_text(format!("Restart {}", process.name))
            .clicked()
        {
            actions.push(TableAction::Restart(process.id.clone()));
        }
    }

    fn draw_stack_log(&mut self, page: &mut Ui, rect: Rect) {
        let painter = page.painter().clone();
        painter.rect(
            rect,
            CornerRadius::same(RADIUS_CARD),
            LOG_BG,
            Stroke::new(1.0_f32, LINE),
            StrokeKind::Inside,
        );
        let head = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(rect.width() - 2.0, 33.0));
        painter.rect_filled(
            head,
            CornerRadius {
                nw: RADIUS_CARD - 1,
                ne: RADIUS_CARD - 1,
                sw: 0,
                se: 0,
            },
            PANEL,
        );
        painter.hline(head.x_range(), head.bottom(), Stroke::new(1.0_f32, LINE));
        let mut job = egui::text::LayoutJob::default();
        job.append(
            "STACK LOG",
            0.0,
            egui::TextFormat {
                font_id: bold(10.5),
                color: FAINT,
                extra_letter_spacing: 0.9,
                ..Default::default()
            },
        );
        let g = page.fonts_mut(|f| f.layout_job(job));
        let gw = g.size().x;
        widgets::paint_galley_left(
            &painter,
            pos2(head.left() + 12.0, head.center().y),
            g,
            FAINT,
        );
        let hint = widgets::galley(
            page,
            "every process, newest last · click a name to open its log",
            body(11.5),
            FAINT,
        );
        widgets::paint_galley_left(
            &painter,
            pos2(head.left() + 12.0 + gw + 10.0, head.center().y),
            hint,
            FAINT,
        );

        let body_rect = Rect::from_min_max(
            pos2(rect.left() + 1.0, head.bottom() + 1.0),
            pos2(rect.right() - 1.0, rect.bottom() - 1.0),
        );
        let mut body_ui = page.new_child(
            egui::UiBuilder::new()
                .max_rect(body_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        body_ui.set_clip_rect(body_rect);
        let lines = &self.runtime_snapshot.stack_logs;
        let names: std::collections::HashMap<&str, &str> = self
            .config
            .processes
            .iter()
            .map(|p| (p.id.as_str(), p.name.as_str()))
            .collect();
        let mut open = None;
        ScrollArea::vertical()
            .id_salt("stack_log")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show_rows(&mut body_ui, 19.0, lines.len(), |ui, range| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                for (id, line) in &lines[range] {
                    let (row, _) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 19.0), Sense::hover());
                    let painter = ui.painter();
                    let (_, kind, text) = format::classify(&line.text);
                    painter.text(
                        pos2(row.left() + 12.0, row.center().y),
                        Align2::LEFT_CENTER,
                        line.at.format("%H:%M:%S").to_string(),
                        mono(11.0),
                        FAINT,
                    );
                    let name = names.get(id.as_str()).copied().unwrap_or("?");
                    let who = Rect::from_min_size(
                        pos2(row.left() + 12.0 + 76.0, row.top()),
                        vec2(120.0, row.height()),
                    );
                    let response = ui.interact(
                        who,
                        ui.id().with(("who", id, line.at.timestamp_nanos_opt())),
                        Sense::click(),
                    );
                    widgets::text_in(
                        ui,
                        who.shrink2(vec2(0.0, 0.0)),
                        name,
                        egui::FontId::monospace(11.5),
                        if response.hovered() { TEXT } else { MUTED },
                    );
                    if response.clicked() {
                        open = Some(id.clone());
                    }
                    let color = match kind {
                        LineKind::Error => LOG_ERROR_TEXT,
                        LineKind::Warning => AMBER,
                        LineKind::Event => ACCENT,
                        LineKind::Normal => LOG_TEXT,
                    };
                    let text_rect = Rect::from_min_max(
                        pos2(who.right() + 8.0, row.top()),
                        pos2(row.right() - 12.0, row.bottom()),
                    );
                    widgets::text_in(ui, text_rect, text, mono(11.5), color);
                }
            });
        if let Some(id) = open {
            self.navigate(Nav::Process(id));
        }
    }
}

struct ColumnLayout {
    name: f32,
    status: f32,
    cpu: Option<f32>,
    memory: Option<f32>,
    uptime: Option<f32>,
    flags: Option<f32>,
    command: Option<f32>,
    right: f32,
}

impl ColumnLayout {
    fn actions_left(&self) -> Option<f32> {
        Some(self.right - 12.0 - 52.0 - 12.0)
    }
}

/// Where each column starts. Name and status share the flexible space; the numbers are fixed.
fn column_layout(left: f32, right: f32, columns: Columns) -> ColumnLayout {
    let actions_w = 12.0 + 52.0 + 12.0;
    let mut fixed = 0.0;
    if columns.resources {
        fixed += 118.0;
    }
    if columns.memory {
        fixed += 90.0;
    }
    if columns.uptime {
        fixed += 90.0;
    }
    let start = left + 34.0;
    let flexible = (right - actions_w - start - fixed).max(200.0);
    let (name_w, status_w, flags_w, command_w) = match (columns.flags, columns.command) {
        (true, true) => (
            flexible * 0.18,
            flexible * 0.26,
            flexible * 0.24,
            flexible * 0.32,
        ),
        (true, false) => (flexible * 0.28, flexible * 0.38, flexible * 0.34, 0.0),
        _ => (flexible * 0.42, flexible * 0.58, 0.0, 0.0),
    };
    let mut x = start;
    let name = x;
    x += name_w;
    let status = x;
    x += status_w;
    let cpu = columns.resources.then(|| {
        let at = x;
        x += 118.0;
        at
    });
    let memory = columns.memory.then(|| {
        let at = x;
        x += 90.0;
        at
    });
    let uptime = columns.uptime.then(|| {
        let at = x;
        x += 90.0;
        at
    });
    let flags = columns.flags.then(|| {
        let at = x;
        x += flags_w;
        at
    });
    let command = columns.command.then(|| {
        let at = x;
        x += command_w;
        at
    });
    ColumnLayout {
        name,
        status,
        cpu,
        memory,
        uptime,
        flags,
        command,
        right,
    }
}

/// A 14 pt checkbox painted in the kit's colours.
fn checkbox(ui: &mut Ui, rect: Rect, id: egui::Id, checked: bool) -> egui::Response {
    let response = ui.interact(rect.expand(4.0), id, Sense::click());
    let painter = ui.painter();
    let radius = CornerRadius::same(3);
    if checked {
        painter.rect_filled(rect, radius, ACCENT);
        icons::paint(painter, rect, 12.0, Icon::Check, ON_ACCENT);
    } else {
        let border = if response.hovered() {
            BORDER_HI
        } else {
            BORDER
        };
        painter.rect(
            rect,
            radius,
            BG,
            Stroke::new(1.0_f32, border),
            StrokeKind::Inside,
        );
    }
    response
}

/// A button on the accent selection bar: dark, with the accent as its text.
fn dark_button(ui: &mut Ui, label: &str) -> egui::Response {
    let g = widgets::galley(ui, label, bold(12.0), ACCENT);
    let (rect, response) = ui.allocate_exact_size(
        vec2(g.size().x + 18.0, SMALL_CONTROL_HEIGHT),
        Sense::click(),
    );
    let fill = if response.hovered() {
        theme::PANEL2
    } else {
        ON_ACCENT
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), fill);
    ui.painter()
        .galley(rect.center() - g.size() * 0.5, g, ACCENT);
    response
}
