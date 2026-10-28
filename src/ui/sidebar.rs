//! The sidebar: Overview, then the processes under their groups, then the stack's totals.
//! Rows drag to reorder (a line shows where they land) and have a context menu.

use eframe::egui::{self, pos2, vec2, CornerRadius, Rect, Sense, Stroke, Ui};

use super::icons::{self, Icon};
use super::theme::*;
use super::widgets::{self, Btn, Light};
use super::{
    format, Nav, ProcessManagerApp, ProcessTab, SidebarDragItem, SidebarDropSlot,
    SidebarDropTarget, SidebarRenderItem, StackAction, View,
};

enum RowAction {
    Select(String),
    Extend(String),
    Toggle(String),
    StartStop(String),
    Restart(String),
    Edit(String),
    Duplicate(String),
    MoveUp(String),
    MoveDown(String),
    Reload(String),
    Delete(String),
    MakeGroup,
    ToggleGroup(String),
    OpenGroup(String),
    RenameGroup(String),
    GroupAction(String, StackAction),
    Ungroup(String),
}

impl ProcessManagerApp {
    pub(super) fn draw_sidebar(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let footer_h = 58.0;
        let list_rect = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - footer_h));
        let footer_rect = Rect::from_min_max(pos2(full.left(), full.bottom() - footer_h), full.max);

        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        self.draw_sidebar_list(&mut list_ui);

        let painter = ui.painter();
        painter.hline(
            footer_rect.x_range(),
            footer_rect.top(),
            Stroke::new(1.0_f32, LINE),
        );
        let rollup = self.stack_rollup();
        let rows = [
            ("Stack CPU", format::format_cpu(rollup.cpu)),
            ("Stack memory", format::format_memory(rollup.memory)),
        ];
        for (index, (label, value)) in rows.into_iter().enumerate() {
            let y = footer_rect.top() + 18.0 + index as f32 * 22.0;
            let job = caps_job(label);
            let g = ui.fonts_mut(|f| f.layout_job(job));
            widgets::paint_galley_left(painter, pos2(footer_rect.left() + 16.0, y), g, FAINT);
            let v = widgets::galley(ui, &value, mono(11.5), TEXT);
            widgets::paint_galley_left(
                painter,
                pos2(footer_rect.right() - 16.0 - v.size().x, y),
                v,
                TEXT,
            );
        }
        ui.allocate_rect(full, Sense::hover());
    }

    fn draw_sidebar_list(&mut self, ui: &mut Ui) {
        let mut actions: Vec<RowAction> = Vec::new();
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 10,
                bottom: 4,
            })
            .show(ui, |ui| {
                let on = self.view == View::Overview;
                if self
                    .nav_row(
                        ui,
                        Icon::Layers,
                        "Overview",
                        on,
                        Some(self.config.processes.len().to_string()),
                    )
                    .clicked()
                {
                    self.overview_group = None;
                    self.navigate(Nav::Overview);
                }
            });

        // "PROCESSES" and the add button.
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            ui.add_space(18.0);
            ui.label(widgets::caps("Processes"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if Btn::icon(Icon::Plus)
                    .ghost()
                    .small()
                    .show(ui)
                    .on_hover_text("Add a process · Ctrl N")
                    .clicked()
                {
                    self.open_new_process();
                }
            });
        });

        if self.config.processes.is_empty() {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("No processes yet")
                        .font(bold(13.0))
                        .color(TEXT),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("Add one with the + above.")
                        .font(body(12.0))
                        .color(MUTED),
                );
            });
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 0,
                        bottom: 10,
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        self.draw_sidebar_items(ui, &mut actions);
                    });
            });

        for action in actions {
            self.apply_row_action(action);
        }
    }

    fn draw_sidebar_items(&mut self, ui: &mut Ui, actions: &mut Vec<RowAction>) {
        let mut drop_slots: Vec<SidebarDropSlot> = Vec::new();
        let mut last_row_rect: Option<Rect> = None;
        let mut previous_was_grouped = false;
        let items = self.sidebar_items();
        let checked_count = self.checked.len();

        for item in items {
            match item {
                SidebarRenderItem::Group(group) => {
                    ui.add_space(6.0);
                    let response = self.group_header(ui, &group);
                    if let Some(first_member_id) = group.process_ids.first().cloned() {
                        drop_slots.push(SidebarDropSlot {
                            y: response.rect.top(),
                            left: response.rect.left() + 8.0,
                            right: response.rect.right() - 8.0,
                            target: SidebarDropTarget::TopLevel {
                                before_process_id: Some(first_member_id),
                            },
                        });
                    }
                    drop_slots.push(SidebarDropSlot {
                        y: response.rect.bottom(),
                        left: response.rect.left() + 26.0,
                        right: response.rect.right() - 8.0,
                        target: SidebarDropTarget::InGroup {
                            group_id: group.id.clone(),
                            before_process_id: group.process_ids.first().cloned(),
                        },
                    });
                    let id = group.id.clone();
                    response.context_menu(|ui| {
                        menu(ui, |ui| {
                            let expand = if group.expanded { "Collapse" } else { "Expand" };
                            if widgets::menu_item(ui, expand, None, false, true).clicked() {
                                actions.push(RowAction::ToggleGroup(id.clone()));
                                ui.close();
                            }
                            if widgets::menu_item(ui, "Open in Overview", None, false, true)
                                .clicked()
                            {
                                actions.push(RowAction::OpenGroup(id.clone()));
                                ui.close();
                            }
                            widgets::menu_separator(ui);
                            for (label, action) in [
                                ("Start group", StackAction::Start),
                                ("Stop group", StackAction::Stop),
                                ("Restart group", StackAction::Restart),
                            ] {
                                if widgets::menu_item(ui, label, None, false, true).clicked() {
                                    actions.push(RowAction::GroupAction(id.clone(), action));
                                    ui.close();
                                }
                            }
                            widgets::menu_separator(ui);
                            if widgets::menu_item(ui, "Rename…", None, false, true).clicked() {
                                actions.push(RowAction::RenameGroup(id.clone()));
                                ui.close();
                            }
                            if widgets::menu_item(ui, "Ungroup", None, false, true).clicked() {
                                actions.push(RowAction::Ungroup(id.clone()));
                                ui.close();
                            }
                        });
                    });
                    if response.drag_started() {
                        self.dragged_sidebar_item = Some(SidebarDragItem::Group(group.id.clone()));
                    } else if response.clicked() {
                        actions.push(RowAction::ToggleGroup(group.id.clone()));
                    }
                    last_row_rect = Some(response.rect);
                    previous_was_grouped = true;
                }
                SidebarRenderItem::Process {
                    process,
                    group_id,
                    last_in_group,
                } => {
                    if group_id.is_none() && previous_was_grouped {
                        // A quiet line between the groups and the loose processes.
                        let (rect, _) = ui
                            .allocate_exact_size(vec2(ui.available_width(), 17.0), Sense::hover());
                        ui.painter().hline(
                            rect.x_range().shrink(10.0),
                            rect.center().y + 2.0,
                            Stroke::new(1.0_f32, LINE),
                        );
                        previous_was_grouped = false;
                    }
                    let process = self.process_config(&process.id).unwrap_or(process);
                    let view = self.status_view(&process);
                    let on = (self.view == View::Process
                        && self.selected_process.as_deref() == Some(process.id.as_str()))
                        || self.is_checked(&process.id);
                    let flash = self.process_row_flash_intensity(ui.ctx(), &process.id);
                    let response = self.process_row(
                        ui,
                        &process.name,
                        view.light,
                        &view.short,
                        view.alert,
                        on,
                        group_id.is_some(),
                        flash,
                    );
                    let response =
                        response.on_hover_text(format!("{} · {}", process.name, view.word));
                    if response.drag_started() {
                        self.dragged_sidebar_item =
                            Some(SidebarDragItem::Process(process.id.clone()));
                    }
                    if let Some(group_id) = group_id.clone() {
                        drop_slots.push(SidebarDropSlot {
                            y: response.rect.top(),
                            left: response.rect.left() + 26.0,
                            right: response.rect.right() - 8.0,
                            target: SidebarDropTarget::InGroup {
                                group_id: group_id.clone(),
                                before_process_id: Some(process.id.clone()),
                            },
                        });
                        if last_in_group {
                            drop_slots.push(SidebarDropSlot {
                                y: response.rect.bottom(),
                                left: response.rect.left() + 26.0,
                                right: response.rect.right() - 8.0,
                                target: SidebarDropTarget::InGroup {
                                    group_id,
                                    before_process_id: None,
                                },
                            });
                        }
                    } else {
                        drop_slots.push(SidebarDropSlot {
                            y: response.rect.top(),
                            left: response.rect.left() + 8.0,
                            right: response.rect.right() - 8.0,
                            target: SidebarDropTarget::TopLevel {
                                before_process_id: Some(process.id.clone()),
                            },
                        });
                    }
                    let id = process.id.clone();
                    let live = self.is_live(&id);
                    response.context_menu(|ui| {
                        menu(ui, |ui| {
                            let mut item =
                                |ui: &mut Ui,
                                 label: &str,
                                 kbd: Option<&str>,
                                 danger: bool,
                                 action: RowAction| {
                                    if widgets::menu_item(ui, label, kbd, danger, true).clicked() {
                                        actions.push(action);
                                        ui.close();
                                    }
                                };
                            item(
                                ui,
                                if live { "Stop" } else { "Start" },
                                Some("Ctrl Enter"),
                                false,
                                RowAction::StartStop(id.clone()),
                            );
                            item(
                                ui,
                                "Restart",
                                Some("Ctrl R"),
                                false,
                                RowAction::Restart(id.clone()),
                            );
                            item(
                                ui,
                                "Edit settings",
                                Some("Ctrl ,"),
                                false,
                                RowAction::Edit(id.clone()),
                            );
                            widgets::menu_separator(ui);
                            item(
                                ui,
                                "Duplicate",
                                None,
                                false,
                                RowAction::Duplicate(id.clone()),
                            );
                            if checked_count > 1 {
                                item(
                                    ui,
                                    "Make a group of the selection",
                                    None,
                                    false,
                                    RowAction::MakeGroup,
                                );
                            }
                            item(ui, "Move up", None, false, RowAction::MoveUp(id.clone()));
                            item(
                                ui,
                                "Move down",
                                None,
                                false,
                                RowAction::MoveDown(id.clone()),
                            );
                            item(
                                ui,
                                "Reload from processes.json",
                                None,
                                false,
                                RowAction::Reload(id.clone()),
                            );
                            widgets::menu_separator(ui);
                            item(ui, "Delete…", None, true, RowAction::Delete(id.clone()));
                        });
                    });
                    if response.clicked() {
                        let modifiers = ui.input(|input| input.modifiers);
                        actions.push(if modifiers.shift {
                            RowAction::Extend(process.id.clone())
                        } else if modifiers.command {
                            RowAction::Toggle(process.id.clone())
                        } else {
                            RowAction::Select(process.id.clone())
                        });
                    }
                    last_row_rect = Some(response.rect);
                    if group_id.is_none() {
                        previous_was_grouped = false;
                    }
                }
            }
        }

        if let Some(last_rect) = last_row_rect {
            drop_slots.push(SidebarDropSlot {
                y: last_rect.bottom(),
                left: last_rect.left() + 8.0,
                right: last_rect.right() - 8.0,
                target: SidebarDropTarget::TopLevel {
                    before_process_id: None,
                },
            });
        }

        let ctx = ui.ctx().clone();
        let active_drop = self.dragged_sidebar_item.as_ref().and_then(|dragged| {
            ctx.input(|input| {
                input.pointer.interact_pos().and_then(|pointer_pos| {
                    self.nearest_sidebar_drop_slot(dragged, &drop_slots, pointer_pos)
                })
            })
        });

        if self.dragged_sidebar_item.is_some() && ctx.input(|input| input.pointer.primary_down()) {
            if let Some(slot) = active_drop.as_ref() {
                ui.painter().line_segment(
                    [pos2(slot.left, slot.y), pos2(slot.right, slot.y)],
                    Stroke::new(2.0_f32, ACCENT),
                );
                ui.painter()
                    .circle_filled(pos2(slot.left, slot.y), 3.0, ACCENT);
            }
        }

        if ctx.input(|input| input.pointer.any_released()) {
            if let (Some(dragged), Some(target)) = (self.dragged_sidebar_item.clone(), active_drop)
            {
                if ctx.input(|i| {
                    i.pointer
                        .press_origin()
                        .zip(i.pointer.interact_pos())
                        .is_some_and(|(a, b)| a.distance(b) > 4.0)
                }) {
                    self.apply_sidebar_drop(dragged, target.target);
                }
            }
            self.dragged_sidebar_item = None;
        }
    }

    fn group_header(
        &self,
        ui: &mut Ui,
        group: &crate::config::ProcessGroupConfig,
    ) -> egui::Response {
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click_and_drag());
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(RADIUS), HOVER);
        }
        let caret = if group.expanded {
            Icon::CaretDown
        } else {
            Icon::CaretRight
        };
        icons::paint(
            painter,
            icons::square(pos2(rect.left() + 13.0, rect.center().y), 13.0),
            13.0,
            caret,
            FAINT,
        );
        let job = {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                &group.name.to_uppercase(),
                0.0,
                egui::TextFormat {
                    font_id: bold(11.0),
                    color: TEXT,
                    extra_letter_spacing: 1.0,
                    ..Default::default()
                },
            );
            job
        };
        let rollup = self.rollup(group.process_ids.iter());
        let meta = if rollup.memory > 0 {
            format!(
                "{} · {}",
                format::format_cpu(rollup.cpu),
                format::format_memory(rollup.memory)
            )
        } else {
            String::new()
        };
        let meta_g = widgets::galley(ui, &meta, mono(10.5), FAINT);
        let name = ui.fonts_mut(|f| f.layout_job(job));
        let mut x = rect.left() + 24.0;
        // The name comes first; the count and the CPU and memory note give way to it.
        let name_w = name.size().x.min(rect.width() - 24.0 - 50.0).max(20.0);
        painter
            .with_clip_rect(Rect::from_min_size(
                pos2(x, rect.top()),
                vec2(name_w, rect.height()),
            ))
            .galley(
                pos2(x, rect.center().y - name.size().y * 0.5),
                name.clone(),
                TEXT,
            );
        x += name_w.min(name.size().x) + 7.0;
        let count = widgets::galley(
            ui,
            format!("{}/{}", rollup.running, rollup.total),
            body(11.5),
            FAINT,
        );
        let count_w = count.size().x;
        widgets::paint_galley_left(painter, pos2(x, rect.center().y), count, FAINT);
        x += count_w + 7.0;
        if !group.expanded && rollup.attention > 0 {
            widgets::paint_light(ui, painter, pos2(x + 4.0, rect.center().y), Light::Error);
        }
        let meta_left = rect.right() - 10.0 - meta_g.size().x;
        if !meta.is_empty() && meta_left > x + 12.0 {
            widgets::paint_galley_left(painter, pos2(meta_left, rect.center().y), meta_g, FAINT);
        }
        response.on_hover_text(format!(
            "{}: {} of {} running. Click to {}; right-click for group actions.",
            group.name,
            rollup.running,
            rollup.total,
            if group.expanded { "collapse" } else { "expand" }
        ))
    }

    fn apply_row_action(&mut self, action: RowAction) {
        match action {
            RowAction::Select(id) => self.select_process(id),
            RowAction::Extend(id) => self.extend_selection_to(id),
            RowAction::Toggle(id) => self.toggle_checked(&id),
            RowAction::StartStop(id) => self.toggle_process(&id),
            RowAction::Restart(id) => self.restart_process(&id),
            RowAction::Edit(id) => {
                self.navigate(Nav::Process(id));
                self.set_tab(ProcessTab::Settings);
            }
            RowAction::Duplicate(id) => self.duplicate_process(&id),
            RowAction::MoveUp(id) => self.move_process_up(&id),
            RowAction::MoveDown(id) => self.move_process_down(&id),
            RowAction::Reload(id) => self.reload_process_from_disk(&id),
            RowAction::Delete(id) => self.delete_process_id = Some(id),
            RowAction::MakeGroup => self.group_checked_processes(),
            RowAction::ToggleGroup(id) => self.toggle_group_expanded(&id),
            RowAction::OpenGroup(id) => {
                self.overview_group = Some(id);
                self.navigate(Nav::Overview);
            }
            RowAction::RenameGroup(id) => {
                let name = self
                    .config
                    .get_group(&id)
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                self.rename_group = Some((id, name));
            }
            RowAction::GroupAction(id, action) => self.group_action(&id, action),
            RowAction::Ungroup(id) => self.ungroup(&id),
        }
    }
}

fn menu(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.set_min_width(230.0);
    ui.spacing_mut().item_spacing.y = 0.0;
    add(ui);
}

fn caps_job(text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: bold(10.5),
            color: FAINT,
            extra_letter_spacing: 0.9,
            ..Default::default()
        },
    );
    job
}
