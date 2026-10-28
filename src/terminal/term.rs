//! The emulator: a screen of styled cells driven by the escape sequences a program writes to its
//! terminal. Rows that leave the primary screen (scrolled off the top, or cleared) are queued in
//! order so the session can turn them into log lines.

use std::time::Instant;

use unicode_width::UnicodeWidthChar;
use vte::{Params, Perform};

/// A colour as the program asked for it; the UI maps it to the palette.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TermColor {
    #[default]
    Default,
    /// 0–15 are the named colours, 16–255 the xterm cube and greys.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

pub const BOLD: u8 = 1;
pub const DIM: u8 = 2;
pub const ITALIC: u8 = 4;
pub const UNDERLINE: u8 = 8;
pub const INVERSE: u8 = 16;
pub const STRIKE: u8 = 32;
pub const HIDDEN: u8 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: TermColor,
    pub bg: TermColor,
    pub flags: u8,
}

impl Style {
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    pub fn is_plain(&self) -> bool {
        *self == Style::default()
    }

    /// Whether a space in this style shows anything.
    fn blank_is_visible(&self) -> bool {
        self.bg != TermColor::Default || self.has(INVERSE) || self.has(UNDERLINE)
    }

    /// The style erased cells take: the pen's background (xterm's "background colour erase").
    fn erased(&self) -> Style {
        Style {
            bg: self.bg,
            ..Style::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
    /// 1 for a normal cell, 2 for the first half of a wide character, 0 for its second half.
    pub width: u8,
}

impl Cell {
    fn blank(style: Style) -> Self {
        Self {
            ch: ' ',
            style,
            width: 1,
        }
    }

    fn is_blank(&self) -> bool {
        self.width == 0 || (self.ch == ' ' && !self.style.blank_is_visible())
    }
}

/// One screen row, and what the log knows about it.
#[derive(Clone, Debug)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// Changed since the session last looked.
    pub dirty: bool,
    /// The log line this row is shown as.
    pub line: Option<u64>,
    /// The text last sent to the log for this row.
    pub shown: Option<String>,
    /// Hash of the text last written to disk for this row.
    pub written: Option<u64>,
    pub changed_at: Option<Instant>,
    /// The row was erased after it had content; the log keeps the old content unless the row is
    /// written again soon (a redraw), and then lets the row go.
    pub blank_since: Option<Instant>,
    /// Erased for good: leave it out of the log until something is written to it.
    pub erased: bool,
    /// On screen when a full-screen program took over. Its log line keeps what it said then: a
    /// later change (the repaint after the program, rewrapped by a resize) becomes a new line.
    pub sealed: bool,
}

impl Row {
    fn new(cols: usize, style: Style) -> Self {
        Self {
            cells: vec![Cell::blank(style); cols],
            dirty: true,
            line: None,
            shown: None,
            written: None,
            changed_at: None,
            blank_since: None,
            erased: false,
            sealed: false,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.cells.iter().all(Cell::is_blank)
    }

    /// The row's text with trailing blanks trimmed, and the styled stretches of it.
    pub fn text_and_spans(&self) -> (String, Vec<(usize, usize, Style)>) {
        let end = self
            .cells
            .iter()
            .rposition(|cell| !cell.is_blank())
            .map_or(0, |i| i + 1);
        let mut text = String::new();
        let mut spans: Vec<(usize, usize, Style)> = Vec::new();
        for cell in &self.cells[..end] {
            if cell.width == 0 {
                continue;
            }
            let start = text.len();
            text.push(cell.ch);
            if cell.style.is_plain() {
                continue;
            }
            match spans.last_mut() {
                Some(last) if last.1 == start && last.2 == cell.style => last.1 = text.len(),
                _ => spans.push((start, text.len(), cell.style)),
            }
        }
        (text, spans)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseMode {
    #[default]
    Off,
    /// 1000: presses and releases.
    Press,
    /// 1002: and drags.
    Drag,
    /// 1003: and every move.
    Motion,
}

/// What the program asked of its keyboard and mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputModes {
    pub app_cursor: bool,
    pub bracketed_paste: bool,
    pub mouse: MouseMode,
    pub mouse_sgr: bool,
    pub focus_events: bool,
}

impl Default for InputModes {
    fn default() -> Self {
        Self {
            app_cursor: false,
            bracketed_paste: false,
            mouse: MouseMode::Off,
            mouse_sgr: false,
            focus_events: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    row: usize,
    col: usize,
    /// The last column was just written; the next character wraps first.
    pending_wrap: bool,
    style: Style,
    origin: bool,
    graphics: [bool; 2],
    charset: usize,
}

pub struct Term {
    cols: usize,
    rows: usize,
    primary: Vec<Row>,
    alt: Vec<Row>,
    alt_active: bool,
    cursor: Cursor,
    saved_primary: Option<Cursor>,
    saved_alt: Option<Cursor>,
    top: usize,
    bottom: usize,
    autowrap: bool,
    insert: bool,
    pub cursor_visible: bool,
    pub modes: InputModes,
    tabs: Vec<bool>,
    last_char: Option<char>,
    pub title: String,
    /// Primary-screen rows that left the screen, oldest first, until the session takes them.
    departed: Vec<Row>,
    /// Answers to the program's queries (cursor position, device attributes).
    pub replies: Vec<u8>,
    /// Bumped on every change, so the UI can skip rebuilding an unchanged screen.
    pub revision: u64,
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(2);
        Self {
            cols,
            rows,
            primary: (0..rows)
                .map(|_| Row::new(cols, Style::default()))
                .collect(),
            alt: (0..rows)
                .map(|_| Row::new(cols, Style::default()))
                .collect(),
            alt_active: false,
            cursor: Cursor::default(),
            saved_primary: None,
            saved_alt: None,
            top: 0,
            bottom: rows - 1,
            autowrap: true,
            insert: false,
            cursor_visible: true,
            modes: InputModes::default(),
            tabs: tab_stops(cols),
            last_char: None,
            title: String::new(),
            departed: Vec::new(),
            replies: Vec::new(),
            revision: 0,
        }
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn alt_active(&self) -> bool {
        self.alt_active
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor.row, self.cursor.col)
    }

    /// The rows on screen now.
    pub fn screen(&self) -> &[Row] {
        if self.alt_active {
            &self.alt
        } else {
            &self.primary
        }
    }

    pub fn primary_mut(&mut self) -> &mut Vec<Row> {
        &mut self.primary
    }

    pub fn take_departed(&mut self) -> Vec<Row> {
        std::mem::take(&mut self.departed)
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        if cols == self.cols && rows == self.rows {
            return;
        }
        for (screen, is_primary) in [(&mut self.primary, true), (&mut self.alt, false)] {
            for row in screen.iter_mut() {
                if row.cells.len() != cols {
                    if cols < row.cells.len() {
                        // A wide character cut in half loses both halves.
                        if row.cells[cols - 1].width == 2 {
                            row.cells[cols - 1] = Cell::blank(row.cells[cols - 1].style);
                        }
                    }
                    row.cells.resize(cols, Cell::blank(Style::default()));
                    row.dirty = true;
                }
            }
            let is_active = is_primary != self.alt_active;
            if rows < screen.len() {
                // Keep the cursor's row on screen: rows above it go first, then rows below.
                let cursor_row = if is_active { self.cursor.row } else { 0 };
                let from_top = (cursor_row + 1).saturating_sub(rows);
                for row in screen.drain(..from_top) {
                    if is_primary {
                        self.departed.push(row);
                    }
                }
                screen.truncate(rows);
                if is_active {
                    self.cursor.row -= from_top;
                }
            }
            while screen.len() < rows {
                screen.push(Row::new(cols, Style::default()));
            }
        }
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.cursor.row = self.cursor.row.min(rows - 1);
        self.cursor.col = self.cursor.col.min(cols - 1);
        self.cursor.pending_wrap = false;
        self.tabs = tab_stops(cols);
        self.revision += 1;
    }

    fn screen_mut(&mut self) -> &mut Vec<Row> {
        if self.alt_active {
            &mut self.alt
        } else {
            &mut self.primary
        }
    }

    fn row_mut(&mut self, row: usize) -> &mut Row {
        let row = row.min(self.rows - 1);
        let r = &mut self.screen_mut()[row];
        r.dirty = true;
        r
    }

    fn blank_row(&self) -> Row {
        Row::new(self.cols, self.cursor.style.erased())
    }

    // Cursor movement.

    fn set_col(&mut self, col: usize) {
        self.cursor.col = col.min(self.cols - 1);
        self.cursor.pending_wrap = false;
    }

    fn set_row(&mut self, row: usize) {
        let (lo, hi) = if self.cursor.origin {
            (self.top, self.bottom)
        } else {
            (0, self.rows - 1)
        };
        self.cursor.row = (row + if self.cursor.origin { self.top } else { 0 }).clamp(lo, hi);
        self.cursor.pending_wrap = false;
    }

    fn move_up(&mut self, n: usize) {
        let limit = if self.cursor.row >= self.top {
            self.top
        } else {
            0
        };
        self.cursor.row = self.cursor.row.saturating_sub(n).max(limit);
        self.cursor.pending_wrap = false;
    }

    fn move_down(&mut self, n: usize) {
        let limit = if self.cursor.row <= self.bottom {
            self.bottom
        } else {
            self.rows - 1
        };
        self.cursor.row = (self.cursor.row + n).min(limit);
        self.cursor.pending_wrap = false;
    }

    fn linefeed(&mut self) {
        if self.cursor.row == self.bottom {
            self.scroll_up(1);
        } else if self.cursor.row < self.rows - 1 {
            self.cursor.row += 1;
        }
        self.cursor.pending_wrap = false;
    }

    fn reverse_index(&mut self) {
        if self.cursor.row == self.top {
            self.scroll_down(1);
        } else {
            self.cursor.row = self.cursor.row.saturating_sub(1);
        }
        self.cursor.pending_wrap = false;
    }

    fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        let leaves_screen = top == 0 && !self.alt_active;
        for _ in 0..n {
            let blank = self.blank_row();
            let screen = self.screen_mut();
            let row = screen.remove(top);
            screen.insert(bottom, blank);
            if leaves_screen {
                self.departed.push(row);
            }
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        for _ in 0..n {
            let blank = self.blank_row();
            let screen = self.screen_mut();
            screen.remove(bottom);
            screen.insert(top, blank);
        }
    }

    fn insert_lines(&mut self, n: usize) {
        if self.cursor.row < self.top || self.cursor.row > self.bottom {
            return;
        }
        let (row, bottom) = (self.cursor.row, self.bottom);
        for _ in 0..n.min(bottom - row + 1) {
            let blank = self.blank_row();
            let screen = self.screen_mut();
            screen.remove(bottom);
            screen.insert(row, blank);
        }
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    fn delete_lines(&mut self, n: usize) {
        if self.cursor.row < self.top || self.cursor.row > self.bottom {
            return;
        }
        let (row, bottom) = (self.cursor.row, self.bottom);
        for _ in 0..n.min(bottom - row + 1) {
            let blank = self.blank_row();
            let screen = self.screen_mut();
            screen.remove(row);
            screen.insert(bottom, blank);
        }
        self.cursor.col = 0;
        self.cursor.pending_wrap = false;
    }

    // Writing and erasing.

    fn put(&mut self, c: char) {
        let c = if self.cursor.graphics[self.cursor.charset] {
            dec_graphics(c)
        } else {
            c
        };
        let width = c.width().unwrap_or(0);
        if width == 0 {
            // Combining marks and zero-width joiners: the cell keeps its base character.
            return;
        }
        let width = width.min(2);
        if self.cursor.pending_wrap {
            if self.autowrap {
                self.cursor.col = 0;
                self.linefeed();
            }
            self.cursor.pending_wrap = false;
        }
        if width == 2 && self.cursor.col + 1 >= self.cols {
            if !self.autowrap {
                return;
            }
            let (row, col) = (self.cursor.row, self.cursor.col);
            let style = self.cursor.style.erased();
            self.row_mut(row).cells[col] = Cell::blank(style);
            self.cursor.col = 0;
            self.linefeed();
        }
        let (row, col, cols, style, insert) = (
            self.cursor.row,
            self.cursor.col,
            self.cols,
            self.cursor.style,
            self.insert,
        );
        let r = self.row_mut(row);
        if insert {
            for _ in 0..width {
                r.cells.insert(col, Cell::blank(style.erased()));
                r.cells.pop();
            }
        }
        clear_wide(&mut r.cells, col);
        if width == 2 {
            clear_wide(&mut r.cells, col + 1);
        }
        r.cells[col] = Cell {
            ch: c,
            style,
            width: width as u8,
        };
        if width == 2 {
            r.cells[col + 1] = Cell {
                ch: ' ',
                style,
                width: 0,
            };
        }
        self.last_char = Some(c);
        if col + width >= cols {
            self.cursor.col = cols - 1;
            self.cursor.pending_wrap = self.autowrap;
        } else {
            self.cursor.col = col + width;
        }
    }

    fn erase_cells(&mut self, row: usize, from: usize, to: usize) {
        let style = self.cursor.style.erased();
        let r = self.row_mut(row);
        let to = to.min(r.cells.len());
        if from >= to {
            return;
        }
        clear_wide(&mut r.cells, from);
        clear_wide(&mut r.cells, to - 1);
        for cell in &mut r.cells[from..to] {
            *cell = Cell::blank(style);
        }
    }

    fn erase_in_line(&mut self, mode: usize) {
        let (row, col, cols) = (self.cursor.row, self.cursor.col, self.cols);
        match mode {
            0 => self.erase_cells(row, col, cols),
            1 => self.erase_cells(row, 0, col + 1),
            _ => self.erase_cells(row, 0, cols),
        }
        self.cursor.pending_wrap = false;
    }

    fn erase_in_display(&mut self, mode: usize) {
        let (row, cols, rows) = (self.cursor.row, self.cols, self.rows);
        match mode {
            0 if row == 0 && self.cursor.col == 0 => self.clear_screen(),
            0 => {
                self.erase_in_line(0);
                for r in row + 1..rows {
                    self.erase_cells(r, 0, cols);
                }
            }
            1 => {
                for r in 0..row {
                    self.erase_cells(r, 0, cols);
                }
                self.erase_in_line(1);
            }
            2 | 3 => self.clear_screen(),
            _ => {}
        }
    }

    /// Clears the whole screen. On the primary screen what was there leaves for the log first,
    /// as a terminal keeps it in its scrollback.
    fn clear_screen(&mut self) {
        if !self.alt_active {
            let last = self
                .primary
                .iter()
                .rposition(|row| !row.is_blank() || row.line.is_some());
            if let Some(last) = last {
                let rows: Vec<Row> = self.primary.drain(..=last).collect();
                self.departed.extend(rows);
                while self.primary.len() < self.rows {
                    self.primary.insert(0, self.blank_row());
                }
            }
        }
        let blank = self.blank_row();
        for row in self.screen_mut().iter_mut() {
            *row = blank.clone();
        }
    }

    fn set_alt(&mut self, on: bool, save_cursor: bool) {
        if on == self.alt_active {
            return;
        }
        if on {
            if save_cursor {
                self.saved_primary = Some(self.cursor);
            }
            self.alt_active = true;
            let blank = self.blank_row();
            for row in self.alt.iter_mut() {
                *row = blank.clone();
            }
            for row in self.primary.iter_mut() {
                row.sealed = row.line.is_some();
            }
        } else {
            self.alt_active = false;
            if save_cursor {
                if let Some(saved) = self.saved_primary.take() {
                    self.cursor = saved;
                }
            }
            for row in self.primary.iter_mut() {
                row.dirty = true;
            }
        }
        self.cursor.row = self.cursor.row.min(self.rows - 1);
        self.cursor.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        if self.alt_active {
            self.saved_alt = Some(self.cursor);
        } else {
            self.saved_primary = Some(self.cursor);
        }
    }

    fn restore_cursor(&mut self) {
        let saved = if self.alt_active {
            self.saved_alt
        } else {
            self.saved_primary
        };
        if let Some(saved) = saved {
            self.cursor = saved;
            self.cursor.row = self.cursor.row.min(self.rows - 1);
            self.cursor.col = self.cursor.col.min(self.cols - 1);
        }
    }

    fn reset(&mut self) {
        self.set_alt(false, false);
        self.clear_screen();
        let departed = std::mem::take(&mut self.departed);
        let title = std::mem::take(&mut self.title);
        let revision = self.revision;
        *self = Term::new(self.cols, self.rows);
        self.departed = departed;
        self.title = title;
        self.revision = revision;
    }

    fn set_mode(&mut self, private: bool, mode: u16, on: bool) {
        if !private {
            if mode == 4 {
                self.insert = on;
            }
            return;
        }
        match mode {
            1 => self.modes.app_cursor = on,
            6 => {
                self.cursor.origin = on;
                self.set_row(0);
                self.set_col(0);
            }
            7 => self.autowrap = on,
            25 => self.cursor_visible = on,
            47 | 1047 => self.set_alt(on, false),
            1048 => {
                if on {
                    self.save_cursor()
                } else {
                    self.restore_cursor()
                }
            }
            1049 => self.set_alt(on, true),
            1000 => self.modes.mouse = if on { MouseMode::Press } else { MouseMode::Off },
            1002 => self.modes.mouse = if on { MouseMode::Drag } else { MouseMode::Off },
            1003 => {
                self.modes.mouse = if on {
                    MouseMode::Motion
                } else {
                    MouseMode::Off
                }
            }
            1006 => self.modes.mouse_sgr = on,
            1004 => self.modes.focus_events = on,
            2004 => self.modes.bracketed_paste = on,
            _ => {}
        }
    }

    fn sgr(&mut self, params: &Params) {
        let mut style = self.cursor.style;
        let mut iter = params.iter();
        if params.is_empty() {
            style = Style::default();
        }
        while let Some(param) = iter.next() {
            let code = param.first().copied().unwrap_or(0);
            match code {
                0 => style = Style::default(),
                1 => style.flags |= BOLD,
                2 => style.flags |= DIM,
                3 => style.flags |= ITALIC,
                4 => {
                    if param.get(1) == Some(&0) {
                        style.flags &= !UNDERLINE
                    } else {
                        style.flags |= UNDERLINE
                    }
                }
                7 => style.flags |= INVERSE,
                8 => style.flags |= HIDDEN,
                9 => style.flags |= STRIKE,
                21 => style.flags |= UNDERLINE,
                22 => style.flags &= !(BOLD | DIM),
                23 => style.flags &= !ITALIC,
                24 => style.flags &= !UNDERLINE,
                27 => style.flags &= !INVERSE,
                28 => style.flags &= !HIDDEN,
                29 => style.flags &= !STRIKE,
                30..=37 => style.fg = TermColor::Indexed((code - 30) as u8),
                39 => style.fg = TermColor::Default,
                40..=47 => style.bg = TermColor::Indexed((code - 40) as u8),
                49 => style.bg = TermColor::Default,
                90..=97 => style.fg = TermColor::Indexed((code - 90 + 8) as u8),
                100..=107 => style.bg = TermColor::Indexed((code - 100 + 8) as u8),
                38 | 48 | 58 => {
                    let color = if param.len() > 1 {
                        colon_color(&param[1..])
                    } else {
                        semicolon_color(&mut iter)
                    };
                    match (code, color) {
                        (38, Some(color)) => style.fg = color,
                        (48, Some(color)) => style.bg = color,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        self.cursor.style = style;
    }
}

impl Perform for Term {
    fn print(&mut self, c: char) {
        self.put(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                self.cursor.col = self.cursor.col.saturating_sub(1);
                self.cursor.pending_wrap = false;
            }
            0x09 => {
                let next = (self.cursor.col + 1..self.cols)
                    .find(|&c| self.tabs[c])
                    .unwrap_or(self.cols - 1);
                self.set_col(next);
            }
            0x0a..=0x0c => self.linefeed(),
            0x0d => self.set_col(0),
            0x0e => self.cursor.charset = 1,
            0x0f => self.cursor.charset = 0,
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        let args: Vec<u16> = params.iter().map(|p| p[0]).collect();
        // A count: missing or zero means one.
        let n = |i: usize| args.get(i).copied().filter(|&v| v > 0).unwrap_or(1) as usize;
        let raw = |i: usize| args.get(i).copied().unwrap_or(0) as usize;
        let private = intermediates.first() == Some(&b'?');
        match (intermediates.first().copied(), action) {
            (None, '@') => {
                let (row, col, style) = (self.cursor.row, self.cursor.col, self.cursor.style);
                let r = self.row_mut(row);
                for _ in 0..n(0).min(r.cells.len() - col) {
                    r.cells.insert(col, Cell::blank(style.erased()));
                    r.cells.pop();
                }
            }
            (None, 'A') => self.move_up(n(0)),
            (None, 'B') | (None, 'e') => self.move_down(n(0)),
            (None, 'C') | (None, 'a') => self.set_col(self.cursor.col + n(0)),
            (None, 'D') => self.set_col(self.cursor.col.saturating_sub(n(0))),
            (None, 'E') => {
                self.move_down(n(0));
                self.set_col(0);
            }
            (None, 'F') => {
                self.move_up(n(0));
                self.set_col(0);
            }
            (None, 'G') | (None, '`') => self.set_col(n(0) - 1),
            (None, 'H') | (None, 'f') => {
                self.set_row(n(0) - 1);
                self.set_col(n(1) - 1);
            }
            (None, 'I') => {
                for _ in 0..n(0) {
                    self.execute(0x09);
                }
            }
            (None, 'J') | (Some(b'?'), 'J') => self.erase_in_display(raw(0)),
            (None, 'K') | (Some(b'?'), 'K') => self.erase_in_line(raw(0)),
            (None, 'L') => self.insert_lines(n(0)),
            (None, 'M') => self.delete_lines(n(0)),
            (None, 'P') => {
                let (row, col, style) = (self.cursor.row, self.cursor.col, self.cursor.style);
                let r = self.row_mut(row);
                for _ in 0..n(0).min(r.cells.len() - col) {
                    r.cells.remove(col);
                    r.cells.push(Cell::blank(style.erased()));
                }
            }
            (None, 'S') => self.scroll_up(n(0)),
            (None, 'T') => self.scroll_down(n(0)),
            (None, 'X') => {
                let (row, col) = (self.cursor.row, self.cursor.col);
                self.erase_cells(row, col, col + n(0));
            }
            (None, 'Z') => {
                for _ in 0..n(0) {
                    let prev = (0..self.cursor.col)
                        .rev()
                        .find(|&c| self.tabs[c])
                        .unwrap_or(0);
                    self.set_col(prev);
                }
            }
            (None, 'b') => {
                if let Some(c) = self.last_char {
                    for _ in 0..n(0).min(self.cols * self.rows) {
                        self.put(c);
                    }
                }
            }
            (None, 'c') if raw(0) == 0 => self.replies.extend_from_slice(b"\x1b[?62;22c"),
            (Some(b'>'), 'c') => self.replies.extend_from_slice(b"\x1b[>0;10;1c"),
            (None, 'd') => self.set_row(n(0) - 1),
            (None, 'g') => match raw(0) {
                0 => self.tabs[self.cursor.col] = false,
                3 => self.tabs.iter_mut().for_each(|t| *t = false),
                _ => {}
            },
            (None, 'h') | (Some(b'?'), 'h') => {
                for &mode in &args {
                    self.set_mode(private, mode, true);
                }
            }
            (None, 'l') | (Some(b'?'), 'l') => {
                for &mode in &args {
                    self.set_mode(private, mode, false);
                }
            }
            (None, 'm') => self.sgr(params),
            (None, 'n') | (Some(b'?'), 'n') => match raw(0) {
                5 => self.replies.extend_from_slice(b"\x1b[0n"),
                6 => {
                    let row = if self.cursor.origin {
                        self.cursor.row - self.top
                    } else {
                        self.cursor.row
                    };
                    let reply = format!(
                        "\x1b[{}{};{}R",
                        if private { "?" } else { "" },
                        row + 1,
                        self.cursor.col + 1
                    );
                    self.replies.extend_from_slice(reply.as_bytes());
                }
                _ => {}
            },
            (None, 'r') => {
                let top = n(0) - 1;
                let bottom = args
                    .get(1)
                    .copied()
                    .filter(|&v| v > 0)
                    .map_or(self.rows, |v| v as usize)
                    .min(self.rows)
                    - 1;
                if top < bottom {
                    self.top = top;
                    self.bottom = bottom;
                    self.set_row(0);
                    self.set_col(0);
                }
            }
            (None, 's') => self.save_cursor(),
            (None, 'u') => self.restore_cursor(),
            (Some(b'!'), 'p') => {
                self.cursor.style = Style::default();
                self.cursor.origin = false;
                self.autowrap = true;
                self.insert = false;
                self.cursor_visible = true;
                self.modes = InputModes::default();
                self.top = 0;
                self.bottom = self.rows - 1;
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (intermediates.first().copied(), byte) {
            (None, b'7') => self.save_cursor(),
            (None, b'8') => self.restore_cursor(),
            (None, b'D') => self.linefeed(),
            (None, b'E') => {
                self.set_col(0);
                self.linefeed();
            }
            (None, b'H') => self.tabs[self.cursor.col] = true,
            (None, b'M') => self.reverse_index(),
            (None, b'c') => self.reset(),
            (Some(b'('), b'0') => self.cursor.graphics[0] = true,
            (Some(b'('), _) => self.cursor.graphics[0] = false,
            (Some(b')'), b'0') => self.cursor.graphics[1] = true,
            (Some(b')'), _) => self.cursor.graphics[1] = false,
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if let [kind, title, ..] = params {
            if matches!(*kind, b"0" | b"2") {
                self.title = String::from_utf8_lossy(title).into_owned();
            }
        }
    }
}

fn tab_stops(cols: usize) -> Vec<bool> {
    (0..cols).map(|c| c > 0 && c % 8 == 0).collect()
}

/// Writing over half of a wide character blanks the other half.
fn clear_wide(cells: &mut [Cell], col: usize) {
    let Some(cell) = cells.get(col).copied() else {
        return;
    };
    if cell.width == 0 && col > 0 {
        cells[col - 1] = Cell::blank(cells[col - 1].style);
    } else if cell.width == 2 && col + 1 < cells.len() {
        cells[col + 1] = Cell::blank(cells[col + 1].style);
    }
}

fn semicolon_color<'a>(iter: &mut impl Iterator<Item = &'a [u16]>) -> Option<TermColor> {
    match iter.next()?.first()? {
        5 => Some(TermColor::Indexed(*iter.next()?.first()? as u8)),
        2 => {
            let r = *iter.next()?.first()? as u8;
            let g = *iter.next()?.first()? as u8;
            let b = *iter.next()?.first()? as u8;
            Some(TermColor::Rgb(r, g, b))
        }
        _ => None,
    }
}

fn colon_color(sub: &[u16]) -> Option<TermColor> {
    match sub {
        [5, index, ..] => Some(TermColor::Indexed(*index as u8)),
        // 38:2:<colour space>:r:g:b, or 38:2:r:g:b without the colour space.
        [2, _, r, g, b, ..] => Some(TermColor::Rgb(*r as u8, *g as u8, *b as u8)),
        [2, r, g, b] => Some(TermColor::Rgb(*r as u8, *g as u8, *b as u8)),
        _ => None,
    }
}

/// The DEC special graphics set, which line-drawing programs switch to with `ESC ( 0`.
fn dec_graphics(c: char) -> char {
    match c {
        '`' => '◆',
        'a' => '▒',
        'f' => '°',
        'g' => '±',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(term: &mut Term, bytes: &str) {
        let mut parser = vte::Parser::new();
        parser.advance(term, bytes.as_bytes());
    }

    fn text(term: &Term, row: usize) -> String {
        term.screen()[row].text_and_spans().0
    }

    #[test]
    fn carriage_return_overwrites_the_line() {
        let mut term = Term::new(20, 4);
        feed(&mut term, "10%\r20%\r30%");
        assert_eq!(text(&term, 0), "30%");
    }

    #[test]
    fn rows_scrolled_off_the_top_are_kept_in_order() {
        let mut term = Term::new(20, 3);
        feed(&mut term, "a\r\nb\r\nc\r\nd\r\ne");
        let gone: Vec<String> = term
            .take_departed()
            .iter()
            .map(|r| r.text_and_spans().0)
            .collect();
        assert_eq!(gone, ["a", "b"]);
        assert_eq!(text(&term, 2), "e");
    }

    #[test]
    fn colours_become_spans() {
        let mut term = Term::new(30, 2);
        feed(
            &mut term,
            "\x1b[31;1merror\x1b[m ok \x1b[38;2;1;2;3mrgb\x1b[38;5;200mx",
        );
        let (text, spans) = term.screen()[0].text_and_spans();
        assert_eq!(text, "error ok rgbx");
        assert_eq!(spans[0].0..spans[0].1, 0..5);
        assert_eq!(spans[0].2.fg, TermColor::Indexed(1));
        assert!(spans[0].2.has(BOLD));
        assert_eq!(spans[1].2.fg, TermColor::Rgb(1, 2, 3));
        assert_eq!(spans[2].2.fg, TermColor::Indexed(200));
    }

    #[test]
    fn redraws_by_absolute_position_replace_rows() {
        let mut term = Term::new(20, 4);
        feed(&mut term, "title\r\nWorking 0%");
        feed(&mut term, "\x1b[2;1HWorking 50%\x1b[K");
        assert_eq!(text(&term, 1), "Working 50%");
        assert!(term.take_departed().is_empty());
    }

    #[test]
    fn the_alternate_screen_leaves_the_primary_alone() {
        let mut term = Term::new(20, 4);
        feed(&mut term, "before");
        feed(&mut term, "\x1b[?1049h\x1b[Hfull screen\x1b[2J");
        assert!(term.alt_active());
        assert!(term.take_departed().is_empty());
        feed(&mut term, "\x1b[?1049l");
        assert!(!term.alt_active());
        assert_eq!(text(&term, 0), "before");
        assert_eq!(term.cursor(), (0, 6));
    }

    #[test]
    fn clearing_the_primary_screen_sends_its_rows_to_the_log() {
        let mut term = Term::new(20, 4);
        feed(&mut term, "one\r\ntwo\r\n\x1b[2J\x1b[Hthree");
        let gone: Vec<String> = term
            .take_departed()
            .iter()
            .map(|r| r.text_and_spans().0)
            .collect();
        assert_eq!(gone, ["one", "two"]);
        assert_eq!(text(&term, 0), "three");
    }

    #[test]
    fn wide_characters_take_two_cells_and_wrap_whole() {
        let mut term = Term::new(5, 3);
        feed(&mut term, "abcd界");
        assert_eq!(text(&term, 0), "abcd");
        assert_eq!(text(&term, 1), "界");
        assert_eq!(term.cursor(), (1, 2));
    }

    #[test]
    fn line_drawing_charset_maps_to_box_characters() {
        let mut term = Term::new(10, 2);
        feed(&mut term, "\x1b(0lqk\x1b(B x");
        assert_eq!(text(&term, 0), "┌─┐ x");
    }

    #[test]
    fn scroll_regions_keep_rows_outside_them() {
        let mut term = Term::new(10, 4);
        feed(&mut term, "head\x1b[2;4r\x1b[2;1Ha\r\nb\r\nc\r\nd");
        assert_eq!(text(&term, 0), "head");
        assert_eq!(text(&term, 1), "b");
        assert_eq!(text(&term, 3), "d");
        assert!(term.take_departed().is_empty());
    }

    #[test]
    fn rich_table_cells_keep_their_colour() {
        let mut term = Term::new(40, 4);
        feed(
            &mut term,
            "\u{2502} cme   \u{2502}\x1b[32m\x1b[1Cok     \x1b[m\u{2502}\r\n",
        );
        let (text, spans) = term.screen()[0].text_and_spans();
        assert_eq!(text, "\u{2502} cme   \u{2502} ok     \u{2502}");
        let ok = text.find("ok").unwrap();
        assert!(
            spans
                .iter()
                .any(|s| s.0 <= ok && ok < s.1 && s.2.fg == TermColor::Indexed(2)),
            "{spans:?}"
        );
    }

    #[test]
    fn answers_cursor_position_queries() {
        let mut term = Term::new(10, 4);
        feed(&mut term, "\x1b[3;5H\x1b[6n");
        assert_eq!(term.replies, b"\x1b[3;5R");
    }

    #[test]
    fn erase_uses_the_pen_background() {
        let mut term = Term::new(6, 2);
        feed(&mut term, "\x1b[44mab\x1b[K");
        let row = &term.screen()[0];
        assert_eq!(row.cells[5].style.bg, TermColor::Indexed(4));
        assert_eq!(row.text_and_spans().0, "ab    ");
    }

    #[test]
    fn shrinking_keeps_the_cursor_row_and_sends_the_top_to_the_log() {
        let mut term = Term::new(10, 5);
        feed(&mut term, "1\r\n2\r\n3\r\n4\r\n5");
        term.resize(10, 3);
        let gone: Vec<String> = term
            .take_departed()
            .iter()
            .map(|r| r.text_and_spans().0)
            .collect();
        assert_eq!(gone, ["1", "2"]);
        assert_eq!(text(&term, 2), "5");
        assert_eq!(term.cursor().0, 2);
    }
}
