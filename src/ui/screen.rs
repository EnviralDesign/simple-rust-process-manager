//! The Screen tab: a terminal process's screen, drawn cell by cell, with the keyboard, paste and
//! mouse going to the program while the screen has focus.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, pos2, text::LayoutJob, vec2, Align, Color32, CornerRadius, Event, EventFilter, FontId,
    Key, Layout, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, TextFormat, Ui,
};

use crate::config::{ProcessConfig, TerminalMode};
use crate::terminal::{term, MouseMode, ScreenSnapshot, Style, TermColor};

use super::icons::Icon;
use super::theme::{self, Tier, *};
use super::widgets::{self, Btn};
use super::ProcessManagerApp;

const FONT_SIZE: f32 = 12.5;
const PAD: f32 = 10.0;
/// Resizes wait this long after the last one, so dragging the window does not flood the program.
const RESIZE_EVERY: Duration = Duration::from_millis(120);

/// What the Screen tab keeps between frames.
#[derive(Default)]
pub(super) struct ScreenState {
    /// The last snapshot, by process and terminal revision.
    cached: Option<(String, u64, ScreenSnapshot)>,
    last_resize: Option<Instant>,
    /// Whether a character's glyph fills exactly its cells in the terminal font.
    fits: HashMap<char, bool>,
    /// The mouse button held down, for drag reports.
    held: Option<u8>,
    /// The selected process's alternate-screen flag last frame, to follow it between tabs.
    pub(super) last_alt: Option<(String, bool)>,
    /// The screen has keyboard focus: the window's shortcuts wait.
    pub(super) focused: bool,
    /// Take keyboard focus on the next frame (`PM_TYPE`).
    pub(super) focus_request: bool,
}

pub(super) fn font() -> FontId {
    mono(FONT_SIZE)
}

/// A style's text and background colours; `None` background means the panel's own.
pub(super) fn style_colors(style: Style, default_fg: Color32) -> (Color32, Option<Color32>) {
    let mut fg = match style.fg {
        TermColor::Default => default_fg,
        // Bold brightens the eight base colours, as terminals do.
        TermColor::Indexed(i) if i < 8 && style.has(term::BOLD) => theme::terminal_color(i + 8),
        TermColor::Indexed(i) => theme::terminal_color(i),
        TermColor::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    };
    let mut bg = match style.bg {
        TermColor::Default => None,
        TermColor::Indexed(i) => Some(theme::terminal_color(i)),
        TermColor::Rgb(r, g, b) => Some(Color32::from_rgb(r, g, b)),
    };
    if style.has(term::INVERSE) {
        let back = bg.unwrap_or(LOG_BG);
        bg = Some(fg);
        fg = back;
    }
    if style.has(term::DIM) {
        fg = fg.gamma_multiply(0.6);
    }
    if style.has(term::HIDDEN) {
        fg = Color32::TRANSPARENT;
    }
    (fg, bg)
}

/// A text format for a styled stretch of terminal text.
pub(super) fn text_format(style: Style, font: FontId, default_fg: Color32) -> TextFormat {
    let (fg, bg) = style_colors(style, default_fg);
    TextFormat {
        font_id: font,
        color: fg,
        background: bg.unwrap_or(Color32::TRANSPARENT),
        italics: style.has(term::ITALIC),
        underline: if style.has(term::UNDERLINE) {
            Stroke::new(1.0_f32, fg)
        } else {
            Stroke::NONE
        },
        strikethrough: if style.has(term::STRIKE) {
            Stroke::new(1.0_f32, fg)
        } else {
            Stroke::NONE
        },
        ..Default::default()
    }
}

impl ProcessManagerApp {
    pub(super) fn draw_screen(
        &mut self,
        ui: &mut Ui,
        process: &ProcessConfig,
        tier: Tier,
        gutter: f32,
    ) {
        let terminal = self.manager.terminal(&process.id);
        let font = font();
        let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(&font, '0'), f.row_height(&font)));

        // Toolbar.
        let outer = ui.available_rect_before_wrap();
        let toolbar = Rect::from_min_size(
            pos2(outer.left() + gutter, outer.top() + 10.0),
            vec2(outer.width() - gutter * 2.0, SMALL_CONTROL_HEIGHT + 4.0),
        );
        ui.allocate_exact_size(
            vec2(outer.width(), 10.0 + SMALL_CONTROL_HEIGHT + 4.0 + 10.0),
            Sense::hover(),
        );
        let outer = ui.available_rect_before_wrap();
        let panel = Rect::from_min_max(
            pos2(outer.left() + gutter, outer.top()),
            pos2(
                outer.right() - gutter,
                outer.bottom() - if tier <= Tier::Compact { 10.0 } else { 14.0 },
            ),
        );
        ui.allocate_rect(outer, Sense::hover());
        let grid_rect = panel.shrink(PAD);
        let want_cols = ((grid_rect.width() / char_w).floor() as usize).max(20);
        let want_rows = ((grid_rect.height() / row_h).floor() as usize).max(5);
        crate::process_manager::set_terminal_size(want_cols, want_rows);

        // The screen, refreshed only when the terminal changed. Full-screen programs (and
        // processes set to open on their screen) are sized to the panel; a log-style program
        // keeps the size it started with, so its lines are not rewrapped.
        let mut snapshot = None;
        if let Some(terminal) = &terminal {
            let mut session = crate::terminal::lock(&terminal);
            let live = session.is_live();
            let fit =
                live && (session.alt_active() || process.terminal_mode == TerminalMode::Screen);
            if fit
                && session.size() != (want_cols, want_rows)
                && self
                    .screen
                    .last_resize
                    .is_none_or(|at| at.elapsed() >= RESIZE_EVERY)
            {
                session.resize(want_cols, want_rows);
                self.screen.last_resize = Some(Instant::now());
            } else if fit && session.size() != (want_cols, want_rows) {
                ui.ctx().request_repaint_after(RESIZE_EVERY);
            }
            let revision = session.revision();
            let fresh = match &self.screen.cached {
                Some((id, rev, snap)) => id == &process.id && *rev == revision && snap.live == live,
                None => false,
            };
            if !fresh {
                self.screen.cached = Some((process.id.clone(), revision, session.snapshot(live)));
            }
            snapshot = self.screen.cached.as_ref().map(|(_, _, snap)| snap.clone());
        }

        let id = ui.id().with(("screen", &process.id));
        let response = ui.interact(panel, id, Sense::click_and_drag());
        if response.clicked()
            || response.drag_started()
            || std::mem::take(&mut self.screen.focus_request)
        {
            response.request_focus();
        }
        let has_focus = |ui: &Ui| ui.memory(|m| m.has_focus(id));
        if response.clicked_elsewhere() && has_focus(ui) {
            ui.memory_mut(|m| m.surrender_focus(id));
        }
        // Focus in memory, not `Response::has_focus`, which also needs the window focused: the
        // screen keeps its focus while you are in another window.
        let focused = has_focus(ui) && snapshot.as_ref().is_some_and(|s| s.live);
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }
        self.screen.focused = focused;

        self.draw_screen_toolbar(ui, toolbar, process, snapshot.as_ref(), focused);

        let painter = ui.painter_at(panel);
        painter.rect(
            panel,
            CornerRadius::same(RADIUS_CARD),
            LOG_BG,
            Stroke::new(1.0_f32, if focused { ACCENT } else { LINE }),
            StrokeKind::Inside,
        );

        let Some(snapshot) = snapshot else {
            let text = if process.terminal_mode == TerminalMode::Off {
                format!(
                    "{} runs without a terminal. Choose one under Output in Settings to see its screen.",
                    process.name
                )
            } else {
                format!("No screen yet. Start {} to see it here.", process.name)
            };
            let g = widgets::galley(ui, &text, body(13.0), MUTED);
            painter.galley(
                pos2(panel.center().x - g.size().x * 0.5, panel.top() + 40.0),
                g,
                MUTED,
            );
            return;
        };

        let origin = grid_rect.min;
        self.paint_grid(ui, &painter, origin, char_w, row_h, &snapshot, focused);

        if focused {
            self.forward_input(ui, &snapshot, origin, char_w, row_h, panel);
        } else if response.clicked() && snapshot.live {
            // The click that focuses also reaches a program that listens to the mouse.
            self.forward_input(ui, &snapshot, origin, char_w, row_h, panel);
        }
        if focused {
            // The cursor and replies to typing should show without waiting for other events.
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }

    fn draw_screen_toolbar(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        process: &ProcessConfig,
        snapshot: Option<&ScreenSnapshot>,
        focused: bool,
    ) {
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        bar.spacing_mut().item_spacing.x = 10.0;
        let (what, detail) = match snapshot {
            None => ("Screen".to_string(), String::new()),
            Some(s) => (
                if !s.live {
                    "Last screen".to_string()
                } else if s.alt {
                    "Full-screen program".to_string()
                } else {
                    "Terminal".to_string()
                },
                format!("{} × {}", s.cols, s.rows),
            ),
        };
        bar.label(egui::RichText::new(what).font(bold(13.0)).color(TEXT));
        if !detail.is_empty() {
            bar.label(egui::RichText::new(detail).font(mono(11.5)).color(FAINT));
        }
        let hint = match snapshot {
            Some(s) if s.live && focused => {
                format!(
                    "Typing goes to {} · click outside the screen to stop",
                    process.name
                )
            }
            Some(s) if s.live => "Click the screen to type into it".to_string(),
            Some(_) => format!("{} has ended; this is what it showed last", process.name),
            None => String::new(),
        };
        if !hint.is_empty() && rect.width() > 520.0 {
            bar.label(
                egui::RichText::new(hint)
                    .font(body(12.0))
                    .color(if focused { ACCENT } else { MUTED }),
            );
        }

        let mut right = bar.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        right.spacing_mut().item_spacing.x = 6.0;
        let live = snapshot.is_some_and(|s| s.live);
        if snapshot.is_some()
            && Btn::new("Copy screen")
                .with_icon(Icon::Copy)
                .small()
                .show(&mut right)
                .on_hover_text("Copy the screen's text")
                .clicked()
        {
            if let Some(terminal) = self.manager.terminal(&process.id) {
                let text = crate::terminal::lock(&terminal).screen_text();
                self.copy_text(&text, "Copied the screen");
            }
        }
        if live
            && Btn::new("Ctrl C")
                .small()
                .show(&mut right)
                .on_hover_text("Send Ctrl+C to the program (interrupt)")
                .clicked()
        {
            self.send_to_terminal(&process.id, vec![0x03]);
        }
    }

    pub(super) fn send_to_terminal(&self, process_id: &str, bytes: Vec<u8>) {
        if let Some(terminal) = self.manager.terminal(process_id) {
            crate::terminal::lock(&terminal).send(bytes);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_grid(
        &mut self,
        ui: &Ui,
        painter: &egui::Painter,
        origin: Pos2,
        char_w: f32,
        row_h: f32,
        snapshot: &ScreenSnapshot,
        focused: bool,
    ) {
        let font = font();
        let cell = |row: usize, col: usize, cells: usize| {
            Rect::from_min_size(
                pos2(
                    origin.x + col as f32 * char_w,
                    origin.y + row as f32 * row_h,
                ),
                vec2(cells as f32 * char_w, row_h),
            )
        };
        // Backgrounds first, then the cursor, then text over both.
        for (r, runs) in snapshot.lines.iter().enumerate() {
            for run in runs {
                if let (_, Some(bg)) = style_colors(run.style, LOG_TEXT) {
                    painter.rect_filled(cell(r, run.col, run.cells), CornerRadius::ZERO, bg);
                }
            }
        }
        if let Some((row, col)) = snapshot.cursor.filter(|_| snapshot.live) {
            let rect = cell(row, col, 1);
            if focused {
                painter.rect_filled(rect, CornerRadius::ZERO, ACCENT.gamma_multiply(0.85));
            } else {
                painter.rect_stroke(
                    rect.shrink(0.5),
                    CornerRadius::ZERO,
                    Stroke::new(1.0_f32, FAINT),
                    StrokeKind::Inside,
                );
            }
        }
        for (r, runs) in snapshot.lines.iter().enumerate() {
            for run in runs {
                if run.text.chars().all(|c| c == ' ') && !run.style.has(term::UNDERLINE) {
                    continue;
                }
                let mut format = text_format(run.style, font.clone(), LOG_TEXT);
                format.background = Color32::TRANSPARENT;
                // Characters whose glyph is not one or two cells wide (a fallback font's) are
                // placed on their own, so the rest of the row stays on the grid.
                let mut col = run.col;
                let mut segment = String::new();
                let mut segment_col = col;
                for c in run.text.chars() {
                    let width = if unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) >= 2 {
                        2
                    } else {
                        1
                    };
                    let fits = width == 1
                        && *self.screen.fits.entry(c).or_insert_with(|| {
                            let w = ui.fonts_mut(|f| f.glyph_width(&font, c));
                            (w - char_w).abs() < 0.5
                        });
                    if fits {
                        if segment.is_empty() {
                            segment_col = col;
                        }
                        segment.push(c);
                    } else {
                        if !segment.is_empty() {
                            paint_text(ui, painter, cell(r, segment_col, 1).min, &segment, &format);
                            segment.clear();
                        }
                        let rect = cell(r, col, width);
                        let mut job = LayoutJob::default();
                        job.append(&c.to_string(), 0.0, format.clone());
                        let galley = ui.fonts_mut(|f| f.layout_job(job));
                        let x = rect.center().x - galley.size().x * 0.5;
                        painter.galley(pos2(x, rect.top()), galley, format.color);
                    }
                    col += width;
                }
                if !segment.is_empty() {
                    paint_text(ui, painter, cell(r, segment_col, 1).min, &segment, &format);
                }
            }
        }
    }

    fn forward_input(
        &mut self,
        ui: &Ui,
        snapshot: &ScreenSnapshot,
        origin: Pos2,
        char_w: f32,
        row_h: f32,
        panel: Rect,
    ) {
        let Some(process_id) = self.selected_process.clone() else {
            return;
        };
        let modes = snapshot.modes;
        let events = ui.input(|i| i.events.clone());
        // Alt types text on its own as Meta, except on a Mac, where Option is for characters.
        let held_alt =
            !cfg!(target_os = "macos") && ui.input(|i| i.modifiers.alt && !i.modifiers.ctrl);
        let (c_down, x_down) = ui.input(|i| (i.key_down(Key::C), i.key_down(Key::X)));
        let cell_at = |pos: Pos2| {
            let col = ((pos.x - origin.x) / char_w).floor().max(0.0) as usize;
            let row = ((pos.y - origin.y) / row_h).floor().max(0.0) as usize;
            (
                col.min(snapshot.cols.saturating_sub(1)),
                row.min(snapshot.rows.saturating_sub(1)),
            )
        };
        let mut out: Vec<u8> = Vec::new();
        for event in events {
            match event {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if let Some(bytes) = encode_key(key, modifiers, modes.app_cursor) {
                        out.extend(bytes);
                    }
                }
                Event::Text(text) if !held_alt => out.extend(text.as_bytes()),
                Event::Paste(text) => {
                    let text = text.replace("\r\n", "\r").replace('\n', "\r");
                    if modes.bracketed_paste {
                        out.extend(b"\x1b[200~");
                        out.extend(text.as_bytes());
                        out.extend(b"\x1b[201~");
                    } else {
                        out.extend(text.as_bytes());
                    }
                }
                // Ctrl+C and Ctrl+X arrive as copy and cut (so do Ctrl+Insert and Shift+Delete,
                // which stay out of the program). On a Mac, Command+C copies and Control+C
                // reaches the program as a key.
                Event::Copy if !cfg!(target_os = "macos") && c_down => out.push(0x03),
                Event::Cut if !cfg!(target_os = "macos") && x_down => out.push(0x18),
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } if modes.mouse != MouseMode::Off && panel.contains(pos) => {
                    let code = match button {
                        PointerButton::Primary => 0,
                        PointerButton::Middle => 1,
                        PointerButton::Secondary => 2,
                        _ => continue,
                    };
                    self.screen.held = pressed.then_some(code);
                    let (col, row) = cell_at(pos);
                    out.extend(mouse_report(
                        code,
                        col,
                        row,
                        pressed,
                        modifiers,
                        modes.mouse_sgr,
                    ));
                }
                Event::PointerMoved(pos)
                    if panel.contains(pos)
                        && (modes.mouse == MouseMode::Motion
                            || (modes.mouse == MouseMode::Drag && self.screen.held.is_some())) =>
                {
                    let (col, row) = cell_at(pos);
                    let code = self.screen.held.unwrap_or(3) + 32;
                    out.extend(mouse_report(
                        code,
                        col,
                        row,
                        true,
                        Modifiers::NONE,
                        modes.mouse_sgr,
                    ));
                }
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } => {
                    let steps = match unit {
                        egui::MouseWheelUnit::Line => delta.y.abs().ceil(),
                        egui::MouseWheelUnit::Point => (delta.y.abs() / row_h).round(),
                        egui::MouseWheelUnit::Page => 3.0,
                    } as usize;
                    if delta.y == 0.0 || steps == 0 {
                        continue;
                    }
                    let up = delta.y > 0.0;
                    let steps = steps.min(5);
                    let pos = ui
                        .input(|i| i.pointer.hover_pos())
                        .unwrap_or(panel.center());
                    for _ in 0..steps {
                        if modes.mouse != MouseMode::Off {
                            let (col, row) = cell_at(pos);
                            let code = if up { 64 } else { 65 };
                            out.extend(mouse_report(
                                code,
                                col,
                                row,
                                true,
                                modifiers,
                                modes.mouse_sgr,
                            ));
                        } else if snapshot.alt {
                            // A full-screen program without the mouse scrolls with arrows.
                            let key = if up { Key::ArrowUp } else { Key::ArrowDown };
                            if let Some(bytes) = encode_key(key, Modifiers::NONE, modes.app_cursor)
                            {
                                out.extend(bytes);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !out.is_empty() {
            self.send_to_terminal(&process_id, out);
        }
    }
}

/// `PM_TYPE`'s keys as the input events a keyboard would make.
pub(super) fn typed_events(keys: &str) -> Vec<Event> {
    let key = |key: Key, modifiers: Modifiers| Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    let mut events = Vec::new();
    let mut rest = keys;
    while !rest.is_empty() {
        if let Some(inner) = rest.strip_prefix('{').and_then(|r| r.split_once('}')) {
            let (name, after) = inner;
            let named = match name {
                "enter" => Some(key(Key::Enter, Modifiers::NONE)),
                "tab" => Some(key(Key::Tab, Modifiers::NONE)),
                "esc" => Some(key(Key::Escape, Modifiers::NONE)),
                "up" => Some(key(Key::ArrowUp, Modifiers::NONE)),
                "down" => Some(key(Key::ArrowDown, Modifiers::NONE)),
                "left" => Some(key(Key::ArrowLeft, Modifiers::NONE)),
                "right" => Some(key(Key::ArrowRight, Modifiers::NONE)),
                _ => name
                    .strip_prefix("ctrl-")
                    .and_then(|letter| Key::from_name(&letter.to_ascii_uppercase()))
                    .map(|k| key(k, Modifiers::CTRL)),
            };
            if let Some(event) = named {
                events.push(event);
                rest = after;
                continue;
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        events.push(Event::Text(c.to_string()));
        rest = &rest[c.len_utf8()..];
    }
    events
}

fn paint_text(ui: &Ui, painter: &egui::Painter, at: Pos2, text: &str, format: &TextFormat) {
    let mut job = LayoutJob::default();
    job.append(text, 0.0, format.clone());
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    painter.galley(at, galley, format.color);
}

/// The bytes an xterm sends for a key, or `None` for keys that arrive as text instead.
pub(super) fn encode_key(key: Key, modifiers: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    // xterm's modifier parameter: 1 + Shift 1 + Alt 2 + Ctrl 4.
    let m =
        1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl);
    let csi = |suffix: char| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[1;{m}{suffix}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{suffix}").into_bytes()
        } else {
            format!("\x1b[{suffix}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[{n};{m}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let ss3 = |suffix: char| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[1;{m}{suffix}").into_bytes()
        } else {
            format!("\x1bO{suffix}").into_bytes()
        }
    };
    let alt = |bytes: Vec<u8>| -> Vec<u8> {
        if modifiers.alt {
            let mut out = vec![0x1b];
            out.extend(bytes);
            out
        } else {
            bytes
        }
    };
    Some(match key {
        Key::ArrowUp => csi('A'),
        Key::ArrowDown => csi('B'),
        Key::ArrowRight => csi('C'),
        Key::ArrowLeft => csi('D'),
        Key::Home => csi('H'),
        Key::End => csi('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => ss3('P'),
        Key::F2 => ss3('Q'),
        Key::F3 => ss3('R'),
        Key::F4 => ss3('S'),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Enter => alt(vec![b'\r']),
        Key::Backspace if modifiers.ctrl => alt(vec![0x08]),
        Key::Backspace => alt(vec![0x7f]),
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![b'\t'],
        Key::Escape => vec![0x1b],
        // Ctrl with a letter or symbol is a control character. Ctrl+Alt is AltGr on many
        // layouts and types text, so it is left alone.
        _ if modifiers.ctrl && !modifiers.alt => {
            let byte = match key {
                Key::Space | Key::Num2 => 0x00,
                Key::OpenBracket => 0x1b,
                Key::Backslash => 0x1c,
                Key::CloseBracket => 0x1d,
                Key::Num6 => 0x1e,
                Key::Minus => 0x1f,
                // Ctrl+V pastes (it arrives as a paste), except on a Mac, where Command+V does.
                Key::V if !cfg!(target_os = "macos") => return None,
                _ => {
                    let name = key.name();
                    let letter = name.chars().next().filter(|_| name.len() == 1)?;
                    if !letter.is_ascii_alphabetic() {
                        return None;
                    }
                    letter.to_ascii_uppercase() as u8 - b'A' + 1
                }
            };
            alt(vec![byte])
        }
        // Alt with a letter or digit is Escape then the character (its text event is dropped).
        // Not on a Mac, where Option types characters (@, [, { on many layouts).
        _ if modifiers.alt && !modifiers.ctrl && !cfg!(target_os = "macos") => {
            let name = key.name();
            let c = name.chars().next().filter(|_| name.len() == 1)?;
            if !c.is_ascii_alphanumeric() {
                return None;
            }
            let c = if modifiers.shift {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            };
            vec![0x1b, c as u8]
        }
        _ => return None,
    })
}

/// A mouse report: SGR (`ESC [ < b ; x ; y M/m`) when the program asked for it, else X10.
fn mouse_report(
    code: u8,
    col: usize,
    row: usize,
    pressed: bool,
    modifiers: Modifiers,
    sgr: bool,
) -> Vec<u8> {
    let code = code
        + 4 * u8::from(modifiers.shift)
        + 8 * u8::from(modifiers.alt)
        + 16 * u8::from(modifiers.ctrl);
    if sgr {
        format!(
            "\x1b[<{};{};{}{}",
            code,
            col + 1,
            row + 1,
            if pressed { 'M' } else { 'm' }
        )
        .into_bytes()
    } else {
        let code = if pressed { code } else { 3 };
        vec![
            0x1b,
            b'[',
            b'M',
            32 + code,
            (33 + col).min(255) as u8,
            (33 + row).min(255) as u8,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: Key, m: Modifiers) -> Vec<u8> {
        encode_key(k, m, false).unwrap()
    }

    #[test]
    fn keys_encode_as_xterm_does() {
        assert_eq!(key(Key::ArrowUp, Modifiers::NONE), b"\x1b[A");
        assert_eq!(
            encode_key(Key::ArrowUp, Modifiers::NONE, true).unwrap(),
            b"\x1bOA"
        );
        assert_eq!(key(Key::ArrowLeft, Modifiers::CTRL), b"\x1b[1;5D");
        assert_eq!(key(Key::Delete, Modifiers::NONE), b"\x1b[3~");
        assert_eq!(key(Key::F5, Modifiers::SHIFT), b"\x1b[15;2~");
        assert_eq!(key(Key::Enter, Modifiers::NONE), b"\r");
        assert_eq!(key(Key::Backspace, Modifiers::NONE), b"\x7f");
        assert_eq!(key(Key::Tab, Modifiers::SHIFT), b"\x1b[Z");
        assert_eq!(key(Key::D, Modifiers::CTRL), [0x04]);
        assert_eq!(key(Key::B, Modifiers::ALT), [0x1b, b'b']);
        assert_eq!(encode_key(Key::A, Modifiers::NONE, false), None);
        assert_eq!(encode_key(Key::V, Modifiers::CTRL, false), None);
    }

    #[test]
    fn mouse_reports_in_both_encodings() {
        assert_eq!(
            mouse_report(0, 4, 2, true, Modifiers::NONE, true),
            b"\x1b[<0;5;3M"
        );
        assert_eq!(
            mouse_report(0, 4, 2, false, Modifiers::NONE, true),
            b"\x1b[<0;5;3m"
        );
        assert_eq!(
            mouse_report(64, 0, 0, true, Modifiers::NONE, false),
            vec![0x1b, b'[', b'M', 96, 33, 33]
        );
    }

    #[test]
    fn inverse_swaps_colours_and_bold_brightens() {
        let mut style = Style::default();
        style.fg = TermColor::Indexed(1);
        style.flags = term::BOLD;
        assert_eq!(style_colors(style, LOG_TEXT).0, theme::terminal_color(9));
        style.flags = term::INVERSE;
        let (fg, bg) = style_colors(style, LOG_TEXT);
        assert_eq!(fg, LOG_BG);
        assert_eq!(bg, Some(theme::terminal_color(1)));
    }
}
