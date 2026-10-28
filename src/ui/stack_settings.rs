//! Stack settings, in place like a process's: the stack's name and log folder, the error flash,
//! the local API, the config file, and About.

use eframe::egui::{
    self, pos2, vec2, Align, CornerRadius, Layout, Rect, RichText, ScrollArea, Sense, Stroke,
    StrokeKind, Ui,
};

use crate::config::AppConfig;

use super::editor::{path_edit, RelativePathSuggestion};
use super::icons::Icon;
use super::theme::{Tier, *};
use super::widgets::{self, Btn};
use super::{
    ProcessManagerApp, PROJECT_GITHUB_ACCOUNT_HANDLE, PROJECT_GITHUB_ACCOUNT_URL,
    PROJECT_WEBSITE_URL,
};

#[derive(Clone)]
pub(super) struct StackSettingsForm {
    stack_name: String,
    log_directory: String,
    log_suggestion: RelativePathSuggestion,
    flash_seconds: String,
    pub api_enabled: bool,
    port: String,
    error: Option<String>,
}

impl StackSettingsForm {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            stack_name: config.stack_name.clone(),
            log_directory: config.log_directory.clone(),
            log_suggestion: RelativePathSuggestion {
                base_directory: Some(crate::platform::data_directory()),
                ..Default::default()
            },
            flash_seconds: config.process_error_flash_seconds.to_string(),
            api_enabled: config.remote_control.enabled,
            port: config.remote_control.port.to_string(),
            error: None,
        }
    }

    fn differs_from(&self, config: &AppConfig) -> bool {
        self.stack_name.trim() != config.stack_name
            || normalize_log_directory(&self.log_directory) != config.log_directory
            || self.flash_seconds.trim() != config.process_error_flash_seconds.to_string()
            || self.api_enabled != config.remote_control.enabled
            || self.port.trim() != config.remote_control.port.to_string()
    }
}

impl ProcessManagerApp {
    pub(super) fn draw_stack_settings(&mut self, ui: &mut Ui, tier: Tier) {
        let gutter = if tier <= Tier::Compact { 14.0 } else { GUTTER };
        let outer = ui.max_rect();
        let dirty = self.stack_form.differs_from(&self.config);
        let bar_h = if dirty { 64.0 } else { 0.0 };
        let scroll_rect =
            Rect::from_min_max(outer.min, pos2(outer.right(), outer.bottom() - bar_h));
        let mut page = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(scroll_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        let mut reload = false;
        let mut copy_skill = false;
        let mut open_config = false;
        ScrollArea::vertical().id_salt("stack_settings").auto_shrink([false, false]).show(&mut page, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin { left: gutter as i8, right: gutter as i8, top: 18, bottom: 24 })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
                    ui.label(RichText::new("Stack settings").font(bold(24.0)).color(TEXT));
                    ui.add_space(-8.0);
                    ui.label(RichText::new("These apply to the whole stack and live in processes.json.").font(body(13.0)).color(MUTED));
                    let width = ui.available_width();
                    let two = width >= 880.0;
                    let column = if two { (width - 14.0) / 2.0 } else { width };
                    let address = self.api_address();
                    let form = &mut self.stack_form;
                    let left = |ui: &mut Ui, form: &mut StackSettingsForm| {
                        widgets::card(ui, "Stack", |ui| {
                            let w = ui.available_width();
                            label(ui, "Name");
                            widgets::text_field(ui, "stack_name", &mut form.stack_name, "My Stack", w, false);
                            hint(ui, "Shown in the header and the window's title.");
                            label(ui, "Log folder");
                            path_edit(ui, "log_dir", &mut form.log_directory, &mut form.log_suggestion, ".", w);
                            hint(ui, "Session logs go to a “Process Manager logs” folder here. A dot means beside processes.json.");
                            label(ui, "Error flash");
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                widgets::text_field(ui, "flash", &mut form.flash_seconds, "5", 64.0, true);
                                ui.label(RichText::new("seconds a sidebar row glows after an error; 0 turns it off").font(body(12.0)).color(FAINT));
                            });
                        });
                    };
                    let right = |ui: &mut Ui, form: &mut StackSettingsForm, address: String, copy_skill: &mut bool, reload: &mut bool, open_config: &mut bool| {
                        widgets::card(ui, "Local API", |ui| {
                            let w = ui.available_width();
                            ui.horizontal(|ui| {
                                ui.allocate_ui_with_layout(vec2(w - 50.0, 0.0), Layout::top_down(Align::Min), |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.label(RichText::new("Turn on the local API").font(bold(13.0)).color(TEXT));
                                    ui.label(RichText::new("Scripts and agents start, stop and read the stack over HTTP. It has no password.").font(body(11.5)).color(FAINT));
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    widgets::switch(ui, &mut form.api_enabled);
                                });
                            });
                            label(ui, "Port");
                            widgets::text_field(ui, "port", &mut form.port, "47821", 120.0, true);
                            hint(ui, &format!("Listens on {address}. The bind address is set in processes.json (remote_control.bind_address)."));
                            if Btn::new("Copy agent skill").with_icon(Icon::Copy).small().show(ui).clicked() {
                                *copy_skill = true;
                            }
                        });
                        widgets::card(ui, "processes.json", |ui| {
                            let path = AppConfig::config_path();
                            ui.label(RichText::new(path.display().to_string()).font(mono(12.0)).color(TEXT));
                            hint(ui, "Edits by hand are picked up with Reload; every process stops first.");
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                if Btn::new("Reload processes.json").small().show(ui).clicked() {
                                    *reload = true;
                                }
                                if Btn::new("Open its folder").with_icon(Icon::Folder).small().ghost().show(ui).clicked() {
                                    *open_config = true;
                                }
                            });
                        });
                    };
                    if two {
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(vec2(column, 0.0), Layout::top_down(Align::Min), |ui| {
                                ui.set_width(column);
                                left(ui, form);
                            });
                            ui.allocate_ui_with_layout(vec2(column, 0.0), Layout::top_down(Align::Min), |ui| {
                                ui.set_width(column);
                                right(ui, form, address.clone(), &mut copy_skill, &mut reload, &mut open_config);
                            });
                        });
                    } else {
                        left(ui, form);
                        right(ui, form, address, &mut copy_skill, &mut reload, &mut open_config);
                    }
                    about_card(ui);
                });
        });
        ui.allocate_rect(outer, Sense::hover());
        if copy_skill {
            self.copy_agent_skill();
        }
        if reload {
            self.reload_processes_confirm_open = true;
        }
        if open_config {
            self.open_folder(crate::platform::data_directory());
        }

        if dirty {
            let bar = Rect::from_min_max(
                pos2(outer.left() + gutter, outer.bottom() - bar_h + 8.0),
                pos2(outer.right() - gutter, outer.bottom() - 12.0),
            );
            let painter = ui.painter();
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
            bar_ui.label(
                RichText::new("Unsaved stack settings")
                    .font(bold(13.0))
                    .color(TEXT),
            );
            if let Some(error) = &self.stack_form.error {
                bar_ui.label(RichText::new(error).font(body(12.0)).color(RED));
            }
            let mut save = false;
            let mut discard = false;
            bar_ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                save = Btn::new("Save").primary().show(ui).clicked();
                discard = Btn::new("Discard").show(ui).clicked();
            });
            if save {
                self.save_stack_settings();
            }
            if discard {
                self.stack_form = StackSettingsForm::from_config(&self.config);
            }
        }
    }

    fn save_stack_settings(&mut self) {
        let form = &mut self.stack_form;
        let port = match form.port.trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                form.error = Some("The port is a number from 1 to 65535.".into());
                return;
            }
        };
        let flash = match form.flash_seconds.trim().parse::<u64>() {
            Ok(seconds) => seconds,
            Err(_) => {
                form.error = Some("The error flash is a whole number of seconds.".into());
                return;
            }
        };
        form.error = None;
        let name = form.stack_name.trim();
        if !name.is_empty() {
            self.config.stack_name = name.to_string();
        }
        self.config.remote_control.enabled = form.api_enabled;
        self.config.remote_control.port = port;
        self.config.log_directory = normalize_log_directory(&form.log_directory);
        self.config.process_error_flash_seconds = flash;
        self.persist_config();
        self.manager
            .set_log_directory(self.config.log_directory.clone());
        self.apply_rest_config();
        self.stack_form = StackSettingsForm::from_config(&self.config);
        self.set_banner("Saved the stack settings");
    }
}

fn label(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(bold(12.0)).color(MUTED));
    ui.add_space(-8.0);
}

fn hint(ui: &mut Ui, text: &str) {
    ui.add_space(-8.0);
    ui.add(egui::Label::new(RichText::new(text).font(body(11.5)).color(FAINT)).wrap());
}

fn about_card(ui: &mut Ui) {
    widgets::card(ui, "About", |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let row = |ui: &mut Ui, name: &str, value: &str, link: Option<&str>| {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(90.0, 18.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.label(RichText::new(name).font(body(12.5)).color(FAINT));
                    },
                );
                match link {
                    Some(url) => {
                        ui.hyperlink_to(RichText::new(value).font(body(12.5)).color(ACCENT), url);
                    }
                    None => {
                        ui.label(RichText::new(value).font(body(12.5)).color(TEXT));
                    }
                }
            });
        };
        row(ui, "Version", env!("CARGO_PKG_VERSION"), None);
        row(
            ui,
            "Author",
            env!("CARGO_PKG_AUTHORS"),
            Some(PROJECT_GITHUB_ACCOUNT_URL),
        );
        row(
            ui,
            "Website",
            PROJECT_WEBSITE_URL,
            Some(PROJECT_WEBSITE_URL),
        );
        row(
            ui,
            "GitHub",
            PROJECT_GITHUB_ACCOUNT_HANDLE,
            Some(PROJECT_GITHUB_ACCOUNT_URL),
        );
        for (name, license) in [
            (
                "Inter font license",
                include_str!("../../assets/fonts/Inter-LICENSE.txt"),
            ),
            (
                "Cascadia Mono font license",
                include_str!("../../assets/fonts/Cascadia-LICENSE.txt"),
            ),
        ] {
            ui.collapsing(RichText::new(name).font(body(12.5)).color(MUTED), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(license).font(mono(11.0)).color(FAINT)).wrap(),
                );
            });
        }
    });
}

fn normalize_log_directory(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    }
}
