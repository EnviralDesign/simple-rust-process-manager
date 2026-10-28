//! Small confirmations: delete a process, reload processes.json, rename a group, and leave
//! settings that have not been saved.

use eframe::egui::{self, vec2, Align, Context, CornerRadius, Layout, RichText, Stroke, Ui};

use super::theme::*;
use super::widgets::{self, Btn};
use super::ProcessManagerApp;

enum Answer {
    Confirm,
    Alternate,
    Cancel,
}

/// A centred dialog: title, body, and buttons on the right (`confirm` is the primary one).
fn dialog(
    ctx: &Context,
    id: &str,
    title: &str,
    body: impl FnOnce(&mut Ui),
    confirm: &str,
    alternate: Option<&str>,
    cancel: &str,
) -> Option<Answer> {
    let mut answer = None;
    let response = egui::Modal::new(egui::Id::new(id))
        .backdrop_color(egui::Color32::from_black_alpha(140))
        .frame(
            egui::Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0_f32, BORDER))
                .corner_radius(CornerRadius::same(RADIUS_CARD + 2))
                .inner_margin(egui::Margin::same(20))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 24],
                    blur: 80,
                    spread: 0,
                    color: egui::Color32::from_black_alpha(180),
                }),
        )
        .show(ctx, |ui| {
            let width = (ctx.content_rect().width() - 64.0).min(420.0);
            ui.set_width(width);
            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
            ui.label(RichText::new(title).font(bold(17.0)).color(TEXT));
            body(ui);
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if Btn::new(confirm).primary().show(ui).clicked() {
                    answer = Some(Answer::Confirm);
                }
                if let Some(alternate) = alternate {
                    if Btn::new(alternate).show(ui).clicked() {
                        answer = Some(Answer::Alternate);
                    }
                }
                if Btn::new(cancel).show(ui).clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });
    if response.should_close() && answer.is_none() {
        answer = Some(Answer::Cancel);
    }
    answer
}

fn paragraph(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(body(13.0)).color(MUTED)).wrap());
}

impl ProcessManagerApp {
    pub(super) fn draw_dialogs(&mut self, ctx: &Context) {
        if let Some(id) = self.delete_process_id.clone() {
            let process = self.config.get_process(&id).cloned();
            let name = process
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "this process".into());
            let running = self.is_live(&id);
            let logs = process.as_ref().is_some_and(|p| p.log_to_disk);
            let text = format!(
                "It is removed from processes.json{}.{}",
                if running { " and stopped first" } else { "" },
                if logs {
                    " Its session logs on disk stay where they are."
                } else {
                    ""
                }
            );
            match dialog(
                ctx,
                "delete_process",
                &format!("Delete {name}?"),
                |ui| paragraph(ui, &text),
                "Delete",
                None,
                "Cancel",
            ) {
                Some(Answer::Confirm) => {
                    self.delete_process(&id);
                    self.delete_process_id = None;
                }
                Some(_) => self.delete_process_id = None,
                None => {}
            }
        }

        if self.reload_processes_confirm_open {
            match dialog(
                ctx,
                "reload_config",
                "Reload processes.json?",
                |ui| {
                    paragraph(ui, "Every process stops first, including those that skip Stop all. Then the file is read again and the list rebuilt.");
                },
                "Reload",
                None,
                "Cancel",
            ) {
                Some(Answer::Confirm) => {
                    self.reload_processes_confirm_open = false;
                    self.reload_processes_from_disk();
                }
                Some(_) => self.reload_processes_confirm_open = false,
                None => {}
            }
        }

        if let Some((id, mut name)) = self.rename_group.clone() {
            let mut submit = false;
            let answer = dialog(
                ctx,
                "rename_group",
                "Name the group",
                |ui| {
                    let width = ui.available_width();
                    let field = widgets::text_field(
                        ui,
                        "group_name",
                        &mut name,
                        "Group name",
                        width,
                        false,
                    );
                    if !field.has_focus() && !field.lost_focus() {
                        field.request_focus();
                    }
                    submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                },
                "Save",
                None,
                "Cancel",
            );
            match answer {
                Some(Answer::Confirm) => {
                    self.rename_group_to(&id, &name);
                    self.rename_group = None;
                }
                Some(_) => self.rename_group = None,
                None if submit => {
                    self.rename_group_to(&id, &name);
                    self.rename_group = None;
                }
                None => self.rename_group = Some((id, name)),
            }
        }

        if let Some(nav) = self.pending_nav.clone() {
            let name = self.editor.as_ref().map(|e| e.name()).unwrap_or_default();
            let answer = dialog(
                ctx,
                "unsaved_settings",
                &format!("Save the changes to {name}?"),
                |ui| paragraph(ui, "You changed its settings and have not saved them."),
                "Save",
                Some("Discard"),
                "Keep editing",
            );
            match answer {
                Some(Answer::Confirm) => {
                    self.save_editor(false);
                    if self.editor.as_ref().is_some_and(|e| e.is_dirty()) {
                        // The draft did not validate; stay with it.
                        self.pending_nav = None;
                    } else {
                        self.go(nav);
                    }
                }
                Some(Answer::Alternate) => {
                    self.editor = None;
                    self.go(nav);
                }
                Some(Answer::Cancel) => self.pending_nav = None,
                None => {}
            }
        }
    }
}
