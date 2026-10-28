//! The header (brand, stack summary, search, stack controls, local API, settings), the footer,
//! and the process switcher that replaces the sidebar in narrow windows.

use eframe::egui::{
    self, pos2, vec2, Align, Color32, CornerRadius, Layout, Popup, PopupCloseBehavior, Rect,
    RectAlign, Sense, Stroke, StrokeKind, Ui,
};

use super::icons::{self, Icon};
use super::theme::{self, Tier, *};
use super::widgets::{self, Btn, Light};
use super::{format, Nav, ProcessManagerApp, StackAction, View};

impl ProcessManagerApp {
    pub(super) fn draw_header(&mut self, ui: &mut Ui, tier: Tier) {
        let pad_y = if tier <= Tier::Compact { 9.0 } else { 12.0 };
        let rect = ui.available_rect_before_wrap();
        let row = Rect::from_min_size(
            pos2(rect.left() + HEADER_PAD_X, rect.top() + pad_y),
            vec2(rect.width() - HEADER_PAD_X * 2.0, CONTROL_HEIGHT),
        );
        ui.allocate_rect(
            Rect::from_min_size(rect.min, vec2(rect.width(), CONTROL_HEIGHT + pad_y * 2.0)),
            Sense::hover(),
        );

        // Right side first, so the left knows how much room it has.
        let mut right_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(Layout::right_to_left(Align::Center)),
        );
        right_ui.spacing_mut().item_spacing.x = 6.0;
        self.draw_header_controls(&mut right_ui, tier);
        let used_right = right_ui.min_rect().width();

        let left_rect =
            Rect::from_min_max(row.min, pos2(row.right() - used_right - 14.0, row.bottom()));
        let mut left_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(left_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        left_ui.set_clip_rect(left_rect.intersect(ui.clip_rect()));
        if tier.has_sidebar() {
            self.draw_brand(&mut left_ui);
            if left_rect.width() > 330.0 {
                left_ui.add_space(12.0);
                self.draw_summary(&mut left_ui, tier);
            }
        } else {
            self.draw_process_switcher(&mut left_ui, tier);
        }
    }

    fn draw_brand(&self, ui: &mut Ui) {
        let app = widgets::galley(ui, "PROCESS MANAGER", bold(10.0), FAINT);
        let stack = widgets::galley(ui, &self.config.stack_name, bold(19.0), TEXT);
        let text_w = app.size().x.max(stack.size().x) + 1.4 * 15.0;
        let (rect, _) = ui.allocate_exact_size(vec2(30.0 + 11.0 + text_w, 30.0), Sense::hover());
        let painter = ui.painter();
        paint_mark(painter, Rect::from_min_size(rect.min, vec2(30.0, 30.0)));
        let x = rect.left() + 41.0;
        let job = {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                "PROCESS MANAGER",
                0.0,
                egui::TextFormat {
                    font_id: bold(10.0),
                    color: FAINT,
                    extra_letter_spacing: 1.4,
                    ..Default::default()
                },
            );
            job
        };
        let app = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(pos2(x, rect.top() - 1.0), app, FAINT);
        painter.galley(pos2(x, rect.top() + 9.0), stack, TEXT);
    }

    fn draw_summary(&self, ui: &mut Ui, tier: Tier) {
        let (line, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
        ui.painter()
            .vline(line.center().x, line.y_range(), Stroke::new(1.0_f32, LINE));
        ui.add_space(16.0);
        let rollup = self.stack_rollup();
        let mut items = vec![(Light::Running, rollup.running, "running")];
        if rollup.busy > 0 {
            items.push((Light::Busy, rollup.busy, "changing"));
        }
        if rollup.attention > 0 {
            items.push((
                Light::Error,
                rollup.attention,
                if rollup.attention == 1 {
                    "needs a look"
                } else {
                    "need a look"
                },
            ));
        }
        if rollup.stopped > 0 {
            items.push((Light::Stopped, rollup.stopped, "stopped"));
        }
        if rollup.scheduled > 0 && tier >= Tier::Full {
            items.push((Light::Scheduled, rollup.scheduled, "scheduled"));
        }
        let available = ui.available_width();
        let mut used = 0.0;
        for (light, count, label) in items {
            let count_g = widgets::galley(ui, count.to_string(), bold(13.0), TEXT);
            let label_g = widgets::galley(ui, label, body(13.0), MUTED);
            let width = 8.0 + 7.0 + count_g.size().x + 4.0 + label_g.size().x;
            if used + width > available {
                break;
            }
            let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
            let painter = ui.painter();
            widgets::paint_light(ui, painter, pos2(rect.left() + 4.0, rect.center().y), light);
            let count_w = count_g.size().x;
            widgets::paint_galley_left(
                painter,
                pos2(rect.left() + 15.0, rect.center().y),
                count_g,
                TEXT,
            );
            widgets::paint_galley_left(
                painter,
                pos2(rect.left() + 19.0 + count_w, rect.center().y),
                label_g,
                MUTED,
            );
            used += width + 14.0;
            ui.add_space(14.0 - ui.spacing().item_spacing.x);
        }
        if tier >= Tier::Wide && rollup.running > 0 {
            let load = format!(
                "{} CPU · {}",
                format::format_cpu(rollup.cpu),
                format::format_memory(rollup.memory)
            );
            let g = widgets::galley(ui, load, mono(11.5), FAINT);
            if used + g.size().x < available {
                ui.label(egui::RichText::new(g.text()).font(mono(11.5)).color(FAINT));
            }
        }
    }

    /// Settings, local API, stack controls and search, laid out right to left.
    fn draw_header_controls(&mut self, ui: &mut Ui, tier: Tier) {
        let settings = Btn::icon(Icon::Sliders)
            .selected(self.view == View::StackSettings)
            .show(ui)
            .on_hover_text("Stack settings: name, log folder, local API");
        if settings.clicked() {
            self.navigate(Nav::StackSettings);
        }

        if tier > Tier::Tiny {
            self.draw_api_pill(ui, tier);
            ui.add_space(4.0);
            widgets::vdivider(ui, 22.0);
            ui.add_space(4.0);
        }

        if tier >= Tier::Medium {
            let compact = tier == Tier::Medium;
            let restart = if compact {
                Btn::icon(Icon::Restart)
            } else {
                Btn::new("Restart all").with_icon(Icon::Restart)
            };
            if restart
                .show(ui)
                .on_hover_text("Restart every process that answers Restart all · Ctrl Shift R")
                .clicked()
            {
                self.stack_action(StackAction::Restart);
            }
            let stop = if compact {
                Btn::icon(Icon::Stop)
            } else {
                Btn::new("Stop all").with_icon(Icon::Stop)
            };
            if stop
                .show(ui)
                .on_hover_text("Stop every process that answers Stop all · Ctrl Shift X")
                .clicked()
            {
                self.stack_action(StackAction::Stop);
            }
            let start = if compact {
                Btn::icon(Icon::Play).primary()
            } else {
                Btn::new("Start all").with_icon(Icon::Play).primary()
            };
            if start
                .show(ui)
                .on_hover_text("Start every process that answers Start all · Ctrl Shift S")
                .clicked()
            {
                self.stack_action(StackAction::Start);
            }
        } else {
            let menu = Btn::icon(Icon::More)
                .show(ui)
                .on_hover_text("Stack: start, stop or restart all");
            Popup::menu(&menu)
                .frame(widgets::popup_frame())
                .align(RectAlign::BOTTOM_END)
                .gap(6.0)
                .show(|ui| {
                    ui.set_min_width(220.0);
                    if widgets::menu_item(ui, "Start all", Some("Ctrl Shift S"), false, true)
                        .clicked()
                    {
                        self.stack_action(StackAction::Start);
                    }
                    if widgets::menu_item(ui, "Stop all", Some("Ctrl Shift X"), false, true)
                        .clicked()
                    {
                        self.stack_action(StackAction::Stop);
                    }
                    if widgets::menu_item(ui, "Restart all", Some("Ctrl Shift R"), false, true)
                        .clicked()
                    {
                        self.stack_action(StackAction::Restart);
                    }
                    if tier == Tier::Tiny {
                        widgets::menu_separator(ui);
                        let label = if self.config.remote_control.enabled {
                            "Turn off the local API"
                        } else {
                            "Turn on the local API"
                        };
                        if widgets::menu_item(ui, label, None, false, true).clicked() {
                            self.toggle_api_enabled();
                        }
                        if widgets::menu_item(ui, "Copy agent skill", None, false, true).clicked() {
                            self.copy_agent_skill();
                        }
                    }
                });
        }

        ui.add_space(4.0);
        let search = if tier >= Tier::Wide {
            Btn::new("Search or run a command")
                .with_icon(Icon::Search)
                .text_color(FAINT)
                .kbd("Ctrl K")
                .min_width(250.0)
        } else {
            Btn::icon(Icon::Search)
        };
        if search
            .show(ui)
            .on_hover_text("Search or run a command · Ctrl K")
            .clicked()
        {
            self.palette = Some(super::palette::PaletteState::default());
        }
    }

    fn draw_api_pill(&mut self, ui: &mut Ui, tier: Tier) {
        let on = self.config.remote_control.enabled;
        let label = widgets::galley(ui, if on { "API on" } else { "API off" }, bold(12.0), TEXT);
        let address = self.api_address();
        let addr_g = (tier >= Tier::Full).then(|| widgets::galley(ui, &address, mono(11.5), MUTED));
        let width = 12.0
            + 8.0
            + 8.0
            + label.size().x
            + addr_g.as_ref().map_or(0.0, |g| g.size().x + 8.0)
            + 12.0;
        let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click());
        let painter = ui.painter();
        let border = if response.hovered() {
            BORDER_HI
        } else {
            BORDER
        };
        painter.rect(
            rect,
            CornerRadius::same(15),
            BUTTON,
            Stroke::new(1.0_f32, border),
            StrokeKind::Inside,
        );
        let light = if on { Light::Running } else { Light::Stopped };
        widgets::paint_light(
            ui,
            painter,
            pos2(rect.left() + 16.0, rect.center().y),
            light,
        );
        let label_w = label.size().x;
        widgets::paint_galley_left(
            painter,
            pos2(rect.left() + 28.0, rect.center().y),
            label,
            TEXT,
        );
        if let Some(g) = addr_g {
            widgets::paint_galley_left(
                painter,
                pos2(rect.left() + 36.0 + label_w, rect.center().y),
                g,
                MUTED,
            );
        }
        let response = response.on_hover_text("Local API: status, agent skill, port");
        Popup::from_toggle_button_response(&response)
            .frame(widgets::popup_frame().inner_margin(egui::Margin::same(14)))
            .align(RectAlign::BOTTOM_END)
            .gap(6.0)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(300.0);
                ui.label(egui::RichText::new("Local API").font(bold(13.0)).color(TEXT));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    widgets::light(ui, if on { Light::Running } else { Light::Stopped });
                    let text = if on { format!("Listening on {address}") } else { "Off".to_string() };
                    ui.label(egui::RichText::new(text).font(bold(13.0)).color(if on { GREEN } else { MUTED }));
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Scripts and agents control the stack through it. It has no password, so keep it on loopback.",
                    )
                    .font(body(12.0))
                    .color(MUTED),
                );
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    if Btn::new(if on { "Turn off" } else { "Turn on" }).small().show(ui).clicked() {
                        self.toggle_api_enabled();
                    }
                    if Btn::new("Copy agent skill").with_icon(Icon::Copy).small().show(ui).clicked() {
                        self.copy_agent_skill();
                        ui.close();
                    }
                    if Btn::new("Port and binding…").small().ghost().show(ui).clicked() {
                        self.navigate(Nav::StackSettings);
                        ui.close();
                    }
                });
            });
    }

    /// In narrow windows: the current process (or view) as a button that opens a list of
    /// everything the sidebar would show.
    fn draw_process_switcher(&mut self, ui: &mut Ui, tier: Tier) {
        let (light, title) = match self.view {
            View::Process => match self.selected_process_config() {
                Some(process) => (Some(self.status_view(&process).light), process.name.clone()),
                None => (None, "No processes".to_string()),
            },
            View::Overview => (None, "Overview".to_string()),
            View::StackSettings => (None, "Stack settings".to_string()),
            View::NewProcess => (None, "New process".to_string()),
        };
        let max_w = ui.available_width();
        let title_g =
            widgets::galley_truncated(ui, &title, bold(15.0), TEXT, (max_w - 60.0).max(40.0));
        let width = 10.0 + light.map_or(0.0, |_| 16.0) + title_g.size().x + 8.0 + 14.0 + 10.0;
        let (rect, response) =
            ui.allocate_exact_size(vec2(width.min(max_w), CONTROL_HEIGHT), Sense::click());
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(RADIUS), HOVER);
        }
        let mut x = rect.left() + 10.0;
        if let Some(light) = light {
            widgets::paint_light(ui, painter, pos2(x + 4.0, rect.center().y), light);
            x += 16.0;
        }
        let title_w = title_g.size().x;
        widgets::paint_galley_left(painter, pos2(x, rect.center().y), title_g, TEXT);
        icons::paint(
            painter,
            icons::square(pos2(x + title_w + 15.0, rect.center().y), 14.0),
            14.0,
            Icon::UpDown,
            MUTED,
        );
        let response = response.on_hover_text("Switch process");
        Popup::from_toggle_button_response(&response)
            .frame(widgets::popup_frame())
            .align(RectAlign::BOTTOM_START)
            .gap(6.0)
            .show(|ui| {
                ui.set_width(if tier == Tier::Tiny {
                    (ui.ctx().content_rect().width() - 32.0).min(300.0)
                } else {
                    300.0
                });
                egui::ScrollArea::vertical()
                    .max_height((ui.ctx().content_rect().height() - 120.0).max(160.0))
                    .show(ui, |ui| self.draw_switcher_list(ui));
            });
    }

    fn draw_switcher_list(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let overview = self.nav_row(
            ui,
            Icon::Layers,
            "Overview",
            self.view == View::Overview,
            Some(self.config.processes.len().to_string()),
        );
        if overview.clicked() {
            self.navigate(Nav::Overview);
            ui.close();
        }
        widgets::menu_separator(ui);
        let mut pick = None;
        for item in self.sidebar_items() {
            match item {
                super::SidebarRenderItem::Group(group) => {
                    ui.add_space(4.0);
                    let rollup = self.rollup(group.process_ids.iter());
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(widgets::caps(&group.name).color(TEXT));
                        ui.label(
                            egui::RichText::new(format!("{}/{}", rollup.running, rollup.total))
                                .font(body(11.5))
                                .color(FAINT),
                        );
                    });
                    ui.add_space(2.0);
                }
                super::SidebarRenderItem::Process {
                    process, group_id, ..
                } => {
                    let view = self.status_view(&process);
                    let on = self.view == View::Process
                        && self.selected_process.as_deref() == Some(process.id.as_str());
                    let row = self.process_row(
                        ui,
                        &process.name,
                        view.light,
                        &view.short,
                        view.alert,
                        on,
                        group_id.is_some(),
                        0.0,
                    );
                    if row.clicked() {
                        pick = Some(process.id.clone());
                    }
                }
            }
        }
        if let Some(id) = pick {
            self.select_process(id);
            ui.close();
        }
        widgets::menu_separator(ui);
        if widgets::menu_item(ui, "Add a process", Some("Ctrl N"), false, true).clicked() {
            self.open_new_process();
            ui.close();
        }
        if widgets::menu_item(ui, "Stack settings", None, false, true).clicked() {
            self.navigate(Nav::StackSettings);
            ui.close();
        }
    }

    pub(super) fn draw_footer(&mut self, ui: &mut Ui, tier: Tier) {
        let rect = ui.max_rect();
        let inner = rect.shrink2(vec2(HEADER_PAD_X, 0.0));
        let mut left = inner.left();
        let painter = ui.painter().clone();
        let on = self.config.remote_control.enabled;
        widgets::paint_light(
            ui,
            &painter,
            pos2(left + 4.0, inner.center().y),
            if on { Light::Running } else { Light::Stopped },
        );
        left += 15.0;
        let api_text = if !on {
            "Local API off".to_string()
        } else if tier >= Tier::Compact {
            format!("Local API on · {}", self.api_address())
        } else {
            "API on".to_string()
        };
        let g = widgets::galley(ui, &api_text, body(11.5), MUTED);
        left += g.size().x;
        widgets::paint_galley_left(
            &painter,
            pos2(left - g.size().x, inner.center().y),
            g,
            MUTED,
        );

        if tier >= Tier::Full {
            let config_path = crate::config::AppConfig::config_path();
            let beside_exe = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
                .is_some_and(|dir| config_path.parent() == Some(dir.as_path()));
            let text = if beside_exe {
                "processes.json beside the executable".to_string()
            } else {
                format!(
                    "processes.json in {}",
                    config_path
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                )
            };
            let g = widgets::galley_truncated(ui, &text, body(11.5), FAINT, 320.0);
            let r = Rect::from_min_size(
                pos2(left + 16.0, inner.top()),
                vec2(g.size().x, inner.height()),
            );
            widgets::paint_galley_left(&painter, pos2(r.left(), inner.center().y), g, FAINT);
            let _ = ui
                .interact(r, ui.id().with("config_path"), Sense::hover())
                .on_hover_text(config_path.display().to_string());
            left = r.right();
        }

        // Right side: version, then the palette hint.
        let mut right = inner.right();
        if tier > Tier::Tiny {
            let version = format!("v{}", env!("CARGO_PKG_VERSION"));
            let g = widgets::galley(ui, &version, body(11.5), FAINT);
            right -= g.size().x;
            widgets::paint_galley_left(&painter, pos2(right, inner.center().y), g, FAINT);
            right -= 16.0;
        }
        if tier >= Tier::Compact {
            let label = widgets::galley(ui, "Commands", body(11.5), MUTED);
            let key = widgets::galley(ui, "Ctrl K", mono(10.5), FAINT);
            let width = key.size().x + 10.0 + 6.0 + label.size().x;
            let r = Rect::from_min_max(
                pos2(right - width, inner.top() + 4.0),
                pos2(right, inner.bottom() - 4.0),
            );
            let response = ui.interact(r, ui.id().with("footer_palette"), Sense::click());
            let key_rect = Rect::from_min_size(
                pos2(r.left(), inner.center().y - 9.0),
                vec2(key.size().x + 10.0, 18.0),
            );
            painter.rect(
                key_rect,
                CornerRadius::same(4),
                PANEL,
                Stroke::new(1.0_f32, BORDER),
                StrokeKind::Inside,
            );
            painter.galley(key_rect.center() - key.size() * 0.5, key, FAINT);
            let color = if response.hovered() { TEXT } else { MUTED };
            widgets::paint_galley_left(
                &painter,
                pos2(key_rect.right() + 6.0, inner.center().y),
                label,
                color,
            );
            if response.clicked() {
                self.palette = Some(super::palette::PaletteState::default());
            }
            right = r.left() - 16.0;
        }

        if let Some(message) = self.visible_message() {
            let room =
                Rect::from_min_max(pos2(left + 16.0, inner.top()), pos2(right, inner.bottom()));
            if room.width() > 40.0 {
                let g = widgets::galley_truncated(ui, &message, body(11.5), TEXT, room.width());
                let x = room.center().x - g.size().x * 0.5;
                widgets::paint_galley_left(
                    &painter,
                    pos2(x.max(room.left()), inner.center().y),
                    g,
                    TEXT,
                );
            }
        }
        let _ = Color32::TRANSPARENT;
    }

    /// A row in the sidebar or the switcher: light, name, a short note on the right.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn process_row(
        &self,
        ui: &mut Ui,
        name: &str,
        light: Light,
        meta: &str,
        alert: bool,
        on: bool,
        indented: bool,
        flash: f32,
    ) -> egui::Response {
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click_and_drag());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let painter = ui.painter();
        let radius = CornerRadius::same(RADIUS);
        if on {
            painter.rect_filled(rect, radius, SELECTED);
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
                CornerRadius {
                    nw: RADIUS,
                    sw: RADIUS,
                    ne: 0,
                    se: 0,
                },
                ACCENT,
            );
        } else if alert {
            painter.rect_filled(rect, radius, theme::red_soft());
        }
        if !on && response.hovered() {
            painter.rect_filled(rect, radius, HOVER);
        }
        if flash > 0.0 {
            painter.rect_filled(
                rect,
                radius,
                Color32::from_rgba_unmultiplied(0xe2, 0x5f, 0x5f, (flash * 60.0) as u8),
            );
        }
        let left = rect.left() + if indented { 27.0 } else { 12.0 };
        widgets::paint_light(ui, painter, pos2(left + 4.0, rect.center().y), light);
        let meta_g = (!meta.is_empty())
            .then(|| widgets::galley(ui, meta, mono(11.0), if alert { RED } else { FAINT }));
        let meta_w = meta_g.as_ref().map_or(0.0, |g| g.size().x + 10.0);
        let name_left = left + 18.0;
        let name_w = rect.right() - 10.0 - meta_w - name_left;
        let name_g = widgets::galley_truncated(ui, name, body(13.0), TEXT, name_w);
        widgets::paint_galley_left(painter, pos2(name_left, rect.center().y), name_g, TEXT);
        if let Some(g) = meta_g {
            let color = if alert { RED } else { FAINT };
            widgets::paint_galley_left(
                painter,
                pos2(rect.right() - 10.0 - g.size().x, rect.center().y),
                g,
                color,
            );
        }
        response
    }

    /// A navigation entry (Overview): icon, label, a count on the right.
    pub(super) fn nav_row(
        &self,
        ui: &mut Ui,
        icon: Icon,
        label: &str,
        on: bool,
        count: Option<String>,
    ) -> egui::Response {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
        let painter = ui.painter();
        let radius = CornerRadius::same(RADIUS);
        if on {
            painter.rect_filled(rect, radius, SELECTED);
            painter.rect_filled(
                Rect::from_min_size(rect.min, vec2(2.0, rect.height())),
                CornerRadius {
                    nw: RADIUS,
                    sw: RADIUS,
                    ne: 0,
                    se: 0,
                },
                ACCENT,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, radius, HOVER);
        }
        let color = if on || response.hovered() {
            TEXT
        } else {
            MUTED
        };
        icons::paint(
            painter,
            icons::square(pos2(rect.left() + 18.0, rect.center().y), 15.0),
            15.0,
            icon,
            color,
        );
        let g = widgets::galley(ui, label, bold(13.0), color);
        widgets::paint_galley_left(painter, pos2(rect.left() + 34.0, rect.center().y), g, color);
        if let Some(count) = count {
            let g = widgets::galley(ui, count, body(11.5), FAINT);
            widgets::paint_galley_left(
                painter,
                pos2(rect.right() - 10.0 - g.size().x, rect.center().y),
                g,
                FAINT,
            );
        }
        response
    }
}

/// The app's mark: a tile with a prompt and three status bars (green, amber, red), after the
/// app icon.
pub(super) fn paint_mark(painter: &egui::Painter, rect: Rect) {
    let s = rect.width() / 30.0;
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * s;
    painter.rect(
        Rect::from_min_max(p(0.5, 0.5), p(29.5, 29.5)),
        CornerRadius::same((7.0 * s) as u8),
        PANEL2,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    painter.add(egui::Shape::line(
        vec![p(8.0, 10.5), p(11.5, 13.5), p(8.0, 16.5)],
        Stroke::new(2.0 * s, TEXT),
    ));
    painter.rect_filled(
        Rect::from_min_size(p(14.0, 12.5), vec2(8.0, 2.0) * s),
        CornerRadius::same(1),
        GREEN,
    );
    painter.rect_filled(
        Rect::from_min_size(p(8.0, 19.5), vec2(2.0, 2.0) * s),
        CornerRadius::same(1),
        AMBER,
    );
    painter.rect_filled(
        Rect::from_min_size(p(12.0, 19.5), vec2(10.0, 2.0) * s),
        CornerRadius::same(1),
        RED,
    );
}
