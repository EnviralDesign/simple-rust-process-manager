//! The command palette (Ctrl K): every action and every process, filtered as you type.

use eframe::egui::{
    self, pos2, vec2, Align2, Context, CornerRadius, Key, Modifiers, Rect, ScrollArea, Sense,
    Stroke, StrokeKind,
};

use super::icons::{self, Icon};
use super::theme::{Tier, *};
use super::widgets;
use super::{Nav, ProcessManagerApp, ProcessTab, StackAction};

#[derive(Default)]
pub(super) struct PaletteState {
    query: String,
    index: usize,
    /// Scroll the chosen row into view after a keyboard move.
    scroll_to_index: bool,
}

#[derive(Clone)]
enum Command {
    StartStop(String),
    Restart(String),
    Edit(String),
    Activity(String),
    Screen(String),
    Interrupt(String),
    CopyCommand(String, String),
    Stack(StackAction),
    Overview,
    GoTo(String),
    NewProcess,
    StackSettings,
    ToggleApi,
    CopyAgentSkill,
    OpenLogs,
    Reload,
}

struct Entry {
    section: String,
    label: String,
    hint: String,
    kbd: &'static str,
    command: Command,
}

impl ProcessManagerApp {
    fn palette_entries(&self) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut add =
            |section: &str, label: String, hint: String, kbd: &'static str, command: Command| {
                entries.push(Entry {
                    section: section.to_string(),
                    label,
                    hint,
                    kbd,
                    command,
                });
            };
        if let Some(process) = self.selected_process_config() {
            let live = self.is_live(&process.id);
            let section = process.name.clone();
            let status = self.status_view(&process).word;
            add(
                &section,
                format!("{} {}", if live { "Stop" } else { "Start" }, process.name),
                status,
                "Ctrl Enter",
                Command::StartStop(process.id.clone()),
            );
            add(
                &section,
                format!("Restart {}", process.name),
                process.command.clone(),
                "Ctrl R",
                Command::Restart(process.id.clone()),
            );
            add(
                &section,
                format!("Edit {} settings", process.name),
                "Command, restart, schedule, logs".into(),
                "Ctrl ,",
                Command::Edit(process.id.clone()),
            );
            add(
                &section,
                format!("Show {} activity", process.name),
                "Starts, stops and exits, with their causes".into(),
                "",
                Command::Activity(process.id.clone()),
            );
            if self.has_screen(&process) {
                add(
                    &section,
                    format!("Show {} screen", process.name),
                    "Its terminal, where you can type into it".into(),
                    "",
                    Command::Screen(process.id.clone()),
                );
                if live {
                    add(
                        &section,
                        format!("Send Ctrl+C to {}", process.name),
                        "Interrupt it, as Ctrl+C in a terminal does".into(),
                        "",
                        Command::Interrupt(process.id.clone()),
                    );
                }
            }
            add(
                &section,
                "Copy the command".into(),
                process.command.clone(),
                "",
                Command::CopyCommand(process.command.clone(), process.name.clone()),
            );
        }
        let answering = |f: fn(&crate::config::ProcessConfig) -> bool| {
            format!(
                "{} of {} answer it",
                self.config.processes.iter().filter(|p| f(p)).count(),
                self.config.processes.len()
            )
        };
        add(
            "Stack",
            "Start all".into(),
            answering(|p| p.respond_to_start_all),
            "Ctrl Shift S",
            Command::Stack(StackAction::Start),
        );
        add(
            "Stack",
            "Stop all".into(),
            answering(|p| p.respond_to_stop_all),
            "Ctrl Shift X",
            Command::Stack(StackAction::Stop),
        );
        add(
            "Stack",
            "Restart all".into(),
            answering(|p| p.respond_to_restart_all),
            "Ctrl Shift R",
            Command::Stack(StackAction::Restart),
        );
        add(
            "Stack",
            "Overview".into(),
            "Every process in one table".into(),
            "Ctrl 0",
            Command::Overview,
        );
        add(
            "Stack",
            "Add a process".into(),
            "A command or a Docker container".into(),
            "Ctrl N",
            Command::NewProcess,
        );
        for process in &self.config.processes {
            let view = self.status_view(process);
            add(
                "Go to",
                process.name.clone(),
                view.word,
                "",
                Command::GoTo(process.id.clone()),
            );
        }
        let api = if self.config.remote_control.enabled {
            "Turn off the local API"
        } else {
            "Turn on the local API"
        };
        add(
            "App",
            api.into(),
            format!("{}, loopback only", self.api_address()),
            "",
            Command::ToggleApi,
        );
        add(
            "App",
            "Copy agent skill".into(),
            "Address, endpoints and process ids for an agent".into(),
            "",
            Command::CopyAgentSkill,
        );
        add(
            "App",
            "Stack settings".into(),
            "Stack name, log folder, local API".into(),
            "",
            Command::StackSettings,
        );
        add(
            "App",
            "Open the logs folder".into(),
            "Process Manager logs".into(),
            "",
            Command::OpenLogs,
        );
        add(
            "App",
            "Reload processes.json".into(),
            "Stops everything, then reads the file again".into(),
            "",
            Command::Reload,
        );
        entries
    }

    fn run_command(&mut self, command: Command) {
        match command {
            Command::StartStop(id) => self.toggle_process(&id),
            Command::Restart(id) => self.restart_process(&id),
            Command::Edit(id) => {
                self.open_process_tab(id, ProcessTab::Settings);
            }
            Command::Activity(id) => {
                self.open_process_tab(id, ProcessTab::Activity);
            }
            Command::Screen(id) => {
                self.open_process_tab(id, ProcessTab::Screen);
            }
            Command::Interrupt(id) => self.send_to_terminal(&id, vec![0x03]),
            Command::CopyCommand(command, name) => {
                self.copy_text(&command, format!("Copied the command of {name}"))
            }
            Command::Stack(action) => self.stack_action(action),
            Command::Overview => {
                self.overview_group = None;
                self.navigate(Nav::Overview);
            }
            Command::GoTo(id) => self.navigate(Nav::Process(id)),
            Command::NewProcess => self.open_new_process(),
            Command::StackSettings => self.navigate(Nav::StackSettings),
            Command::ToggleApi => self.toggle_api_enabled(),
            Command::CopyAgentSkill => self.copy_agent_skill(),
            Command::OpenLogs => {
                let root = self.config.log_directory.clone();
                let base = if root.trim().is_empty() || root.trim() == "." {
                    crate::platform::data_directory()
                } else {
                    std::path::PathBuf::from(root.trim())
                };
                self.open_folder(base.join("Process Manager logs"));
            }
            Command::Reload => self.reload_processes_confirm_open = true,
        }
    }

    pub(super) fn draw_palette(&mut self, ctx: &Context, tier: Tier) {
        let Some(mut state) = self.palette.take() else {
            return;
        };
        let entries = self.palette_entries();
        let words: Vec<String> = state
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let shown: Vec<&Entry> = entries
            .iter()
            .filter(|e| {
                let hay = format!("{} {} {}", e.label, e.hint, e.section).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect();
        if state.index >= shown.len() {
            state.index = shown.len().saturating_sub(1);
        }

        // Keys first, before the text field sees them.
        let (down, up, enter, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if down && !shown.is_empty() {
            state.index = (state.index + 1).min(shown.len() - 1);
            state.scroll_to_index = true;
        }
        if up {
            state.index = state.index.saturating_sub(1);
            state.scroll_to_index = true;
        }
        let mut run: Option<Command> = None;
        if enter {
            run = shown.get(state.index).map(|e| e.command.clone());
        }
        let mut close = escape || run.is_some();

        let screen = ctx.content_rect();
        let width = (screen.width() - 32.0).min(620.0);
        let top = if tier <= Tier::Compact { 24.0 } else { 92.0 };
        let modal = egui::Modal::new(egui::Id::new("command_palette"))
            .backdrop_color(egui::Color32::from_black_alpha(140))
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(CornerRadius::same(RADIUS_CARD + 2))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 24],
                        blur: 80,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(180),
                    }),
            )
            .area(
                egui::Modal::default_area(egui::Id::new("command_palette_area"))
                    .anchor(Align2::CENTER_TOP, vec2(0.0, top)),
            );
        let response = modal.show(ctx, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(0.0, 0.0);

            // Search row.
            let (row, _) = ui.allocate_exact_size(vec2(width, 50.0), Sense::hover());
            icons::paint(
                ui.painter(),
                icons::square(pos2(row.left() + 24.0, row.center().y), 16.0),
                16.0,
                Icon::Search,
                FAINT,
            );
            let esc = widgets::galley(ui, "Esc", mono(10.5), FAINT);
            let esc_rect = Rect::from_center_size(
                pos2(
                    row.right() - 16.0 - (esc.size().x + 10.0) * 0.5,
                    row.center().y,
                ),
                vec2(esc.size().x + 10.0, 18.0),
            );
            ui.painter().rect(
                esc_rect,
                CornerRadius::same(4),
                PANEL,
                Stroke::new(1.0_f32, BORDER),
                StrokeKind::Inside,
            );
            ui.painter()
                .galley(esc_rect.center() - esc.size() * 0.5, esc, FAINT);
            let field = Rect::from_min_max(
                pos2(row.left() + 42.0, row.top()),
                pos2(esc_rect.left() - 10.0, row.bottom()),
            );
            let edit = widgets::put_free(
                ui,
                field,
                egui::TextEdit::singleline(&mut state.query)
                    .frame(false)
                    .hint_text(egui::RichText::new("Type a command or a process name").color(FAINT))
                    .font(body(15.0))
                    .text_color(TEXT)
                    .margin(egui::Margin::ZERO)
                    .desired_width(field.width())
                    .vertical_align(egui::Align::Center)
                    .id(egui::Id::new("palette_query")),
            );
            edit.request_focus();
            if edit.changed() {
                state.index = 0;
            }
            ui.painter().hline(
                row.x_range(),
                row.bottom() - 0.5,
                Stroke::new(1.0_f32, LINE),
            );

            // Entries.
            let list_h = (screen.height() - top - 50.0 - 34.0 - 40.0).clamp(120.0, 420.0);
            ScrollArea::vertical()
                .max_height(list_h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.add_space(6.0);
                    if shown.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(format!("Nothing matches “{}”.", state.query))
                                    .font(body(13.0))
                                    .color(MUTED),
                            );
                        });
                        ui.add_space(20.0);
                    }
                    let mut section = "";
                    for (index, entry) in shown.iter().enumerate() {
                        if entry.section != section {
                            section = &entry.section;
                            let (r, _) = ui.allocate_exact_size(vec2(width, 28.0), Sense::hover());
                            let mut job = egui::text::LayoutJob::default();
                            job.append(
                                &entry.section.to_uppercase(),
                                0.0,
                                egui::TextFormat {
                                    font_id: bold(10.5),
                                    color: FAINT,
                                    extra_letter_spacing: 0.9,
                                    ..Default::default()
                                },
                            );
                            let g = ui.fonts_mut(|f| f.layout_job(job));
                            ui.painter().galley(
                                pos2(r.left() + 16.0, r.bottom() - 4.0 - g.size().y),
                                g,
                                FAINT,
                            );
                        }
                        let (outer, response) =
                            ui.allocate_exact_size(vec2(width, 36.0), Sense::click());
                        let r = outer.shrink2(vec2(6.0, 0.0));
                        let on = index == state.index;
                        if response.hovered() && ui.input(|i| i.pointer.delta() != vec2(0.0, 0.0)) {
                            state.index = index;
                        }
                        let painter = ui.painter();
                        if on {
                            painter.rect_filled(r, CornerRadius::same(RADIUS), SELECTED);
                            painter.rect_filled(
                                Rect::from_min_size(r.min, vec2(2.0, r.height())),
                                CornerRadius {
                                    nw: RADIUS,
                                    sw: RADIUS,
                                    ne: 0,
                                    se: 0,
                                },
                                ACCENT,
                            );
                            if state.scroll_to_index {
                                ui.scroll_to_rect(outer, None);
                                state.scroll_to_index = false;
                            }
                        }
                        let label = widgets::galley(ui, &entry.label, bold(13.0), TEXT);
                        let label_w = label.size().x;
                        widgets::paint_galley_left(
                            painter,
                            pos2(r.left() + 10.0, r.center().y),
                            label,
                            TEXT,
                        );
                        let kbd_w = if entry.kbd.is_empty() {
                            0.0
                        } else {
                            let g = widgets::galley(ui, entry.kbd, mono(10.5), FAINT);
                            let k = Rect::from_center_size(
                                pos2(r.right() - 10.0 - (g.size().x + 10.0) * 0.5, r.center().y),
                                vec2(g.size().x + 10.0, 18.0),
                            );
                            painter.rect(
                                k,
                                CornerRadius::same(4),
                                PANEL,
                                Stroke::new(1.0_f32, BORDER),
                                StrokeKind::Inside,
                            );
                            let w = k.width();
                            painter.galley(k.center() - g.size() * 0.5, g, FAINT);
                            w + 14.0
                        };
                        let hint_left = r.left() + 10.0 + label_w + 12.0;
                        let hint_rect = Rect::from_min_max(
                            pos2(hint_left, r.top()),
                            pos2(r.right() - 10.0 - kbd_w, r.bottom()),
                        );
                        if hint_rect.width() > 30.0 {
                            widgets::text_in(ui, hint_rect, &entry.hint, body(12.0), FAINT);
                        }
                        if response.clicked() {
                            run = Some(entry.command.clone());
                            close = true;
                        }
                    }
                    ui.add_space(6.0);
                });

            // Footer.
            let (foot, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
            ui.painter()
                .hline(foot.x_range(), foot.top() + 0.5, Stroke::new(1.0_f32, LINE));
            let mut x = foot.left() + 16.0;
            for (keys, label) in [("↑ ↓", "move"), ("Enter", "run"), ("Esc", "close")] {
                let k = widgets::galley(ui, keys, mono(10.5), FAINT);
                let kr = Rect::from_min_size(
                    pos2(x, foot.center().y - 9.0),
                    vec2(k.size().x + 10.0, 18.0),
                );
                ui.painter().rect(
                    kr,
                    CornerRadius::same(4),
                    PANEL,
                    Stroke::new(1.0_f32, BORDER),
                    StrokeKind::Inside,
                );
                ui.painter().galley(kr.center() - k.size() * 0.5, k, FAINT);
                let l = widgets::galley(ui, label, body(11.5), FAINT);
                let lw = l.size().x;
                widgets::paint_galley_left(
                    ui.painter(),
                    pos2(kr.right() + 6.0, foot.center().y),
                    l,
                    FAINT,
                );
                x = kr.right() + 6.0 + lw + 16.0;
            }
        });
        if response.backdrop_response.clicked() {
            close = true;
        }
        if !close {
            self.palette = Some(state);
        }
        if let Some(command) = run {
            self.run_command(command);
        }
    }
}
