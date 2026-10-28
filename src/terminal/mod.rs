//! Terminals for processes that run in one: the pseudo-terminal's handles, the emulator, and the
//! rules that turn its screen into log lines.
//!
//! A row of the primary screen is a log line. It joins the log as soon as it has text, and while
//! it is on screen the program may rewrite it (a progress bar, a live table): the line changes in
//! place. A row is final when it leaves the screen (scrolled off, or the screen cleared) or the
//! process ends; it is written to disk and checked for errors once it has been still for a
//! moment, so a progress bar is not written a hundred times. The alternate screen, where
//! full-screen programs draw, never reaches the log.

pub mod term;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

pub use term::{InputModes, MouseMode, Style, TermColor};
use term::{Row, Term};

/// How long a row must stay unchanged before it is written to disk and checked for errors.
const SETTLE_AFTER: Duration = Duration::from_millis(1000);
/// How long an erased row keeps its old text in the log, in case the program is redrawing it.
const ERASE_AFTER: Duration = Duration::from_millis(400);

/// A styled stretch of a log line, as byte offsets into its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogSpan {
    pub start: u32,
    pub end: u32,
    pub style: Style,
}

pub type LogSpans = Option<Arc<[LogSpan]>>;

/// Where a terminal's lines go: the process's in-memory log, its disk log, its error signal.
pub trait LogSink {
    /// Adds a line; returns its number.
    fn push(&mut self, text: String, spans: LogSpans) -> u64;
    /// Rewrites line `number`, if it is still in the log.
    fn amend(&mut self, number: u64, text: String, spans: LogSpans);
    /// The line has settled: write it to disk and check it for errors.
    fn settle(&mut self, text: &str);
}

/// What the UI needs to draw the screen and talk to the program.
#[derive(Clone, Debug, Default)]
pub struct ScreenSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<Vec<ScreenRun>>,
    pub cursor: Option<(usize, usize)>,
    pub alt: bool,
    pub modes: InputModes,
    /// The process is still attached: keys and resizes reach it.
    pub live: bool,
}

/// Cells of one style in a screen row.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenRun {
    pub col: usize,
    /// Columns the run covers (a wide character covers two).
    pub cells: usize,
    pub text: String,
    pub style: Style,
}

pub struct TerminalSession {
    term: Term,
    parser: vte::Parser,
    /// The size the terminal started at, which it returns to when a full-screen program it was
    /// fitted for ends (`restore_size`).
    start_size: (usize, usize),
    restore_size: bool,
    input: Option<mpsc::Sender<Vec<u8>>>,
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    reader_done: Option<mpsc::Receiver<()>>,
}

impl TerminalSession {
    /// `restore_size`: go back to the starting size when the program leaves the alternate screen
    /// (Auto mode, whose log is laid out at that size).
    pub fn new(
        cols: usize,
        rows: usize,
        restore_size: bool,
        master: Box<dyn portable_pty::MasterPty + Send>,
        input: mpsc::Sender<Vec<u8>>,
        reader_done: mpsc::Receiver<()>,
    ) -> Self {
        Self {
            term: Term::new(cols, rows),
            parser: vte::Parser::new(),
            start_size: (cols, rows),
            restore_size,
            input: Some(input),
            master: Some(master),
            reader_done: Some(reader_done),
        }
    }

    /// Runs the program's output through the emulator.
    pub fn feed(&mut self, bytes: &[u8]) {
        let was_alt = self.term.alt_active();
        self.parser.advance(&mut self.term, bytes);
        self.term.revision += 1;
        if was_alt && !self.term.alt_active() && self.restore_size {
            let (cols, rows) = self.start_size;
            self.resize(cols, rows);
        }
        if !self.term.replies.is_empty() {
            let replies = std::mem::take(&mut self.term.replies);
            self.send(replies);
        }
    }

    pub fn alt_active(&self) -> bool {
        self.term.alt_active()
    }

    pub fn is_live(&self) -> bool {
        self.master.is_some()
    }

    pub fn size(&self) -> (usize, usize) {
        self.term.size()
    }

    /// Sends keys, pastes or mouse reports to the program.
    pub fn send(&self, bytes: Vec<u8>) {
        if let Some(input) = &self.input {
            let _ = input.send(bytes);
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.clamp(20, 500);
        let rows = rows.clamp(5, 300);
        if (cols, rows) == self.term.size() {
            return;
        }
        if let Some(master) = &self.master {
            let _ = master.resize(portable_pty::PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.term.resize(cols, rows);
    }

    /// Detaches from the program. The caller drops what comes back outside any lock: closing a
    /// Windows pseudo-console waits for its last output, which the reader thread must be free
    /// to read.
    pub fn detach(&mut self) -> Detached {
        self.input = None;
        Detached {
            master: self.master.take(),
            reader_done: self.reader_done.take(),
        }
    }

    /// Brings the log up to date with the screen. With `finish`, every row is final.
    pub fn sync(&mut self, sink: &mut dyn LogSink, now: Instant, finish: bool) {
        sync_term(&mut self.term, sink, now, finish);
    }

    pub fn snapshot(&self, live: bool) -> ScreenSnapshot {
        let (cols, rows) = self.term.size();
        let lines = self.term.screen().iter().map(screen_runs).collect();
        let (row, col) = self.term.cursor();
        ScreenSnapshot {
            cols,
            rows,
            lines,
            cursor: self.term.cursor_visible.then_some((row, col)),
            alt: self.term.alt_active(),
            modes: self.term.modes,
            live,
        }
    }

    pub fn revision(&self) -> u64 {
        self.term.revision
    }

    /// The screen as plain text, for copying.
    pub fn screen_text(&self) -> String {
        let rows: Vec<String> = self
            .term
            .screen()
            .iter()
            .map(|row| row.text_and_spans().0.trim_end().to_string())
            .collect();
        let end = rows
            .iter()
            .rposition(|r| !r.is_empty())
            .map_or(0, |i| i + 1);
        rows[..end].join("\n")
    }
}

/// The handles a session gives up when its process ends.
pub struct Detached {
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    reader_done: Option<mpsc::Receiver<()>>,
}

impl Detached {
    /// Closes the terminal and waits (briefly) for the reader to take the last output.
    pub fn close(self, wait: Duration) {
        drop(self.master);
        if let Some(done) = self.reader_done {
            let _ = done.recv_timeout(wait);
        }
    }
}

/// A thread that writes input to the program, so a program that stops reading never blocks the
/// window.
pub fn spawn_input_writer(mut writer: Box<dyn Write + Send>) -> mpsc::Sender<Vec<u8>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(bytes) = rx.recv() {
            if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                break;
            }
        }
    });
    tx
}

fn sync_term(term: &mut Term, sink: &mut dyn LogSink, now: Instant, finish: bool) {
    for mut row in term.take_departed() {
        sync_row(&mut row, sink, now, true);
        settle_row(&mut row, sink, now, true);
    }
    // The primary screen does not change while a full-screen program has the alternate one.
    if term.alt_active() && !finish {
        return;
    }
    let rows = term.primary_mut();
    let last = rows.iter().rposition(|row| !row.is_blank());
    for (index, row) in rows.iter_mut().enumerate() {
        let in_text = last.is_some_and(|last| index <= last);
        sync_row(row, sink, now, in_text);
        settle_row(row, sink, now, finish);
    }
}

fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Puts a row's current text in the log: a new line, a rewrite of its line, or nothing.
/// `in_text` says the row is above the screen's last text, where a blank row is a blank line.
fn sync_row(row: &mut Row, sink: &mut dyn LogSink, now: Instant, in_text: bool) {
    let dirty = std::mem::take(&mut row.dirty);
    if dirty {
        row.changed_at = Some(now);
    }
    if row.line.is_some() && !dirty {
        return;
    }
    let (text, spans) = row.text_and_spans();
    if row.sealed && row.shown.as_deref() != Some(text.as_str()) {
        // Keep the line as it was before the full-screen program; what the row shows now is
        // new output (or nothing).
        if let Some(shown) = row.shown.take().filter(|s| !s.is_empty()) {
            if row.written != Some(hash_text(&shown)) {
                sink.settle(&shown);
            }
        }
        row.line = None;
        row.written = None;
        row.blank_since = None;
        row.sealed = false;
        row.erased = text.is_empty();
    }
    if text.is_empty() {
        match row.line {
            // Had text: keep it in the log a moment, in case this is a redraw.
            Some(_) if row.shown.as_deref().is_some_and(|s| !s.is_empty()) => {
                row.blank_since.get_or_insert(now);
            }
            Some(_) => {}
            None if in_text && !row.erased => {
                row.line = Some(sink.push(String::new(), None));
                row.shown = Some(String::new());
            }
            None => {}
        }
        return;
    }
    row.blank_since = None;
    row.erased = false;
    if row.shown.as_deref() == Some(text.as_str()) {
        return;
    }
    let spans = to_spans(&spans);
    match row.line {
        Some(number) => sink.amend(number, text.clone(), spans),
        None => row.line = Some(sink.push(text.clone(), spans)),
    }
    row.shown = Some(text);
}

/// Writes a row to disk once it has been still long enough (or at once with `now_final`), and
/// lets go of rows that stayed erased.
fn settle_row(row: &mut Row, sink: &mut dyn LogSink, now: Instant, now_final: bool) {
    if row.line.is_none() {
        return;
    }
    let still = row
        .changed_at
        .is_none_or(|at| now.saturating_duration_since(at) >= SETTLE_AFTER);
    let erased_for_good = row
        .blank_since
        .is_some_and(|at| now.saturating_duration_since(at) >= ERASE_AFTER);
    if now_final || still || erased_for_good {
        if let Some(shown) = row.shown.as_deref() {
            let hash = hash_text(shown);
            if row.written != Some(hash) && !shown.is_empty() {
                sink.settle(shown);
            }
            row.written = Some(hash);
        }
    }
    if erased_for_good || (now_final && row.blank_since.is_some()) {
        row.line = None;
        row.shown = None;
        row.written = None;
        row.blank_since = None;
        row.erased = true;
    }
}

fn to_spans(spans: &[(usize, usize, Style)]) -> LogSpans {
    (!spans.is_empty()).then(|| {
        spans
            .iter()
            .map(|&(start, end, style)| LogSpan {
                start: start as u32,
                end: end as u32,
                style,
            })
            .collect()
    })
}

fn screen_runs(row: &Row) -> Vec<ScreenRun> {
    let end = row
        .cells
        .iter()
        .rposition(|cell| {
            cell.width != 0 && (cell.ch != ' ' || cell.style.bg != TermColor::Default)
                || cell.style.has(term::INVERSE)
        })
        .map_or(0, |i| i + 1);
    let mut runs: Vec<ScreenRun> = Vec::new();
    for (col, cell) in row.cells[..end].iter().enumerate() {
        if cell.width == 0 {
            if let Some(last) = runs.last_mut() {
                last.cells += 1;
            }
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.style == cell.style && last.col + last.cells == col => {
                last.text.push(cell.ch);
                last.cells += 1;
            }
            _ => runs.push(ScreenRun {
                col,
                cells: 1,
                text: cell.ch.to_string(),
                style: cell.style,
            }),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log that remembers what it was told.
    #[derive(Default)]
    struct TestLog {
        lines: Vec<String>,
        settled: Vec<String>,
    }

    impl LogSink for TestLog {
        fn push(&mut self, text: String, _spans: LogSpans) -> u64 {
            self.lines.push(text);
            self.lines.len() as u64
        }
        fn amend(&mut self, number: u64, text: String, _spans: LogSpans) {
            self.lines[number as usize - 1] = text;
        }
        fn settle(&mut self, text: &str) {
            self.settled.push(text.to_string());
        }
    }

    struct Harness {
        term: Term,
        parser: vte::Parser,
        log: TestLog,
        start: Instant,
    }

    impl Harness {
        fn new(cols: usize, rows: usize) -> Self {
            Self {
                term: Term::new(cols, rows),
                parser: vte::Parser::new(),
                log: TestLog::default(),
                start: Instant::now(),
            }
        }

        /// Feeds output and syncs `ms` milliseconds after the start.
        fn feed(&mut self, bytes: &str, ms: u64) {
            self.parser.advance(&mut self.term, bytes.as_bytes());
            self.sync(ms, false);
        }

        fn sync(&mut self, ms: u64, finish: bool) {
            let now = self.start + Duration::from_millis(ms);
            sync_term(&mut self.term, &mut self.log, now, finish);
        }
    }

    #[test]
    fn lines_join_the_log_as_soon_as_they_are_written() {
        let mut h = Harness::new(40, 10);
        h.feed("starting\r\nlistening on 3000\r\n", 0);
        assert_eq!(h.log.lines, ["starting", "listening on 3000"]);
        assert!(h.log.settled.is_empty());
        h.sync(1500, false);
        assert_eq!(h.log.settled, ["starting", "listening on 3000"]);
    }

    #[test]
    fn a_progress_bar_is_one_line_that_changes() {
        let mut h = Harness::new(40, 10);
        h.feed("download\r\n 10%", 0);
        h.feed("\r 50%", 100);
        h.feed("\r100%\r\ndone\r\n", 200);
        assert_eq!(h.log.lines, ["download", "100%", "done"]);
        h.sync(2000, false);
        assert_eq!(h.log.settled, ["download", "100%", "done"]);
    }

    #[test]
    fn a_redraw_by_position_rewrites_its_line() {
        let mut h = Harness::new(40, 10);
        h.feed("error x\r\nWorking 0%\r\n", 0);
        h.feed("\x1b[2;1HWorking 60%\x1b[K\x1b[3;1H", 300);
        h.feed("\x1b[2;1HWorking 100%\x1b[K\r\nfinished\r\n", 600);
        assert_eq!(h.log.lines, ["error x", "Working 100%", "finished"]);
    }

    #[test]
    fn a_brief_erase_keeps_the_line_and_a_lasting_one_lets_it_go() {
        let mut h = Harness::new(40, 10);
        h.feed("keep me\r\n", 0);
        h.feed("\x1b[1;1H\x1b[K", 100);
        assert_eq!(h.log.lines, ["keep me"]);
        h.feed("keep me", 150);
        assert_eq!(h.log.lines, ["keep me"]);
        h.feed("\x1b[1;1H\x1b[K", 200);
        h.sync(700, false);
        h.feed("\x1b[1;1Hnew text", 800);
        assert_eq!(h.log.lines, ["keep me", "new text"]);
    }

    #[test]
    fn blank_lines_between_text_are_kept() {
        let mut h = Harness::new(40, 10);
        h.feed("a\r\n\r\n", 0);
        h.feed("b\r\n", 10);
        assert_eq!(h.log.lines, ["a", "", "b"]);
    }

    #[test]
    fn scrolled_and_cleared_rows_stay_in_the_log_in_order() {
        let mut h = Harness::new(20, 3);
        h.feed("1\r\n2\r\n3\r\n4\r\n", 0);
        h.feed("\x1b[2J\x1b[Hafter clear", 10);
        assert_eq!(h.log.lines, ["1", "2", "3", "4", "after clear"]);
        h.sync(20, true);
        assert_eq!(h.log.settled, ["1", "2", "3", "4", "after clear"]);
    }

    #[test]
    fn lines_from_before_a_full_screen_program_survive_its_repaint() {
        let mut h = Harness::new(20, 4);
        h.feed("build ok\r\nserving\r\n", 0);
        h.feed("\x1b[?1049h\x1b[Hmenu", 10);
        // The repaint after the program: the old rows erased and rewritten differently.
        h.feed("\x1b[?1049l\x1b[H\x1b[2K\x1b[2;1H\x1b[2K\x1b[Hbye", 20);
        h.sync(1000, false);
        assert_eq!(h.log.lines, ["build ok", "serving", "bye"]);
        assert!(h.log.settled.contains(&"serving".to_string()));
    }

    #[test]
    fn full_screen_programs_stay_out_of_the_log() {
        let mut h = Harness::new(20, 4);
        h.feed("before\r\n", 0);
        h.feed("\x1b[?1049h\x1b[Hmenu\x1b[2;1Hitem", 10);
        h.feed("\x1b[?1049l", 20);
        h.feed("after\r\n", 30);
        assert_eq!(h.log.lines, ["before", "after"]);
    }
}
