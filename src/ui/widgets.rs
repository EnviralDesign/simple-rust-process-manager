//! The widget kit: every control is painted to the mockup's measurements so the app looks the
//! same at every DPI, and none of it depends on egui's default widget look.

use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, FontId, Frame, Galley, Margin, Painter, Pos2, Rect,
    Response, RichText, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui,
};
use std::sync::Arc;

use super::icons::{self, Icon};
use super::theme::*;

/// The status lights: the only coloured things in the app besides the accent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Light {
    Running,
    Busy,
    Stopped,
    Scheduled,
    Error,
}

/// Paints a status light (8 pt) centred on `center`.
pub fn paint_light(ui: &Ui, painter: &Painter, center: Pos2, light: Light) {
    let r = 4.0;
    match light {
        Light::Running => {
            painter.circle_filled(
                center,
                r + 3.0,
                Color32::from_rgba_unmultiplied(0x5f, 0xbf, 0x62, 40),
            );
            painter.circle_filled(center, r, GREEN);
        }
        Light::Error => {
            painter.circle_filled(
                center,
                r + 3.0,
                Color32::from_rgba_unmultiplied(0xe2, 0x5f, 0x5f, 50),
            );
            painter.circle_filled(center, r, RED);
        }
        Light::Busy => {
            let t = ui.input(|i| i.time);
            let alpha = 0.35 + 0.65 * (0.5 + 0.5 * (t * std::f64::consts::TAU).cos()) as f32;
            painter.circle_filled(center, r, AMBER.gamma_multiply(alpha));
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        Light::Stopped => {
            painter.circle_filled(center, r, GREY);
        }
        Light::Scheduled => {
            painter.circle_stroke(center, r - 0.75, Stroke::new(1.5_f32, MUTED));
        }
    }
}

/// A light as its own 8 × 8 widget, for flowing layouts.
pub fn light(ui: &mut Ui, light: Light) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
    paint_light(ui, ui.painter(), rect.center(), light);
    response
}

/// Text laid out without a colour of its own: the colour passed when painting it is used.
/// (`_color` documents the intended colour at the call site.)
pub fn galley(ui: &Ui, text: impl Into<String>, font: FontId, _color: Color32) -> Arc<Galley> {
    ui.painter()
        .layout_no_wrap(text.into(), font, Color32::PLACEHOLDER)
}

/// Text that is cut with an ellipsis to fit `max_width`.
pub fn galley_truncated(
    ui: &Ui,
    text: impl Into<String>,
    font: FontId,
    _color: Color32,
    max_width: f32,
) -> Arc<Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.into(), font, Color32::PLACEHOLDER);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_width.max(0.0));
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

pub fn paint_galley_left(
    painter: &Painter,
    left_center: Pos2,
    galley: Arc<Galley>,
    color: Color32,
) {
    let pos = pos2(left_center.x, left_center.y - galley.size().y * 0.5);
    painter.galley(pos, galley, color);
}

/// Uppercase section label: bold 10.5 pt, tracked out, faint.
pub fn caps(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .font(bold(10.5))
        .extra_letter_spacing(0.9)
        .color(FAINT)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Secondary,
    Primary,
    Ghost,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Normal,
    Small,
    XSmall,
}

impl Size {
    fn height(self) -> f32 {
        match self {
            Self::Normal => CONTROL_HEIGHT,
            Self::Small => SMALL_CONTROL_HEIGHT,
            Self::XSmall => 22.0,
        }
    }
    fn pad(self) -> f32 {
        match self {
            Self::Normal => 12.0,
            Self::Small => 9.0,
            Self::XSmall => 7.0,
        }
    }
    fn font(self) -> f32 {
        match self {
            Self::Normal => 12.5,
            Self::Small => 12.0,
            Self::XSmall => 11.5,
        }
    }
}

/// A button: label, icon or both, in one of three kinds and three sizes.
pub struct Btn<'a> {
    label: Option<&'a str>,
    icon: Option<Icon>,
    kind: Kind,
    size: Size,
    enabled: bool,
    selected: bool,
    min_width: f32,
    text_color: Option<Color32>,
    kbd: Option<&'a str>,
}

impl<'a> Btn<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label: Some(label),
            icon: None,
            kind: Kind::Secondary,
            size: Size::Normal,
            enabled: true,
            selected: false,
            min_width: 0.0,
            text_color: None,
            kbd: None,
        }
    }
    pub fn icon(icon: Icon) -> Self {
        Self {
            label: None,
            icon: Some(icon),
            ..Self::new("")
        }
    }
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn primary(mut self) -> Self {
        self.kind = Kind::Primary;
        self
    }
    pub fn ghost(mut self) -> Self {
        self.kind = Kind::Ghost;
        self
    }
    pub fn small(mut self) -> Self {
        self.size = Size::Small;
        self
    }
    pub fn xsmall(mut self) -> Self {
        self.size = Size::XSmall;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
    pub fn text_color(mut self, color: Color32) -> Self {
        self.text_color = Some(color);
        self
    }
    pub fn kbd(mut self, kbd: &'a str) -> Self {
        self.kbd = Some(kbd);
        self
    }

    /// The width this button takes, for layouts that measure before placing.
    pub fn width(&self, ui: &Ui) -> f32 {
        self.layout(ui).0
    }

    fn layout(&self, ui: &Ui) -> (f32, Option<Arc<Galley>>, Option<Arc<Galley>>, f32) {
        let height = self.size.height();
        let icon_size = if self.label.is_some() { 13.0 } else { 15.0 };
        let label = self
            .label
            .filter(|label| !label.is_empty())
            .map(|label| galley(ui, label, bold(self.size.font()), TEXT));
        let kbd = self.kbd.map(|kbd| galley(ui, kbd, mono(10.5), FAINT));
        let width = match (&label, self.icon) {
            (None, Some(_)) => height,
            (Some(label), icon) => {
                let mut w = self.size.pad() * 2.0 + label.size().x;
                if icon.is_some() {
                    w += icon_size + 7.0;
                }
                if let Some(kbd) = &kbd {
                    w += kbd.size().x + 10.0 + 10.0;
                }
                w
            }
            (None, None) => height,
        };
        (width.max(self.min_width), label, kbd, icon_size)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let (width, label, kbd, icon_size) = self.layout(ui);
        let height = self.size.height();
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
        self.paint(ui, rect, &response, label, kbd, icon_size);
        response
    }

    /// Paints into a rect the caller already allocated.
    pub fn show_in(self, ui: &mut Ui, rect: Rect, id: egui::Id) -> Response {
        let (_, label, kbd, icon_size) = self.layout(ui);
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, id, sense);
        self.paint(ui, rect, &response, label, kbd, icon_size);
        response
    }

    fn paint(
        &self,
        ui: &Ui,
        rect: Rect,
        response: &Response,
        label: Option<Arc<Galley>>,
        kbd: Option<Arc<Galley>>,
        icon_size: f32,
    ) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        let hovered = self.enabled && response.hovered();
        let pressed = self.enabled && response.is_pointer_button_down_on();
        let painter = ui.painter();
        let radius = CornerRadius::same(RADIUS);
        let (fill, stroke, mut text) = match self.kind {
            Kind::Primary => (
                if hovered { ACCENT_HOVER } else { ACCENT },
                Stroke::NONE,
                ON_ACCENT,
            ),
            Kind::Secondary => (
                if self.selected {
                    accent_soft()
                } else if hovered || pressed {
                    BUTTON_HOVER
                } else {
                    BUTTON
                },
                Stroke::new(
                    1.0_f32,
                    if self.selected {
                        ACCENT
                    } else if hovered {
                        BORDER_HI
                    } else {
                        BORDER
                    },
                ),
                TEXT,
            ),
            Kind::Ghost => (
                if self.selected {
                    SELECTED
                } else if hovered || pressed {
                    HOVER
                } else {
                    Color32::TRANSPARENT
                },
                if hovered {
                    Stroke::new(1.0_f32, BORDER)
                } else {
                    Stroke::NONE
                },
                if hovered || self.selected {
                    TEXT
                } else {
                    MUTED
                },
            ),
        };
        if let Some(color) = self.text_color {
            text = color;
        }
        if !self.enabled {
            text = text.gamma_multiply(0.45);
        }
        painter.rect(rect, radius, fill, stroke, StrokeKind::Inside);
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(RADIUS + 2),
                Stroke::new(2.0_f32, ACCENT),
                StrokeKind::Inside,
            );
        }

        match (label, self.icon) {
            (None, Some(icon)) => {
                icons::paint(painter, rect, icon_size, icon, text);
            }
            (Some(label), icon) => {
                let mut content_w = label.size().x;
                if icon.is_some() {
                    content_w += icon_size + 7.0;
                }
                let kbd_w = kbd.as_ref().map_or(0.0, |k| k.size().x + 10.0);
                let mut x = if kbd.is_some() {
                    rect.left() + self.size.pad()
                } else {
                    rect.center().x - content_w * 0.5
                };
                if let Some(icon) = icon {
                    icons::paint(
                        painter,
                        icons::square(pos2(x + icon_size * 0.5, rect.center().y), icon_size),
                        icon_size,
                        icon,
                        text,
                    );
                    x += icon_size + 7.0;
                }
                paint_galley_left(painter, pos2(x, rect.center().y), label, text);
                if let Some(kbd) = kbd {
                    let kbd_rect = Rect::from_center_size(
                        pos2(
                            rect.right() - self.size.pad() - kbd_w * 0.5 + 5.0,
                            rect.center().y,
                        ),
                        vec2(kbd.size().x + 10.0, 18.0),
                    );
                    paint_kbd_box(painter, kbd_rect);
                    painter.galley(kbd_rect.center() - kbd.size() * 0.5, kbd, FAINT);
                }
            }
            (None, None) => {}
        }
    }
}

fn paint_kbd_box(painter: &Painter, rect: Rect) {
    painter.rect(
        rect,
        CornerRadius::same(4),
        PANEL,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    painter.hline(
        rect.x_range().shrink(1.0),
        rect.bottom() - 0.5,
        Stroke::new(1.0_f32, BORDER),
    );
}

/// A filter chip: rounded, with an optional light and a count.
pub fn chip(
    ui: &mut Ui,
    label: &str,
    count: Option<&str>,
    on: bool,
    light: Option<Light>,
) -> Response {
    let label_g = galley(ui, label, bold(12.0), MUTED);
    let count_g = count.map(|count| galley(ui, count, body(12.0), FAINT));
    let mut width = 20.0 + label_g.size().x;
    if light.is_some() {
        width += 8.0 + 6.0;
    }
    if let Some(count) = &count_g {
        width += 6.0 + count.size().x;
    }
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, SMALL_CONTROL_HEIGHT), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let painter = ui.painter();
        let radius = CornerRadius::same(13);
        if on {
            painter.rect(
                rect,
                radius,
                chip_on_fill(),
                Stroke::new(1.0_f32, chip_on_border()),
                StrokeKind::Inside,
            );
        } else if hovered {
            painter.rect_filled(rect, radius, HOVER);
        }
        let text = if on || hovered { TEXT } else { MUTED };
        let mut x = rect.left() + 10.0;
        if let Some(light) = light {
            paint_light(ui, painter, pos2(x + 4.0, rect.center().y), light);
            x += 14.0;
        }
        let label_w = label_g.size().x;
        paint_galley_left(painter, pos2(x, rect.center().y), label_g, text);
        x += label_w + 6.0;
        if let Some(count) = count_g {
            let color = if on { TEXT.gamma_multiply(0.7) } else { FAINT };
            paint_galley_left(painter, pos2(x, rect.center().y), count, color);
        }
    }
    response
}

/// A small on/off button used in toolbars (Time, Wrap, Follow).
pub fn toggle(ui: &mut Ui, label: &str, icon: Option<Icon>, on: bool) -> Response {
    let label_g = galley(ui, label, bold(12.0), MUTED);
    let mut width = 18.0 + label_g.size().x;
    if icon.is_some() {
        width += 13.0 + 6.0;
    }
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, SMALL_CONTROL_HEIGHT), Sense::click());
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let painter = ui.painter();
        let (fill, border, text) = if on {
            (accent_soft(), ACCENT, TEXT)
        } else if hovered {
            (BUTTON, BORDER_HI, TEXT)
        } else {
            (BUTTON, BORDER, MUTED)
        };
        painter.rect(
            rect,
            CornerRadius::same(RADIUS),
            fill,
            Stroke::new(1.0_f32, border),
            StrokeKind::Inside,
        );
        let mut x = rect.left() + 9.0;
        if let Some(icon) = icon {
            icons::paint(
                painter,
                icons::square(pos2(x + 6.5, rect.center().y), 13.0),
                13.0,
                icon,
                text,
            );
            x += 19.0;
        }
        paint_galley_left(painter, pos2(x, rect.center().y), label_g, text);
    }
    response
}

/// A 36 × 20 switch. Returns the response; flips `on` when clicked.
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(response.id, *on, 0.12);
        let painter = ui.painter();
        let radius = CornerRadius::same(10);
        let fill = lerp_color(BG, ACCENT, t);
        let border = lerp_color(
            if response.hovered() {
                BORDER_HI
            } else {
                BORDER
            },
            ACCENT,
            t,
        );
        painter.rect(
            rect,
            radius,
            fill,
            Stroke::new(1.0_f32, border),
            StrokeKind::Inside,
        );
        let knob_x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
        painter.circle_filled(
            pos2(knob_x, rect.center().y),
            7.0,
            lerp_color(FAINT, ON_ACCENT, t),
        );
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                CornerRadius::same(12),
                Stroke::new(2.0_f32, ACCENT),
                StrokeKind::Inside,
            );
        }
    }
    response
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

/// A segmented control. Returns the index clicked this frame, if any.
pub fn segmented(ui: &mut Ui, options: &[&str], selected: usize) -> Option<usize> {
    let galleys: Vec<_> = options
        .iter()
        .map(|o| galley(ui, *o, bold(12.0), MUTED))
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 24.0).collect();
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(total, 28.0), Sense::hover());
    let painter = ui.painter().clone();
    let radius = CornerRadius::same(RADIUS);
    painter.rect(rect, radius, BG, Stroke::NONE, StrokeKind::Inside);
    let mut clicked = None;
    let mut x = rect.left();
    for (index, (galley, width)) in galleys.into_iter().zip(widths).enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.top()), vec2(width, rect.height()));
        let response = ui.interact(
            seg,
            ui.id().with(("segment", index, options[index])),
            Sense::click(),
        );
        let on = index == selected;
        if on {
            let r = CornerRadius {
                nw: if index == 0 { RADIUS } else { 0 },
                sw: if index == 0 { RADIUS } else { 0 },
                ne: if index + 1 == options.len() {
                    RADIUS
                } else {
                    0
                },
                se: if index + 1 == options.len() {
                    RADIUS
                } else {
                    0
                },
            };
            painter.rect_filled(seg, r, chip_on_fill());
        }
        let color = if on || response.hovered() {
            TEXT
        } else {
            MUTED
        };
        painter.galley(seg.center() - galley.size() * 0.5, galley, color);
        if index > 0 {
            painter.vline(seg.left(), seg.y_range(), Stroke::new(1.0_f32, BORDER));
        }
        if response.clicked() {
            clicked = Some(index);
        }
        x += width;
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0_f32, BORDER),
        StrokeKind::Inside,
    );
    clicked
}

/// A quiet bordered tag (badge) such as "Auto-start".
pub fn badge(ui: &mut Ui, text: &str, small: bool) -> Response {
    let font = bold(if small { 10.5 } else { 11.5 });
    let galley = galley(ui, text, font, MUTED);
    let (h, pad) = if small { (19.0, 6.0) } else { (22.0, 8.0) };
    let (rect, response) =
        ui.allocate_exact_size(vec2(galley.size().x + pad * 2.0, h), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect(
            rect,
            CornerRadius::same(RADIUS),
            PANEL,
            Stroke::new(1.0_f32, LINE),
            StrokeKind::Inside,
        );
        ui.painter()
            .galley(rect.center() - galley.size() * 0.5, galley, MUTED);
    }
    response
}

/// A single-line text field with the kit's frame. `mono` uses the code font.
pub fn text_field(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    value: &mut String,
    hint: &str,
    width: f32,
    mono_font: bool,
) -> Response {
    let height = CONTROL_HEIGHT;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let font = if mono_font { mono(12.5) } else { body(13.0) };
    let inner = rect.shrink2(vec2(10.0, 0.0));
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), BG);
    let response = put_free(
        ui,
        inner,
        TextEdit::singleline(value)
            .id_salt(id_salt)
            .frame(false)
            .hint_text(RichText::new(hint).color(FAINT))
            .font(font)
            .text_color(TEXT)
            .margin(Margin::symmetric(0, 0))
            .desired_width(inner.width())
            .vertical_align(egui::Align::Center),
    );
    let border = if response.has_focus() {
        ACCENT
    } else if response.hovered() {
        BORDER_HI
    } else {
        BORDER
    };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    response
}

/// The search field: icon, text, and a quiet trailing note such as a match count.
pub fn search_field(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash,
    value: &mut String,
    hint: &str,
    width: f32,
    trailing: Option<&str>,
) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::hover());
    let trailing_g = trailing.map(|t| galley(ui, t, body(11.0), MUTED));
    let trailing_w = trailing_g.as_ref().map_or(0.0, |g| g.size().x + 8.0);
    let inner = Rect::from_min_max(
        pos2(rect.left() + 31.0, rect.top()),
        pos2(rect.right() - 9.0 - trailing_w, rect.bottom()),
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(RADIUS), BUTTON);
    let response = put_free(
        ui,
        inner,
        TextEdit::singleline(value)
            .id_salt(id_salt)
            .frame(false)
            .hint_text(RichText::new(hint).color(FAINT))
            .font(body(12.5))
            .text_color(TEXT)
            .margin(Margin::symmetric(0, 0))
            .desired_width(inner.width())
            .vertical_align(egui::Align::Center),
    );
    let painter = ui.painter();
    let border = if response.has_focus() { ACCENT } else { BORDER };
    painter.rect_stroke(
        rect,
        CornerRadius::same(RADIUS),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    icons::paint(
        painter,
        icons::square(pos2(rect.left() + 16.0, rect.center().y), 15.0),
        15.0,
        Icon::Search,
        FAINT,
    );
    if let Some(g) = trailing_g {
        let pos = pos2(
            rect.right() - 9.0 - g.size().x,
            rect.center().y - g.size().y * 0.5,
        );
        painter.galley(pos, g, MUTED);
    }
    response
}

/// A bordered card with an uppercase title, for settings sections.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(Margin {
            left: 16,
            right: 16,
            top: 14,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(caps(title));
            ui.add_space(10.0);
            add(ui)
        })
        .inner
}

/// The frame for menus and popovers.
pub fn popup_frame() -> Frame {
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(Margin::same(5))
        .shadow(egui::epaint::Shadow {
            offset: [0, 14],
            blur: 40,
            spread: 0,
            color: Color32::from_black_alpha(150),
        })
}

/// A full-width menu entry, with an optional key hint on the right.
pub fn menu_item(
    ui: &mut Ui,
    label: &str,
    kbd_text: Option<&str>,
    danger: bool,
    enabled: bool,
) -> Response {
    let width = ui.available_width().max(200.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if enabled && response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(RADIUS), HOVER);
        }
        let mut color = if danger { RED } else { TEXT };
        if !enabled {
            color = FAINT;
        }
        let g = galley(ui, label, body(12.5), color);
        paint_galley_left(painter, pos2(rect.left() + 10.0, rect.center().y), g, color);
        if let Some(kbd_text) = kbd_text {
            let g = galley(ui, kbd_text, mono(10.5), FAINT);
            let kbd_rect = Rect::from_min_size(
                pos2(
                    rect.right() - 10.0 - g.size().x - 10.0,
                    rect.center().y - 9.0,
                ),
                vec2(g.size().x + 10.0, 18.0),
            );
            paint_kbd_box(painter, kbd_rect);
            painter.galley(kbd_rect.center() - g.size() * 0.5, g, FAINT);
        }
    }
    response
}

pub fn menu_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(4.0),
        rect.center().y,
        Stroke::new(1.0_f32, LINE),
    );
}

/// A thin vertical divider of `height`, for toolbars.
pub fn vdivider(ui: &mut Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(1.0, height), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0_f32, LINE));
}

/// A sparkline of `values` scaled to their own maximum (or `floor`, if larger), with a soft area
/// under the line when `area` is set.
pub fn sparkline(
    painter: &Painter,
    rect: Rect,
    values: &[f32],
    floor: f32,
    color: Color32,
    area: bool,
) {
    if values.len() < 2 {
        return;
    }
    let max = values.iter().copied().fold(floor, f32::max) * 1.15;
    let n = values.len();
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            pos2(
                rect.left() + rect.width() * i as f32 / (n - 1) as f32,
                rect.bottom() - 2.0 - (v / max).clamp(0.0, 1.0) * (rect.height() - 4.0),
            )
        })
        .collect();
    if area {
        let fill = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 28);
        // A strip of quads, since the area under a line is not convex.
        let mut mesh = egui::Mesh::default();
        for pair in points.windows(2) {
            let base = mesh.vertices.len() as u32;
            for p in [
                pair[0],
                pair[1],
                pos2(pair[1].x, rect.bottom()),
                pos2(pair[0].x, rect.bottom()),
            ] {
                mesh.colored_vertex(p, fill);
            }
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        painter.add(Shape::mesh(mesh));
    }
    painter.add(Shape::line(points, Stroke::new(1.5_f32, color)));
}

/// A sparkline normalised between the series' own minimum and maximum, for memory, which moves
/// by small amounts on a large base. Zero means not running and sits on the floor.
pub fn sparkline_range(painter: &Painter, rect: Rect, values: &[f32], color: Color32) {
    if values.len() < 2 {
        return;
    }
    let live: Vec<f32> = values.iter().copied().filter(|v| *v > 0.0).collect();
    let (lo, hi) = if live.is_empty() {
        (0.0, 1.0)
    } else {
        let lo = live.iter().copied().fold(f32::MAX, f32::min);
        let hi = live.iter().copied().fold(f32::MIN, f32::max);
        let pad = ((hi - lo) * 0.15).max(hi * 0.02).max(1.0);
        (lo - pad, hi + pad)
    };
    let n = values.len();
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let y = if *v > 0.0 {
                rect.bottom() - 3.0 - ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * (rect.height() - 6.0)
            } else {
                rect.bottom() - 1.0
            };
            pos2(rect.left() + rect.width() * i as f32 / (n - 1) as f32, y)
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(1.5_f32, color)));
}

/// Draws `text` inside `rect`, left-aligned and vertically centred, cut to fit.
pub fn text_in(ui: &Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    let g = galley_truncated(ui, text, font, color, rect.width());
    paint_galley_left(ui.painter(), pos2(rect.left(), rect.center().y), g, color);
}

/// Places a widget in `rect` without touching the parent's cursor (unlike `Ui::put`, which
/// allocates the rect and so eats the spacing after a widget that already allocated it).
pub fn put_free(ui: &mut Ui, rect: Rect, widget: impl egui::Widget) -> Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
    ));
    child.add(widget)
}
