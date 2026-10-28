//! The Graphite look: colours, type and metrics for the whole app, in one place.
//!
//! The spec is the "Process Manager Redesign" mockup (Graphite artboard): a near-black ground with
//! a cool tint, one periwinkle accent for focus, selection and the primary button, and status
//! colours used only for status. Type is Inter (Regular and Bold) for the interface and Cascadia
//! Mono for logs and commands.

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x0e);
pub const PANEL: Color32 = Color32::from_rgb(0x10, 0x12, 0x15);
pub const PANEL2: Color32 = Color32::from_rgb(0x16, 0x19, 0x1d);
pub const HOVER: Color32 = Color32::from_rgb(0x17, 0x1a, 0x1f);
pub const SELECTED: Color32 = Color32::from_rgb(0x1d, 0x22, 0x2a);
pub const LINE: Color32 = Color32::from_rgb(0x22, 0x26, 0x2d);
pub const ROW_LINE: Color32 = Color32::from_rgb(0x15, 0x17, 0x1b);
pub const BORDER: Color32 = Color32::from_rgb(0x2f, 0x35, 0x3e);
pub const BORDER_HI: Color32 = Color32::from_rgb(0x55, 0x60, 0x70);
pub const BUTTON: Color32 = Color32::from_rgb(0x12, 0x15, 0x19);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(0x18, 0x1c, 0x21);
pub const TEXT: Color32 = Color32::from_rgb(0xec, 0xee, 0xf2);
/// Secondary text: labels, counts, the second half of a sentence.
pub const MUTED: Color32 = Color32::from_rgb(0x9b, 0xa3, 0xae);
/// The quietest text: column headers, section labels, hints.
pub const FAINT: Color32 = Color32::from_rgb(0x86, 0x8f, 0x9b);
pub const ACCENT: Color32 = Color32::from_rgb(0xa3, 0xb8, 0xff);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xbc, 0xcb, 0xff);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x0e);
pub const LOG_BG: Color32 = Color32::from_rgb(0x08, 0x09, 0x0b);
pub const LOG_TEXT: Color32 = Color32::from_rgb(0xd7, 0xdb, 0xe1);
pub const LOG_ERROR_TEXT: Color32 = Color32::from_rgb(0xf0, 0x8a, 0x8a);

pub const GREEN: Color32 = Color32::from_rgb(0x5f, 0xbf, 0x62);
pub const AMBER: Color32 = Color32::from_rgb(0xe3, 0xa6, 0x3a);
pub const RED: Color32 = Color32::from_rgb(0xe2, 0x5f, 0x5f);
pub const GREY: Color32 = Color32::from_rgb(0x6f, 0x74, 0x7b);

/// `rgba(163,184,255,.11)`: selected rows, pressed toggles.
pub fn accent_soft() -> Color32 {
    Color32::from_rgba_unmultiplied(0xa3, 0xb8, 0xff, 28)
}

/// `rgba(163,184,255,.14)` with a half-strength border: the chosen chip or segment.
pub fn chip_on_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(0xa3, 0xb8, 0xff, 36)
}

pub fn chip_on_border() -> Color32 {
    Color32::from_rgba_unmultiplied(0xa3, 0xb8, 0xff, 128)
}

/// The faint red behind a row or log line that needs attention.
pub fn red_soft() -> Color32 {
    Color32::from_rgba_unmultiplied(0xe2, 0x5f, 0x5f, 20)
}

/// Search matches in the log.
pub fn hit_fill() -> Color32 {
    Color32::from_rgba_unmultiplied(0xe3, 0xa6, 0x3a, 82)
}

/// The 16 named terminal colours, tuned to read on `LOG_BG`. These are the program's colours,
/// not status: they show only inside log lines and terminal screens.
const TERMINAL_16: [Color32; 16] = [
    Color32::from_rgb(0x3b, 0x41, 0x4b), // black: lifted so it shows on the log background
    Color32::from_rgb(0xe8, 0x76, 0x76),
    Color32::from_rgb(0x86, 0xc9, 0x7d),
    Color32::from_rgb(0xe0, 0xbd, 0x74),
    Color32::from_rgb(0x7f, 0xa2, 0xf0),
    Color32::from_rgb(0xc3, 0x93, 0xe6),
    Color32::from_rgb(0x6e, 0xc4, 0xd2),
    Color32::from_rgb(0xc9, 0xce, 0xd6),
    Color32::from_rgb(0x67, 0x6f, 0x7c),
    Color32::from_rgb(0xff, 0x93, 0x93),
    Color32::from_rgb(0xa4, 0xe0, 0x9a),
    Color32::from_rgb(0xf2, 0xd3, 0x8e),
    Color32::from_rgb(0xa3, 0xb8, 0xff),
    Color32::from_rgb(0xdb, 0xb2, 0xf5),
    Color32::from_rgb(0x8f, 0xdc, 0xe8),
    Color32::from_rgb(0xf2, 0xf4, 0xf7),
];

/// A terminal colour by xterm index: the 16 named ones, the 6×6×6 cube, then 24 greys.
pub fn terminal_color(index: u8) -> Color32 {
    match index {
        0..=15 => TERMINAL_16[index as usize],
        16..=231 => {
            let i = index - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (index - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

// Corners.
pub const RADIUS: u8 = 6;
pub const RADIUS_CARD: u8 = 8;

// Metrics from the mockup.
pub const GUTTER: f32 = 22.0;
pub const HEADER_PAD_X: f32 = 16.0;
pub const CONTROL_HEIGHT: f32 = 30.0;
pub const SMALL_CONTROL_HEIGHT: f32 = 26.0;
pub const ROW_HEIGHT: f32 = 30.0;
pub const LOG_ROW_HEIGHT: f32 = 20.0;
pub const FOOTER_HEIGHT: f32 = 30.0;
pub const SIDEBAR_WIDTH: f32 = 260.0;
pub const SIDEBAR_MIN_WIDTH: f32 = 220.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 400.0;

/// Window widths where the layout steps down. Below `COMPACT` the sidebar folds into a process
/// switcher in the header; below `TINY` the header keeps only that switcher and a menu.
pub const WIDE: f32 = 1100.0;
pub const MEDIUM: f32 = 900.0;
pub const COMPACT: f32 = 760.0;
pub const TINY: f32 = 520.0;

pub const BOLD: &str = "bold";

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");
const CASCADIA_MONO: &[u8] = include_bytes!("../../assets/fonts/CascadiaMono.ttf");

/// egui's fonts with Inter in front of the proportional family, Inter Bold as `bold` and
/// Cascadia Mono in front of the monospace family; egui's own fonts stay behind them for any
/// symbol Inter lacks.
pub fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("Inter", INTER_REGULAR),
        ("Inter Bold", INTER_BOLD),
        ("Cascadia Mono", CASCADIA_MONO),
    ] {
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
    }
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let family = |first: &str| {
        std::iter::once(first.to_string())
            .chain(fallback.iter().cloned())
            .collect::<Vec<_>>()
    };
    fonts
        .families
        .insert(FontFamily::Proportional, family("Inter"));
    fonts
        .families
        .insert(FontFamily::Name(BOLD.into()), family("Inter Bold"));
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .insert(0, "Cascadia Mono".into());
    fonts
}

pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn mono(size: f32) -> FontId {
    FontId::monospace(size)
}

/// Fonts, visuals and spacing for the whole app.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    // Select the theme before styling it so OS theme events cannot replace our chrome.
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.text_styles = [
        (TextStyle::Small, body(11.5)),
        (TextStyle::Body, body(13.0)),
        (TextStyle::Button, bold(12.5)),
        (TextStyle::Heading, bold(24.0)),
        (TextStyle::Monospace, mono(12.0)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size.y = 20.0;
    style.spacing.indent = 16.0;
    style.spacing.scroll = egui::style::ScrollStyle::thin();
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.scroll.floating_allocated_width = 0.0;
    style.spacing.scroll.bar_inner_margin = 2.0;
    style.interaction.tooltip_delay = 0.35;
    // Don't select text in labels by dragging; rows and logs handle selection themselves.
    style.interaction.selectable_labels = false;

    let mut v = egui::Visuals::dark();
    // Do NOT set override_text_color: it prevents selected text from being visible.
    v.override_text_color = None;
    v.weak_text_color = Some(MUTED);
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.text_edit_bg_color = Some(BG);
    v.faint_bg_color = PANEL;
    v.code_bg_color = PANEL;
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = AMBER;
    v.error_fg_color = RED;
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_corner_radius = egui::CornerRadius::same(RADIUS_CARD + 2);
    v.menu_corner_radius = egui::CornerRadius::same(RADIUS_CARD);
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 14],
        blur: 40,
        spread: 0,
        color: Color32::from_black_alpha(150),
    };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = Color32::from_rgba_unmultiplied(0xa3, 0xb8, 0xff, 70);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.text_cursor.stroke = Stroke::new(1.5_f32, ACCENT);
    let corner = egui::CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    w.noninteractive.corner_radius = corner;
    for (state, fill, border, text) in [
        (&mut w.inactive, BUTTON, BORDER, TEXT),
        (&mut w.hovered, BUTTON_HOVER, BORDER_HI, TEXT),
        (&mut w.active, PANEL2, ACCENT, TEXT),
        (&mut w.open, PANEL2, BORDER_HI, TEXT),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0_f32, border);
        state.fg_stroke = Stroke::new(1.5_f32, text);
        state.corner_radius = corner;
        state.expansion = 0.0;
    }
    style.visuals = v;
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// The layout tier for a window width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Tiny,
    Compact,
    Medium,
    Full,
    Wide,
}

impl Tier {
    pub fn for_width(width: f32) -> Self {
        if width < TINY {
            Self::Tiny
        } else if width < COMPACT {
            Self::Compact
        } else if width < MEDIUM {
            Self::Medium
        } else if width < WIDE {
            Self::Full
        } else {
            Self::Wide
        }
    }

    /// The sidebar shows as a column; below this it folds into the header's process switcher.
    pub fn has_sidebar(self) -> bool {
        self >= Self::Medium
    }
}
