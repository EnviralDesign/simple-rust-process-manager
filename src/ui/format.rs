//! Words and numbers: how a status, a schedule, a size or a duration is said, everywhere in the
//! app. Pure functions, so the wording is tested in one place.

use chrono::{Datelike, Duration as ChronoDuration, Local, Timelike};

use crate::config::{
    ManagedRestartSchedule, ProcessConfig, ProcessType, ScheduledRun, ScheduledRunMode,
    TerminalMode,
};
use crate::log_classification::contains_error_indicator;
use crate::process_manager::{ProcessResourceUsage, ProcessRuntimeInfo, ProcessStatus};

use super::widgets::Light;

/// How a process's state reads: its light, the full sentence, the short sidebar note, and its
/// tone for colouring the sentence.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusView {
    pub light: Light,
    pub word: String,
    pub short: String,
    pub tone: Tone,
    /// The process needs attention (failed or exited with an error).
    pub alert: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Busy,
    Bad,
    Quiet,
}

pub fn status_view(
    config: &ProcessConfig,
    status: &ProcessStatus,
    info: Option<&ProcessRuntimeInfo>,
    usage: Option<ProcessResourceUsage>,
) -> StatusView {
    let docker = config.process_type == ProcessType::Docker;
    let restart_pending = info.is_some_and(|info| info.restart_pending);
    let last_exit = info.and_then(|info| info.last_exit);
    let view = |light, word: String, short: String, tone, alert| StatusView {
        light,
        word,
        short,
        tone,
        alert,
    };
    match status {
        ProcessStatus::Running => {
            let short = if docker {
                "docker".to_string()
            } else {
                usage
                    .and_then(|usage| usage.cpu_percent)
                    .map(format_cpu)
                    .unwrap_or_default()
            };
            view(Light::Running, "Running".into(), short, Tone::Good, false)
        }
        ProcessStatus::Starting => view(
            Light::Busy,
            if config.startup_delay_seconds > 0 {
                format!("Starting · waits {} s", config.startup_delay_seconds)
            } else {
                "Starting…".into()
            },
            "starting".into(),
            Tone::Busy,
            false,
        ),
        ProcessStatus::Stopping => view(
            Light::Busy,
            "Stopping…".into(),
            "stopping".into(),
            Tone::Busy,
            false,
        ),
        ProcessStatus::Error(_) => view(
            Light::Error,
            "Failed to start".into(),
            "failed".into(),
            Tone::Bad,
            true,
        ),
        ProcessStatus::Stopped => {
            if let Some(exit) = last_exit.filter(|exit| !exit.success) {
                let code = exit
                    .code
                    .map(|code| format!("Exited with code {code}"))
                    .unwrap_or_else(|| "Ended by a signal".to_string());
                let short = exit
                    .code
                    .map(|code| format!("exit {code}"))
                    .unwrap_or_else(|| "signal".to_string());
                if restart_pending {
                    return view(
                        Light::Error,
                        format!("{code} · restarting"),
                        "restarting".into(),
                        Tone::Bad,
                        true,
                    );
                }
                return view(Light::Error, code, short, Tone::Bad, true);
            }
            if restart_pending {
                return view(
                    Light::Busy,
                    "Exited · restarting".into(),
                    "restarting".into(),
                    Tone::Busy,
                    false,
                );
            }
            if config.scheduled_run.enabled {
                let next = next_scheduled_run(&config.scheduled_run, Local::now());
                return view(
                    Light::Scheduled,
                    format!("Scheduled · next {next}"),
                    next,
                    Tone::Quiet,
                    false,
                );
            }
            if last_exit.is_some_and(|exit| exit.success) {
                return view(
                    Light::Stopped,
                    "Exited with code 0".into(),
                    String::new(),
                    Tone::Quiet,
                    false,
                );
            }
            view(
                Light::Stopped,
                "Stopped".into(),
                String::new(),
                Tone::Quiet,
                false,
            )
        }
    }
}

/// When a scheduled run fires next, as `02:00` within a day or `Thu 02:00` beyond.
pub fn next_scheduled_run(run: &ScheduledRun, now: chrono::DateTime<Local>) -> String {
    let top_of_hour = now
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(now);
    for step in 1..=24 * 8 {
        let at = top_of_hour + ChronoDuration::hours(step);
        let day = at.weekday().num_days_from_monday() as usize;
        if run.due_at(day, at.hour(), 0) {
            return if at - now <= ChronoDuration::hours(24) {
                format!("{:02}:00", at.hour())
            } else {
                format!("{} {:02}:00", day_label(day), at.hour())
            };
        }
    }
    "never".to_string()
}

pub fn schedule_text(run: &ScheduledRun) -> String {
    let hour = format!("{:02}:00", run.hour.min(23));
    match run.mode {
        ScheduledRunMode::Hourly => "Every hour".into(),
        ScheduledRunMode::EveryNHours => format!("Every {} hours", run.interval_hours.clamp(1, 24)),
        ScheduledRunMode::Daily => format!("Daily at {hour}"),
        ScheduledRunMode::SelectedWeekdays => {
            let days: Vec<&str> = (0..7)
                .filter(|day| run.weekdays.get(*day).copied().unwrap_or(false))
                .map(day_label)
                .collect();
            let weekdays = run.weekdays.iter().take(7).copied().collect::<Vec<_>>()
                == [true, true, true, true, true, false, false];
            let days = if weekdays {
                "Mon–Fri".to_string()
            } else if days.is_empty() {
                "No days".to_string()
            } else {
                days.join(", ")
            };
            format!("{days} at {hour}")
        }
    }
}

/// The active-hours grid in a few words: `always`, `weekdays 08–20`, `40 h a week`.
pub fn hours_text(hours: &[bool]) -> String {
    let on = hours.iter().filter(|h| **h).count();
    if on == hours.len() && !hours.is_empty() {
        return "always".into();
    }
    if on == 0 {
        return "never".into();
    }
    let at = |day: usize, hour: usize| hours.get(day * 24 + hour).copied().unwrap_or(false);
    let first_day: Vec<usize> = (0..24).filter(|h| at(0, *h)).collect();
    if let (Some(&from), Some(&last)) = (first_day.first(), first_day.last()) {
        let to = last + 1;
        let contiguous = first_day.len() == to - from;
        let weekdays_same = (0..5).all(|day| (0..24).all(|h| at(day, h) == (h >= from && h < to)));
        let weekend_off = (5..7).all(|day| (0..24).all(|h| !at(day, h)));
        if contiguous && weekdays_same && weekend_off {
            return format!("weekdays {from:02}–{to:02}");
        }
    }
    format!("{on} h a week")
}

pub fn managed_restart_text(schedule: &ManagedRestartSchedule) -> String {
    if !schedule.enabled {
        return "Managed restart".into();
    }
    format!("Managed restart · {}", hours_text(&schedule.hours))
}

/// Which stack controls a process ignores, in words; empty when it answers all three.
pub fn skip_text(process: &ProcessConfig) -> String {
    let mut skipped = Vec::new();
    if !process.respond_to_start_all {
        skipped.push("Start all");
    }
    if !process.respond_to_stop_all {
        skipped.push("Stop all");
    }
    if !process.respond_to_restart_all {
        skipped.push("Restart all");
    }
    match skipped.len() {
        0 => String::new(),
        3 => "Manual only".into(),
        _ => format!("Skips {}", skipped.join(", ")),
    }
}

/// The badges under a process's name.
/// The Output choices in a process's settings, in menu order.
pub const TERMINAL_MODES: [TerminalMode; 3] =
    [TerminalMode::Off, TerminalMode::Auto, TerminalMode::Screen];

/// A terminal mode as the Output menu names it.
pub fn terminal_mode_label(mode: TerminalMode) -> &'static str {
    match mode {
        TerminalMode::Off => "Plain output",
        TerminalMode::Auto => "Terminal",
        TerminalMode::Screen => "Terminal, opens on its screen",
    }
}

/// What a terminal mode does, under the Output menu.
pub fn terminal_mode_hint(mode: TerminalMode) -> &'static str {
    match mode {
        TerminalMode::Off => {
            "Read through pipes, as before 2.1: plain lines, no colour. Tools that need a \
             terminal may refuse to start or draw badly."
        }
        TerminalMode::Auto => {
            "Runs in a terminal. The log keeps the program's colours and progress bars update \
             in place; a full-screen program switches the console to its screen."
        }
        TerminalMode::Screen => {
            "Runs in a terminal and opens on its screen, where you can type into it. Its \
             lines still reach the log."
        }
    }
}

pub fn badges(process: &ProcessConfig) -> Vec<String> {
    let mut badges = vec![match process.process_type {
        ProcessType::Process => "Process".to_string(),
        ProcessType::Docker => "Docker container".to_string(),
    }];
    if process.process_type == ProcessType::Process {
        match process.terminal_mode {
            TerminalMode::Off => {}
            TerminalMode::Auto => badges.push("In a terminal".into()),
            TerminalMode::Screen => badges.push("In a terminal · opens on its screen".into()),
        }
    }
    if process.auto_start {
        badges.push(if process.startup_delay_seconds > 0 {
            format!("Auto-start after {} s", process.startup_delay_seconds)
        } else {
            "Auto-start".into()
        });
    }
    if process.auto_restart {
        badges.push(managed_restart_text(&process.restart_schedule));
    }
    if process.scheduled_run.enabled {
        badges.push(schedule_text(&process.scheduled_run));
    }
    let skip = skip_text(process);
    if !skip.is_empty() {
        badges.push(skip);
    }
    if process.log_to_disk {
        badges.push(format!(
            "Logs to disk · keeps {}",
            process.log_rotation_count
        ));
    }
    badges
}

/// The short behaviour tags in the overview table.
pub fn flags(process: &ProcessConfig) -> Vec<String> {
    let mut flags = Vec::new();
    if process.process_type == ProcessType::Docker {
        flags.push("Docker".into());
    }
    if process.auto_start {
        flags.push("Auto-start".into());
    }
    if process.auto_restart {
        flags.push("Managed".into());
    }
    if process.scheduled_run.enabled {
        flags.push(schedule_text(&process.scheduled_run));
    }
    if !process.respond_to_start_all
        && !process.respond_to_stop_all
        && !process.respond_to_restart_all
    {
        flags.push("Manual".into());
    }
    flags
}

pub fn day_label(index: usize) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        .get(index)
        .copied()
        .unwrap_or("Sun")
}

pub fn format_cpu(value: f32) -> String {
    if value < 9.95 {
        format!("{:.1}%", value.max(0.0))
    } else {
        format!("{:.0}%", value)
    }
}

/// Memory as a number and a unit, so the unit can be set smaller.
pub fn memory_parts(bytes: u64) -> (String, &'static str) {
    let mib = bytes as f64 / 1_048_576.0;
    if mib >= 1024.0 {
        (format!("{:.2}", mib / 1024.0), "GB")
    } else if mib >= 10.0 {
        (format!("{:.0}", mib), "MB")
    } else {
        (format!("{:.1}", mib), "MB")
    }
}

pub fn format_memory(bytes: u64) -> String {
    let (value, unit) = memory_parts(bytes);
    format!("{value} {unit}")
}

/// Uptime: `42s`, `3m 05s`, `2h 14m`, `3d 1h`.
pub fn duration_text(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else if s < 86_400 {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {}h", s / 86_400, (s % 86_400) / 3600)
    }
}

pub fn uptime_seconds(info: Option<&ProcessRuntimeInfo>) -> Option<i64> {
    info.and_then(|info| info.running_since)
        .map(|since| (Local::now() - since).num_seconds())
}

/// A count with thousands separators: `1,284`.
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// When an activity entry happened: `14:02:11` today, `Mon 19:42:05` before.
pub fn when(at: chrono::DateTime<Local>, now: chrono::DateTime<Local>) -> String {
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M:%S").to_string()
    } else {
        format!(
            "{} {}",
            day_label(at.weekday().num_days_from_monday() as usize),
            at.format("%H:%M:%S")
        )
    }
}

/// What a log line is, for colour and the level filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Normal,
    Warning,
    Error,
    /// A line Process Manager wrote itself, such as `[Started with PID 9772]`.
    Event,
}

/// Which stream a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Out,
    Err,
    Event,
}

/// Splits a stored log line into its stream, kind and the text to show. The manager stores
/// stderr lines with a `[stderr] ` prefix and its own events in square brackets.
pub fn classify(line: &str) -> (Stream, LineKind, &str) {
    let trimmed = line.trim_end();
    let (stream, content) = match trimmed
        .strip_prefix("[stderr] ")
        .or_else(|| trimmed.strip_prefix("[stderr]"))
    {
        Some(rest) => (Stream::Err, rest),
        None => (Stream::Out, trimmed),
    };
    let lead = content.trim_start();
    if stream == Stream::Out && lead.starts_with('[') && lead.ends_with(']') && is_event_text(lead)
    {
        let inner = &lead[1..lead.len() - 1];
        let failed_exit = inner.starts_with("Process exited") && !inner.ends_with(": 0");
        let kind = if inner.starts_with("Failed") || inner.contains("error") || failed_exit {
            LineKind::Error
        } else {
            LineKind::Event
        };
        return (Stream::Event, kind, inner);
    }
    let lower = content.to_ascii_lowercase();
    let kind = if contains_error_indicator(content) {
        LineKind::Error
    } else if lower.contains("warn") {
        LineKind::Warning
    } else {
        LineKind::Normal
    };
    (stream, kind, content)
}

/// The manager's own event lines, as opposed to program output that happens to be bracketed.
fn is_event_text(text: &str) -> bool {
    const EVENTS: [&str; 14] = [
        "[Started with PID",
        "[Process exited",
        "[Process stopped",
        "[Failed to start",
        "[Managed",
        "[Scheduled run",
        "[Startup delay",
        "[Docker container",
        "[Container stopped",
        "[Disk logging",
        "[Stop error",
        "[stdout reader error",
        "[stderr reader error",
        "[docker log reader error",
    ];
    EVENTS.iter().any(|prefix| text.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_manager::ExitSummary;

    fn process() -> ProcessConfig {
        ProcessConfig::new(
            "api".into(),
            "uv run dev".into(),
            String::new(),
            ProcessType::Process,
        )
    }

    #[test]
    fn statuses_read_as_plain_sentences() {
        let p = process();
        let info = |exit: Option<ExitSummary>, pending| ProcessRuntimeInfo {
            last_exit: exit,
            restart_pending: pending,
            ..Default::default()
        };
        let usage = Some(ProcessResourceUsage {
            cpu_percent: Some(3.24),
            memory_bytes: None,
        });
        assert_eq!(
            status_view(&p, &ProcessStatus::Running, None, usage).short,
            "3.2%"
        );
        let failed = ExitSummary {
            code: Some(1),
            success: false,
        };
        let v = status_view(
            &p,
            &ProcessStatus::Stopped,
            Some(&info(Some(failed), false)),
            None,
        );
        assert_eq!(
            (v.word.as_str(), v.short.as_str(), v.light),
            ("Exited with code 1", "exit 1", Light::Error)
        );
        let v = status_view(
            &p,
            &ProcessStatus::Stopped,
            Some(&info(Some(failed), true)),
            None,
        );
        assert_eq!(v.word, "Exited with code 1 · restarting");
        let v = status_view(&p, &ProcessStatus::Stopped, Some(&info(None, false)), None);
        assert_eq!((v.word.as_str(), v.light), ("Stopped", Light::Stopped));
        let v = status_view(&p, &ProcessStatus::Error("boom".into()), None, None);
        assert!(v.alert && v.word == "Failed to start");
    }

    #[test]
    fn schedules_and_hours_read_as_words() {
        let mut run = ScheduledRun {
            enabled: true,
            hour: 2,
            ..Default::default()
        };
        assert_eq!(schedule_text(&run), "Daily at 02:00");
        run.mode = ScheduledRunMode::SelectedWeekdays;
        run.weekdays = vec![true, true, true, true, true, false, false];
        assert_eq!(schedule_text(&run), "Mon–Fri at 02:00");
        let mut hours = vec![false; 168];
        for day in 0..5 {
            for hour in 8..20 {
                hours[day * 24 + hour] = true;
            }
        }
        assert_eq!(hours_text(&hours), "weekdays 08–20");
        assert_eq!(hours_text(&vec![true; 168]), "always");
        hours[6 * 24] = true;
        assert_eq!(hours_text(&hours), "61 h a week");
    }

    #[test]
    fn the_next_daily_run_is_within_a_day() {
        let run = ScheduledRun {
            enabled: true,
            hour: 2,
            ..Default::default()
        };
        let now = Local::now();
        let next = next_scheduled_run(&run, now);
        assert_eq!(next, "02:00");
    }

    #[test]
    fn numbers_are_short_and_stable() {
        assert_eq!(duration_text(42), "42s");
        assert_eq!(duration_text(185), "3m 05s");
        assert_eq!(duration_text(8051), "2h 14m");
        assert_eq!(duration_text(261_204), "3d 0h");
        assert_eq!(thousands(1_284), "1,284");
        assert_eq!(thousands(12), "12");
        assert_eq!(format_memory(184 * 1_048_576), "184 MB");
        assert_eq!(format_memory(1_288_490_189), "1.20 GB");
    }

    #[test]
    fn log_lines_split_into_stream_kind_and_text() {
        assert_eq!(
            classify("[stderr] INFO:     Application startup complete."),
            (
                Stream::Err,
                LineKind::Normal,
                "INFO:     Application startup complete."
            )
        );
        assert_eq!(
            classify("[Started with PID 9772]"),
            (Stream::Event, LineKind::Event, "Started with PID 9772")
        );
        assert_eq!(classify("[Failed to start: nope]").1, LineKind::Error);
        assert_eq!(classify("WARNING:  slow query").1, LineKind::Warning);
        assert_eq!(
            classify("Traceback (most recent call last):").1,
            LineKind::Error
        );
        // Program output in brackets is not one of ours.
        assert_eq!(classify("[worker] job 9214 done").0, Stream::Out);
    }
}
