//! Process management logic for starting, stopping, and monitoring processes.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Datelike, Timelike};
use serde::Serialize;
use tokio::sync::watch;

use crate::config::{ProcessConfig, ProcessType, TerminalMode};
use crate::log_classification::contains_error_indicator;
use crate::terminal::{LogSink, LogSpans, TerminalSession};

const IN_MEMORY_LOG_LIMIT: usize = 1000;
const ACTIVITY_LIMIT: usize = 200;
/// Resource samples kept per process for the sparklines: two minutes at the 2 s sample interval.
pub const RESOURCE_HISTORY_LIMIT: usize = 60;
/// Lines per process merged into the stack log.
const STACK_LOG_LINES_PER_PROCESS: usize = 120;
const PROCESS_LOG_FOLDER_NAME: &str = "Process Manager logs";
const RESOURCE_SAMPLE_INTERVAL: Duration = Duration::from_secs(2);
const CPU_TIME_UNITS_PER_SECOND: f64 = 10_000_000.0;
/// How long a stopped or exited terminal process waits for its last output to arrive.
const TERMINAL_DRAIN: Duration = Duration::from_millis(1500);

type SharedLogFile = Arc<Mutex<File>>;
pub type SharedTerminal = Arc<Mutex<TerminalSession>>;

/// Status of a managed process
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    Stopped,
    Running,
    Starting,
    Stopping,
    Error(String),
}

impl std::fmt::Display for ProcessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessStatus::Stopped => write!(f, "Stopped"),
            ProcessStatus::Running => write!(f, "Running"),
            ProcessStatus::Starting => write!(f, "Starting"),
            ProcessStatus::Stopping => write!(f, "Stopping"),
            ProcessStatus::Error(e) => write!(f, "Error: {}", e),
        }
    }
}

/// One line of a process's in-memory log and when it arrived.
#[derive(Debug, Clone)]
pub struct LogLine {
    pub text: String,
    pub at: chrono::DateTime<chrono::Local>,
    /// Colours and emphasis, for lines from a terminal.
    pub spans: LogSpans,
}

/// A running process: spawned with pipes, or in a pseudo-terminal.
pub enum ManagedChild {
    Piped(Child),
    Terminal {
        child: Box<dyn portable_pty::Child + Send + Sync>,
        pid: u32,
    },
}

/// How a process ended, from either kind of child.
struct ChildExit {
    code: Option<i32>,
    success: bool,
    text: String,
}

impl ManagedChild {
    pub fn id(&self) -> u32 {
        match self {
            ManagedChild::Piped(child) => child.id(),
            ManagedChild::Terminal { pid, .. } => *pid,
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ChildExit>> {
        Ok(match self {
            ManagedChild::Piped(child) => child.try_wait()?.map(|status| ChildExit {
                code: status.code(),
                success: status.success(),
                text: status.to_string(),
            }),
            ManagedChild::Terminal { child, .. } => child.try_wait()?.map(|status| ChildExit {
                code: Some(status.exit_code() as i32),
                success: status.success(),
                text: format!("exit code: {}", status.exit_code()),
            }),
        })
    }

    fn wait(&mut self) {
        match self {
            ManagedChild::Piped(child) => {
                let _ = child.wait();
            }
            ManagedChild::Terminal { child, .. } => {
                let _ = child.wait();
            }
        }
    }

    fn kill(&mut self) -> std::io::Result<()> {
        match self {
            ManagedChild::Piped(child) => child.kill(),
            ManagedChild::Terminal { child, .. } => child.kill(),
        }
    }

    /// Stops the process and everything it started (Unix: its process group).
    #[cfg(unix)]
    fn stop_group(&mut self) -> std::io::Result<()> {
        let pid = self.id();
        crate::platform::stop_process_group(pid, || {
            let _ = self.try_wait();
        })
    }
}

/// What kind of thing happened to a process, for the Activity view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Started,
    Stopped,
    Exited,
    Failed,
    Settings,
}

/// One entry of a process's session history: what happened and what caused it.
#[derive(Debug, Clone)]
pub struct ActivityEvent {
    pub at: chrono::DateTime<chrono::Local>,
    pub kind: ActivityKind,
    pub text: String,
    pub cause: String,
}

/// How the last run of a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitSummary {
    /// The exit code, when the platform reports one (a Unix signal has none).
    pub code: Option<i32>,
    pub success: bool,
}

/// Runtime facts about a process beyond its status, for the UI.
#[derive(Debug, Clone, Default)]
pub struct ProcessRuntimeInfo {
    pub pid: Option<u32>,
    pub running_since: Option<chrono::DateTime<chrono::Local>>,
    /// Starts that reached Running this session.
    pub start_count: u32,
    pub last_exit: Option<ExitSummary>,
    /// A managed restart is about to start the process again.
    pub restart_pending: bool,
    /// Lines ever added to the in-memory log this session, so line numbers stay stable as old
    /// lines fall off the front.
    pub log_total: u64,
    /// The process has a terminal (running, or its last screen kept after it ended).
    pub has_terminal: bool,
    /// A full-screen program has its terminal's alternate screen.
    pub alt_screen: bool,
}

/// Runtime state for a single process
pub struct ProcessState {
    pub config: ProcessConfig,
    pub status: ProcessStatus,
    pub logs: Vec<LogLine>,
    pub disk_log: Option<SharedLogFile>,
    pub child: Option<ManagedChild>,
    /// The terminal of a process run in one; kept after it ends so its last screen stays.
    pub terminal: Option<SharedTerminal>,
    /// Mirrors the terminal's alternate-screen flag, so snapshots need not lock the terminal.
    terminal_alt: bool,
    pub resource_usage: ProcessResourceUsage,
    pub suppress_restart_once: bool,
    resource_sample: Option<ResourceSample>,
    start_generation: u64,
    log_total: u64,
    activity: VecDeque<ActivityEvent>,
    resource_history: VecDeque<ProcessResourceUsage>,
    start_cause: String,
    stop_cause: String,
    running_since: Option<chrono::DateTime<chrono::Local>>,
    start_count: u32,
    last_exit: Option<ExitSummary>,
    restart_pending: bool,
    #[cfg(windows)]
    pub job: Option<JobHandle>,
}

impl ProcessState {
    pub fn new(config: ProcessConfig) -> Self {
        Self {
            config,
            status: ProcessStatus::Stopped,
            logs: Vec::new(),
            disk_log: None,
            child: None,
            terminal: None,
            terminal_alt: false,
            resource_usage: ProcessResourceUsage::default(),
            suppress_restart_once: false,
            resource_sample: None,
            start_generation: 0,
            log_total: 0,
            activity: VecDeque::new(),
            resource_history: VecDeque::new(),
            start_cause: String::new(),
            stop_cause: String::new(),
            running_since: None,
            start_count: 0,
            last_exit: None,
            restart_pending: false,
            #[cfg(windows)]
            job: None,
        }
    }

    fn record(&mut self, kind: ActivityKind, text: impl Into<String>, cause: impl Into<String>) {
        self.activity.push_back(ActivityEvent {
            at: chrono::Local::now(),
            kind,
            text: text.into(),
            cause: cause.into(),
        });
        while self.activity.len() > ACTIVITY_LIMIT {
            self.activity.pop_front();
        }
    }

    /// Puts the process into Starting for a new start request. Earlier output stays in the log:
    /// a crash's traceback must survive the managed restart that follows it.
    fn begin_start(&mut self, cause: impl Into<String>) -> u64 {
        self.suppress_restart_once = false;
        self.status = ProcessStatus::Starting;
        self.disk_log = None;
        let _ = clear_resource_usage(self);
        self.start_generation = self.start_generation.wrapping_add(1);
        self.start_cause = cause.into();
        self.restart_pending = false;
        self.last_exit = None;
        self.start_generation
    }

    fn mark_running(&mut self, pid: Option<u32>) {
        self.status = ProcessStatus::Running;
        self.running_since = Some(chrono::Local::now());
        self.start_count = self.start_count.saturating_add(1);
        let text = match pid {
            Some(pid) => format!("Started · PID {pid}"),
            None => "Container started".to_string(),
        };
        let cause = std::mem::take(&mut self.start_cause);
        self.record(ActivityKind::Started, text, cause);
    }

    fn mark_failed(&mut self, error: &str) {
        self.running_since = None;
        let cause = std::mem::take(&mut self.start_cause);
        self.record(
            ActivityKind::Failed,
            format!("Failed to start: {}", error.trim()),
            cause,
        );
    }

    fn mark_stopped(&mut self) {
        self.running_since = None;
        let cause = std::mem::take(&mut self.stop_cause);
        self.record(ActivityKind::Stopped, "Stopped", cause);
    }

    fn runtime_info(&self) -> ProcessRuntimeInfo {
        ProcessRuntimeInfo {
            pid: self.child.as_ref().map(|child| child.id()),
            running_since: self.running_since,
            start_count: self.start_count,
            last_exit: self.last_exit,
            restart_pending: self.restart_pending,
            log_total: self.log_total,
            has_terminal: self.terminal.is_some(),
            alt_screen: self.terminal_alt,
        }
    }
}

/// Serializable runtime snapshot for REST responses and other external consumers.
#[derive(Debug, Clone, Serialize)]
pub struct ProcessRuntimeSnapshot {
    pub id: String,
    pub name: String,
    pub process_type: String,
    pub status: String,
    pub status_detail: Option<String>,
    pub pid: Option<u32>,
    pub cpu_percent: Option<f32>,
    pub memory_bytes: Option<u64>,
    pub auto_start: bool,
    pub startup_delay_seconds: u64,
    pub auto_restart: bool,
    pub respond_to_start_all: bool,
    pub respond_to_stop_all: bool,
    pub respond_to_restart_all: bool,
    pub command: String,
    pub working_directory: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct ProcessCounts {
    pub total: usize,
    pub running: usize,
    pub stopped: usize,
    pub starting: usize,
    pub stopping: usize,
    pub error: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct ProcessResourceUsage {
    pub cpu_percent: Option<f32>,
    pub memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct ResourceSample {
    measured_at: Instant,
    cpu_time_100ns: u64,
}

#[derive(Debug, Clone, Copy)]
struct ResourceCounterSample {
    cpu_time_100ns: Option<u64>,
    memory_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct UiRuntimeSnapshot {
    pub counts: ProcessCounts,
    pub statuses: HashMap<String, ProcessStatus>,
    pub resource_usage: HashMap<String, ProcessResourceUsage>,
    pub info: HashMap<String, ProcessRuntimeInfo>,
    pub selected_logs: Vec<LogLine>,
    /// Line number (1-based, stable for the session) of `selected_logs[0]`.
    pub selected_first_line: u64,
    pub selected_activity: Vec<ActivityEvent>,
    pub selected_history: Vec<ProcessResourceUsage>,
    /// The newest lines of every process, oldest first, when the stack log was asked for.
    pub stack_logs: Vec<(String, LogLine)>,
}

#[derive(Debug, Clone, Default)]
struct ProcessScheduleState {
    last_restart_schedule_active: Option<bool>,
    last_scheduled_run_minute: Option<i64>,
}

/// Manages all running processes
pub struct ProcessManager {
    pub processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    log_directory: Arc<Mutex<String>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
    error_version: Arc<AtomicU64>,
    process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
    background_started: AtomicBool,
    has_docker_entries: Arc<AtomicBool>,
    schedule_state: Arc<Mutex<HashMap<String, ProcessScheduleState>>>,
}

#[derive(Clone, Copy)]
enum StackControlAction {
    Start,
    Stop,
    Restart,
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessManager {
    pub fn new() -> Self {
        let (event_tx, _event_rx) = watch::channel(0u64);
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
            log_directory: Arc::new(Mutex::new(".".to_string())),
            event_tx,
            event_version: Arc::new(AtomicU64::new(0)),
            error_version: Arc::new(AtomicU64::new(0)),
            process_error_versions: Arc::new(Mutex::new(HashMap::new())),
            background_started: AtomicBool::new(false),
            has_docker_entries: Arc::new(AtomicBool::new(false)),
            schedule_state: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn error_version(&self) -> u64 {
        self.error_version.load(Ordering::Relaxed)
    }

    pub fn error_versions(&self) -> HashMap<String, u64> {
        self.process_error_versions.lock().unwrap().clone()
    }

    pub fn set_log_directory(&self, directory: impl Into<String>) {
        let mut log_directory = self.log_directory.lock().unwrap();
        *log_directory = directory.into();
    }

    fn notify(&self) {
        bump_event(&self.event_tx, &self.event_version);
    }

    pub fn current_version(&self) -> u64 {
        self.event_version.load(Ordering::Relaxed)
    }

    fn update_docker_polling_flag_locked(&self, processes: &HashMap<String, ProcessState>) {
        let has_docker = processes
            .values()
            .any(|state| state.config.process_type == ProcessType::Docker);
        self.has_docker_entries.store(has_docker, Ordering::Relaxed);
    }

    pub fn start_background_tasks(&self) {
        if self.background_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let processes = self.processes.clone();
        let event_tx = self.event_tx.clone();
        let event_version = self.event_version.clone();
        let error_version = self.error_version.clone();
        let process_error_versions = self.process_error_versions.clone();
        let has_docker_entries = self.has_docker_entries.clone();
        let log_directory = self.log_directory.clone();
        let schedule_state = self.schedule_state.clone();

        thread::spawn(move || loop {
            if !has_docker_entries.load(Ordering::Relaxed) {
                thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }

            thread::sleep(std::time::Duration::from_millis(750));

            let docker_ids: Vec<String> = {
                let processes = processes.lock().unwrap();
                processes
                    .iter()
                    .filter(|(_, s)| s.config.process_type == ProcessType::Docker)
                    .map(|(id, _)| id.clone())
                    .collect()
            };

            for id in docker_ids {
                refresh_docker_status_inner(
                    &id,
                    &processes,
                    &log_directory,
                    &event_tx,
                    &event_version,
                    &error_version,
                    &process_error_versions,
                );
            }
        });

        let processes = self.processes.clone();
        let event_tx = self.event_tx.clone();
        let event_version = self.event_version.clone();
        let error_version = self.error_version.clone();
        let process_error_versions = self.process_error_versions.clone();
        let log_directory = self.log_directory.clone();

        thread::spawn(move || loop {
            thread::sleep(std::time::Duration::from_secs(20));
            run_scheduler_tick(
                &processes,
                &log_directory,
                &event_tx,
                &event_version,
                &error_version,
                &process_error_versions,
                &schedule_state,
            );
        });

        let processes = self.processes.clone();
        let event_tx = self.event_tx.clone();
        let event_version = self.event_version.clone();

        thread::spawn(move || loop {
            thread::sleep(RESOURCE_SAMPLE_INTERVAL);
            if refresh_resource_usage(&processes) {
                bump_event(&event_tx, &event_version);
            }
        });
    }

    /// Initialize process states from config
    pub fn init_from_config(&self, configs: &[ProcessConfig]) {
        let mut processes = self.processes.lock().unwrap();
        let mut process_error_versions = self.process_error_versions.lock().unwrap();
        let mut schedule_state = self.schedule_state.lock().unwrap();
        for config in configs {
            if !processes.contains_key(&config.id) {
                processes.insert(config.id.clone(), ProcessState::new(config.clone()));
            }
            process_error_versions.entry(config.id.clone()).or_insert(0);
            schedule_state.entry(config.id.clone()).or_default();
        }
        process_error_versions.retain(|id, _| processes.contains_key(id));
        schedule_state.retain(|id, _| processes.contains_key(id));
        self.update_docker_polling_flag_locked(&processes);
    }

    /// Reload all process states from a new config set.
    pub fn reload_from_config(&self, configs: &[ProcessConfig]) {
        self.stop_all_forced();

        let mut processes = self.processes.lock().unwrap();
        let mut process_error_versions = self.process_error_versions.lock().unwrap();
        let mut schedule_state = self.schedule_state.lock().unwrap();

        processes.clear();
        process_error_versions.clear();
        schedule_state.clear();

        for config in configs {
            let process_id = config.id.clone();
            processes.insert(process_id.clone(), ProcessState::new(config.clone()));
            process_error_versions
                .entry(process_id.clone())
                .or_insert(0);
            schedule_state.entry(process_id).or_default();
        }

        self.update_docker_polling_flag_locked(&processes);
        self.notify();
    }

    /// Add a new process
    pub fn add_process(&self, config: ProcessConfig) {
        let mut processes = self.processes.lock().unwrap();
        let process_id = config.id.clone();
        processes.insert(process_id.clone(), ProcessState::new(config));
        self.process_error_versions
            .lock()
            .unwrap()
            .entry(process_id.clone())
            .or_insert(0);
        self.schedule_state
            .lock()
            .unwrap()
            .entry(process_id)
            .or_default();
        self.update_docker_polling_flag_locked(&processes);
        self.notify();
    }

    /// Update an existing process config while preserving runtime state
    pub fn update_process_config(&self, config: ProcessConfig) -> bool {
        let mut processes = self.processes.lock().unwrap();
        if let Some(state) = processes.get_mut(&config.id) {
            state.config = config;
            self.update_docker_polling_flag_locked(&processes);
            self.notify();
            true
        } else {
            false
        }
    }

    /// Remove a process (stops it first if running)
    pub fn remove_process(&self, id: &str) {
        self.stop_process_by(id, "Deleted");
        let mut processes = self.processes.lock().unwrap();
        processes.remove(id);
        self.process_error_versions.lock().unwrap().remove(id);
        self.schedule_state.lock().unwrap().remove(id);
        self.update_docker_polling_flag_locked(&processes);
        self.notify();
    }

    /// Start a process at the user's request.
    pub fn start_process(&self, id: &str) {
        self.start_process_by(id, "You");
    }

    /// Start a process, recording `cause` (who or what asked) in its activity.
    pub fn start_process_by(&self, id: &str, cause: &str) {
        println!("[DEBUG] start_process called with id: {}", id);
        let processes_arc = self.processes.clone();
        let log_directory = self.log_directory.clone();
        let event_tx = self.event_tx.clone();
        let event_version = self.event_version.clone();
        let error_version = self.error_version.clone();
        let process_error_versions = self.process_error_versions.clone();

        // Get config and update status
        let (config, start_generation) = {
            let mut processes = processes_arc.lock().unwrap();
            println!("[DEBUG] Got lock, looking for process id: {}", id);
            if let Some(state) = processes.get_mut(id) {
                println!(
                    "[DEBUG] Found process: {}, status: {:?}",
                    state.config.name, state.status
                );
                if !process_is_dormant(state) || state.status == ProcessStatus::Starting {
                    println!("[DEBUG] Already running or starting, returning");
                    return; // Already running
                }
                let start_generation = state.begin_start(cause);
                bump_event(&event_tx, &event_version);
                (state.config.clone(), start_generation)
            } else {
                println!("[DEBUG] Process not found in manager!");
                return;
            }
        };

        let id_owned = id.to_string();
        println!(
            "[DEBUG] Starting process type: {:?}, command: {}",
            config.process_type, config.command
        );

        launch_process(
            &id_owned,
            &config,
            start_generation,
            processes_arc,
            log_directory,
            event_tx,
            event_version,
            error_version,
            process_error_versions,
        );
    }

    fn start_system_process(
        id: &str,
        config: &ProcessConfig,
        start_generation: u64,
        processes_arc: Arc<Mutex<HashMap<String, ProcessState>>>,
        log_directory: Arc<Mutex<String>>,
        event_tx: watch::Sender<u64>,
        event_version: Arc<AtomicU64>,
        error_version: Arc<AtomicU64>,
        process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
    ) {
        let id_owned = id.to_string();
        let command = config.command.clone();
        let working_dir = config.working_directory.clone();
        let config_clone = config.clone();

        thread::spawn(move || {
            if !wait_for_startup_delay(
                &id_owned,
                &config_clone,
                start_generation,
                &processes_arc,
                &event_tx,
                &event_version,
            ) {
                return;
            }

            println!("[DEBUG] Thread spawned for command: {}", command);
            println!("[DEBUG] Working dir: '{}'", working_dir);

            let fail = |error: &str| {
                fail_start(
                    &processes_arc,
                    &id_owned,
                    error,
                    &error_version,
                    &process_error_versions,
                    &event_tx,
                    &event_version,
                )
            };

            let (program, args) = match parse_command(&command) {
                Ok((program, args)) => (program, args),
                Err(e) => return fail(&e),
            };

            let spawned = if config_clone.terminal_mode == TerminalMode::Off {
                spawn_piped(&program, &args, &working_dir)
            } else {
                spawn_terminal(&program, &args, &working_dir)
            };
            let (mut child, output, program_label) = match spawned {
                Ok(spawned) => spawned,
                Err(e) => return fail(&e),
            };

            #[cfg(windows)]
            let mut job = match create_job() {
                Ok(job) => {
                    if let Err(e) = assign_job(&job, &child) {
                        eprintln!("[WARN] Failed to assign job: {}", e);
                        None
                    } else {
                        Some(job)
                    }
                }
                Err(e) => {
                    eprintln!("[WARN] Failed to create job: {}", e);
                    None
                }
            };

            println!(
                "[DEBUG] Command spawned successfully! PID: {:?}, Program: '{}'",
                child.id(),
                program_label
            );
            let disk_log = if config_clone.log_to_disk {
                let base_directory = log_directory.lock().unwrap().clone();
                match create_disk_log_session(&base_directory, &config_clone) {
                    Ok(file) => Some(file),
                    Err(err) => {
                        eprintln!(
                            "[WARN] Failed to initialize disk logging for '{}': {}",
                            config_clone.name, err
                        );
                        None
                    }
                }
            } else {
                None
            };

            {
                let mut processes = processes_arc.lock().unwrap();
                if !processes
                    .get(&id_owned)
                    .is_some_and(|state| start_request_is_current_state(state, start_generation))
                {
                    // Stop/shutdown may have arrived between validation and spawn.
                    // Do not publish an already-cancelled child into the managed set.
                    drop(processes);
                    #[cfg(unix)]
                    let _ = child.stop_group();
                    #[cfg(not(unix))]
                    let _ = child.kill();
                    child.wait();
                    output.discard();
                    return;
                }
                if let Some(state) = processes.get_mut(&id_owned) {
                    state.mark_running(Some(child.id()));
                    state.disk_log = disk_log.clone();
                    log_process_state_event(state, format!("[Started with PID {}]", child.id()));
                    if config_clone.log_to_disk && state.disk_log.is_none() {
                        push_in_memory_log(
                            state,
                            "[Disk logging unavailable. See stderr for details.]".to_string(),
                        );
                    }
                    state.terminal = output.terminal();
                    state.terminal_alt = false;
                    println!("[DEBUG] Status set to Running");
                    state.child = Some(child);
                    #[cfg(windows)]
                    {
                        state.job = job.take();
                    }
                }
            }
            bump_event(&event_tx, &event_version);

            match output {
                Output::Piped { stdout, stderr } => {
                    for (reader, is_stderr) in [
                        (
                            stdout.map(|r| Box::new(r) as Box<dyn std::io::Read + Send>),
                            false,
                        ),
                        (
                            stderr.map(|r| Box::new(r) as Box<dyn std::io::Read + Send>),
                            true,
                        ),
                    ] {
                        if let Some(reader) = reader {
                            pump_pipe(
                                reader,
                                is_stderr,
                                id_owned.clone(),
                                processes_arc.clone(),
                                event_tx.clone(),
                                event_version.clone(),
                                error_version.clone(),
                                process_error_versions.clone(),
                            );
                        }
                    }
                }
                Output::Terminal {
                    reader,
                    terminal,
                    done,
                } => pump_terminal(
                    id_owned.clone(),
                    terminal,
                    reader,
                    done,
                    processes_arc.clone(),
                    event_tx.clone(),
                    event_version.clone(),
                    error_version.clone(),
                    process_error_versions.clone(),
                ),
            }

            monitor_child(
                id_owned,
                start_generation,
                processes_arc,
                log_directory,
                event_tx,
                event_version,
                error_version,
                process_error_versions,
            );
        });
    }

    fn start_docker_container(
        id: &str,
        config: &ProcessConfig,
        start_generation: u64,
        processes_arc: Arc<Mutex<HashMap<String, ProcessState>>>,
        log_directory: Arc<Mutex<String>>,
        event_tx: watch::Sender<u64>,
        event_version: Arc<AtomicU64>,
        error_version: Arc<AtomicU64>,
        process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
    ) {
        let id_owned = id.to_string();
        let container_name = config.command.clone();
        let config_clone = config.clone();

        thread::spawn(move || {
            if !wait_for_startup_delay(
                &id_owned,
                &config_clone,
                start_generation,
                &processes_arc,
                &event_tx,
                &event_version,
            ) {
                return;
            }

            let disk_log = if config_clone.log_to_disk {
                let base_directory = log_directory.lock().unwrap().clone();
                match create_disk_log_session(&base_directory, &config_clone) {
                    Ok(file) => Some(file),
                    Err(err) => {
                        eprintln!(
                            "[WARN] Failed to initialize disk logging for '{}': {}",
                            config_clone.name, err
                        );
                        None
                    }
                }
            } else {
                None
            };

            // Start docker container
            let mut cmd = Command::new("docker");
            cmd.args(["start", &container_name]);

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }

            match cmd.output() {
                Ok(output) => {
                    let mut processes = processes_arc.lock().unwrap();
                    if let Some(state) = processes.get_mut(&id_owned) {
                        state.disk_log = disk_log.clone();
                        if output.status.success() {
                            // The status poll may have seen it running first.
                            if state.status != ProcessStatus::Running {
                                state.mark_running(None);
                            }
                            log_process_state_event(
                                state,
                                format!("[Docker container '{}' started]", container_name),
                            );
                            if config_clone.log_to_disk && state.disk_log.is_none() {
                                push_in_memory_log(
                                    state,
                                    "[Disk logging unavailable. See stderr for details.]"
                                        .to_string(),
                                );
                            }
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            state.status = ProcessStatus::Error(stderr.to_string());
                            let _ = clear_resource_usage(state);
                            log_process_state_event(
                                state,
                                format!("[Failed to start: {}]", stderr),
                            );
                            state.mark_failed(&stderr);
                            state.disk_log = None;
                            bump_error(&error_version, &process_error_versions, &id_owned);
                        }
                    }
                    bump_event(&event_tx, &event_version);
                }
                Err(e) => {
                    let mut processes = processes_arc.lock().unwrap();
                    if let Some(state) = processes.get_mut(&id_owned) {
                        state.status = ProcessStatus::Error(e.to_string());
                        let _ = clear_resource_usage(state);
                        log_process_state_event(state, format!("[Failed to start docker: {}]", e));
                        state.mark_failed(&e.to_string());
                        state.disk_log = None;
                    }
                    bump_error(&error_version, &process_error_versions, &id_owned);
                    bump_event(&event_tx, &event_version);
                }
            }

            // Start log streaming for docker
            Self::stream_docker_logs(
                &id_owned,
                &container_name,
                processes_arc,
                event_tx,
                event_version,
                error_version,
                process_error_versions,
            );
        });
    }

    fn stream_docker_logs(
        id: &str,
        container_name: &str,
        processes_arc: Arc<Mutex<HashMap<String, ProcessState>>>,
        event_tx: watch::Sender<u64>,
        event_version: Arc<AtomicU64>,
        error_version: Arc<AtomicU64>,
        process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
    ) {
        let id_owned = id.to_string();
        let container = container_name.to_string();

        thread::spawn(move || {
            let mut cmd = Command::new("docker");
            cmd.args(["logs", "-f", "--tail", "100", &container]);
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::piped());

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }

            if let Ok(mut child) = cmd.spawn() {
                if let Some(stdout) = child.stdout.take() {
                    let mut reader = BufReader::new(stdout);
                    let mut buffer = Vec::new();
                    loop {
                        buffer.clear();
                        let read = match reader.read_until(b'\n', &mut buffer) {
                            Ok(read) => read,
                            Err(err) => {
                                let (updated, has_error) = append_runtime_log(
                                    &processes_arc,
                                    &id_owned,
                                    format!("[docker log reader error: {}]", err),
                                    true,
                                );
                                if updated {
                                    if has_error {
                                        bump_error(
                                            &error_version,
                                            &process_error_versions,
                                            &id_owned,
                                        );
                                    }
                                    bump_event(&event_tx, &event_version);
                                }
                                break;
                            }
                        };
                        if read == 0 {
                            break;
                        }
                        while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                            buffer.pop();
                        }
                        let line = String::from_utf8_lossy(&buffer).into_owned();
                        if line.is_empty() {
                            continue;
                        }
                        let mut should_break = false;
                        let (updated, has_error, disk_log, formatted) = {
                            let mut updated = false;
                            let mut has_error = false;
                            let mut disk_log = None;
                            let mut formatted = String::new();
                            let mut processes = processes_arc.lock().unwrap();
                            if let Some(state) = processes.get_mut(&id_owned) {
                                if state.status != ProcessStatus::Running {
                                    should_break = true;
                                } else {
                                    formatted = line.clone();
                                    has_error = line_has_error(&formatted);
                                    disk_log = state.disk_log.clone();
                                    push_in_memory_log(state, formatted.clone());
                                    updated = true;
                                }
                            } else {
                                should_break = true;
                                updated = false;
                                has_error = false;
                            }
                            (updated, has_error, disk_log, formatted)
                        };
                        if let Some(file) = disk_log {
                            write_disk_log_line(&file, &formatted);
                        }
                        if updated {
                            if has_error {
                                bump_error(&error_version, &process_error_versions, &id_owned);
                            }
                            bump_event(&event_tx, &event_version);
                        }
                        if should_break {
                            break;
                        }
                    }
                }
                let _ = child.kill();
            }
        });
    }

    /// Stop a process at the user's request.
    pub fn stop_process(&self, id: &str) {
        self.stop_process_by(id, "You");
    }

    /// Stop a process, recording `cause` in its activity.
    pub fn stop_process_by(&self, id: &str, cause: &str) {
        stop_process_inner(
            id,
            cause,
            self.processes.clone(),
            self.event_tx.clone(),
            self.event_version.clone(),
        );
    }

    /// Restart a process at the user's request.
    pub fn restart_process(&self, id: &str) {
        self.restart_process_by(id, "You");
    }

    /// Restart a process, recording `cause` in its activity.
    pub fn restart_process_by(&self, id: &str, cause: &str) {
        self.stop_process_by(id, cause);

        // Wait for process to stop (max 5 seconds)
        let start = std::time::Instant::now();
        while start.elapsed().as_secs() < 5 {
            thread::sleep(std::time::Duration::from_millis(100));
            if let Some(status) = self.get_status(id) {
                if status == ProcessStatus::Stopped || matches!(status, ProcessStatus::Error(_)) {
                    break;
                }
            } else {
                break;
            }
        }

        self.start_process_by(id, cause);
    }

    /// Start all processes
    pub fn start_all(&self) {
        let ids: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes
                .iter()
                .filter(|(_, state)| state.config.respond_to_start_all)
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in ids {
            self.start_process_by(&id, "Start all");
        }
    }

    /// Start selected processes that opt into Start All. Returns the number requested.
    pub fn start_processes_respecting_start_all(
        &self,
        process_ids: &[String],
        cause: &str,
    ) -> usize {
        let ids =
            self.process_ids_responding_to_stack_control(process_ids, StackControlAction::Start);
        for id in &ids {
            self.start_process_by(id, cause);
        }
        ids.len()
    }

    /// Start only processes explicitly marked for auto-start on app launch
    pub fn start_auto_start_processes(&self) {
        let ids: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes
                .iter()
                .filter(|(_, state)| state.config.auto_start)
                .map(|(id, _)| id.clone())
                .collect()
        };

        for id in ids {
            self.start_process_by(&id, "Auto-start");
        }
    }

    /// Stop all processes
    pub fn stop_all(&self) {
        let ids: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes
                .iter()
                .filter(|(_, state)| state.config.respond_to_stop_all)
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in ids {
            self.stop_process_by(&id, "Stop all");
        }
    }

    /// Stop selected processes that opt into Stop All. Returns the number requested.
    pub fn stop_processes_respecting_stop_all(&self, process_ids: &[String], cause: &str) -> usize {
        let ids =
            self.process_ids_responding_to_stack_control(process_ids, StackControlAction::Stop);
        for id in &ids {
            self.stop_process_by(id, cause);
        }
        ids.len()
    }

    /// Stop all managed processes regardless of stack-control flags.
    pub fn stop_all_forced(&self) {
        let ids: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes.keys().cloned().collect()
        };
        for id in &ids {
            self.stop_process_by(id, "Reload of processes.json");
        }

        let _ = self.wait_for_processes_to_stop(&ids, Duration::from_secs(5));
    }

    fn wait_for_processes_to_stop(&self, ids: &[String], timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            let all_stopped = {
                let processes = self.processes.lock().unwrap();
                ids.iter().all(|id| {
                    processes.get(id).map_or(true, |state| {
                        matches!(
                            state.status,
                            ProcessStatus::Stopped | ProcessStatus::Error(_)
                        )
                    })
                })
            };

            if all_stopped {
                return true;
            }

            if start.elapsed() >= timeout {
                return false;
            }

            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Restart all processes
    pub fn restart_all(&self) {
        let ids: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes
                .iter()
                .filter(|(_, state)| state.config.respond_to_restart_all)
                .map(|(id, _)| id.clone())
                .collect()
        };

        for id in &ids {
            self.stop_process_by(id, "Restart all");
        }

        // Wait for all processes to stop (max 5 seconds)
        if !self.wait_for_processes_to_stop(&ids, Duration::from_secs(5)) {
            println!("[WARN] Restart all timeout waiting for stops");
        }

        for id in ids {
            self.start_process_by(&id, "Restart all");
        }
    }

    /// Restart selected processes that opt into Restart All. Returns the number requested.
    pub fn restart_processes_respecting_restart_all(
        &self,
        process_ids: &[String],
        cause: &str,
    ) -> usize {
        let ids =
            self.process_ids_responding_to_stack_control(process_ids, StackControlAction::Restart);

        for id in &ids {
            self.stop_process_by(id, cause);
        }

        if !self.wait_for_processes_to_stop(&ids, Duration::from_secs(5)) {
            println!("[WARN] Restart selected timeout waiting for stops");
        }

        for id in &ids {
            self.start_process_by(id, cause);
        }

        ids.len()
    }

    fn process_ids_responding_to_stack_control(
        &self,
        process_ids: &[String],
        action: StackControlAction,
    ) -> Vec<String> {
        let processes = self.processes.lock().unwrap();
        process_ids
            .iter()
            .filter(|id| {
                processes.get(*id).is_some_and(|state| match action {
                    StackControlAction::Start => state.config.respond_to_start_all,
                    StackControlAction::Stop => state.config.respond_to_stop_all,
                    StackControlAction::Restart => state.config.respond_to_restart_all,
                })
            })
            .cloned()
            .collect()
    }

    /// Stop all non-Docker processes (called on app shutdown)
    pub fn stop_non_docker(&self) {
        let mut processes = self.processes.lock().unwrap();
        for state in processes.values_mut() {
            if state.config.process_type == ProcessType::Process {
                if let Some(ref mut child) = state.child {
                    #[cfg(windows)]
                    let pid = child.id();
                    #[cfg(windows)]
                    {
                        if let Some(job) = state.job.take() {
                            drop(job);
                        } else {
                            let _ = kill_process_tree(pid);
                        }
                    }
                    #[cfg(unix)]
                    {
                        let _ = child.stop_group();
                    }
                    #[cfg(not(any(windows, unix)))]
                    {
                        let _ = child.kill();
                    }
                    child.wait();
                    state.child = None;
                }
                #[cfg(windows)]
                {
                    state.job = None;
                }
                state.status = ProcessStatus::Stopped;
                let _ = clear_resource_usage(state);
            }
        }
        self.notify();
    }

    /// Get status of a process
    pub fn get_status(&self, id: &str) -> Option<ProcessStatus> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).map(|s| s.status.clone())
    }

    pub fn get_process_config(&self, id: &str) -> Option<ProcessConfig> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).map(|state| state.config.clone())
    }

    /// Replace a single managed process with a new config, stopping it first if needed.
    pub fn reload_process_from_config(&self, config: ProcessConfig) -> bool {
        let process_id = config.id.clone();
        if self.processes.lock().unwrap().get(&process_id).is_none() {
            return false;
        }

        if matches!(
            self.get_status(&process_id),
            Some(ProcessStatus::Running | ProcessStatus::Starting | ProcessStatus::Stopping)
        ) {
            self.stop_process_by(&process_id, "Reload from processes.json");
            let _ = self.wait_for_processes_to_stop(&[process_id.clone()], Duration::from_secs(5));
        }

        let mut processes = self.processes.lock().unwrap();
        if let Some(state) = processes.get_mut(&process_id) {
            state.config = config;
            self.update_docker_polling_flag_locked(&processes);
            self.notify();
            true
        } else {
            false
        }
    }

    /// The terminal of a process run in one (its last screen, once it has ended). Lock it only
    /// after this returns: the manager's lock is never held while waiting for a terminal's.
    pub fn terminal(&self, id: &str) -> Option<SharedTerminal> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).and_then(|state| state.terminal.clone())
    }

    pub fn get_recent_logs(&self, id: &str, limit: usize) -> Option<Vec<String>> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).map(|state| {
            let start = state.logs.len().saturating_sub(limit);
            state.logs[start..]
                .iter()
                .map(|line| line.text.clone())
                .collect()
        })
    }

    /// Empties the in-memory log of a process. Line numbers keep counting; disk logs are untouched.
    pub fn clear_logs(&self, id: &str) {
        let mut processes = self.processes.lock().unwrap();
        if let Some(state) = processes.get_mut(id) {
            state.logs.clear();
        }
        drop(processes);
        self.notify();
    }

    /// Adds an entry to a process's activity that did not come from the manager itself, such
    /// as saved settings.
    pub fn record_activity(&self, id: &str, kind: ActivityKind, text: &str, cause: &str) {
        let mut processes = self.processes.lock().unwrap();
        if let Some(state) = processes.get_mut(id) {
            state.record(kind, text, cause);
        }
        drop(processes);
        self.notify();
    }

    pub fn get_log_count(&self, id: &str) -> Option<usize> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).map(|state| state.logs.len())
    }

    pub fn list_processes(&self) -> Vec<ProcessRuntimeSnapshot> {
        let processes = self.processes.lock().unwrap();
        let mut snapshots: Vec<_> = processes
            .values()
            .map(process_snapshot_from_state)
            .collect();
        snapshots.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        snapshots
    }

    pub fn get_process_snapshot(&self, id: &str) -> Option<ProcessRuntimeSnapshot> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).map(process_snapshot_from_state)
    }

    pub fn get_counts(&self) -> ProcessCounts {
        let processes = self.processes.lock().unwrap();
        let mut counts = ProcessCounts {
            total: processes.len(),
            ..ProcessCounts::default()
        };

        for state in processes.values() {
            match &state.status {
                ProcessStatus::Running => counts.running += 1,
                ProcessStatus::Stopped => counts.stopped += 1,
                ProcessStatus::Starting => counts.starting += 1,
                ProcessStatus::Stopping => counts.stopping += 1,
                ProcessStatus::Error(_) => counts.error += 1,
            }
        }

        counts
    }

    pub fn build_ui_snapshot(
        &self,
        selected_id: Option<&str>,
        log_limit: usize,
        include_stack_logs: bool,
    ) -> UiRuntimeSnapshot {
        let processes = self.processes.lock().unwrap();
        let mut snapshot = UiRuntimeSnapshot {
            counts: ProcessCounts {
                total: processes.len(),
                ..ProcessCounts::default()
            },
            statuses: HashMap::with_capacity(processes.len()),
            resource_usage: HashMap::with_capacity(processes.len()),
            info: HashMap::with_capacity(processes.len()),
            ..UiRuntimeSnapshot::default()
        };

        for (id, state) in processes.iter() {
            snapshot.statuses.insert(id.clone(), state.status.clone());
            snapshot
                .resource_usage
                .insert(id.clone(), state.resource_usage);
            snapshot.info.insert(id.clone(), state.runtime_info());

            let counts = &mut snapshot.counts;
            match &state.status {
                ProcessStatus::Running => counts.running += 1,
                ProcessStatus::Stopped => counts.stopped += 1,
                ProcessStatus::Starting => counts.starting += 1,
                ProcessStatus::Stopping => counts.stopping += 1,
                ProcessStatus::Error(_) => counts.error += 1,
            }

            if selected_id == Some(id.as_str()) {
                let start = state.logs.len().saturating_sub(log_limit);
                snapshot.selected_logs = state.logs[start..].to_vec();
                snapshot.selected_first_line =
                    state.log_total - snapshot.selected_logs.len() as u64 + 1;
                snapshot.selected_activity = state.activity.iter().cloned().collect();
                snapshot.selected_history = state.resource_history.iter().copied().collect();
            }

            if include_stack_logs {
                let start = state.logs.len().saturating_sub(STACK_LOG_LINES_PER_PROCESS);
                snapshot.stack_logs.extend(
                    state.logs[start..]
                        .iter()
                        .map(|line| (id.clone(), line.clone())),
                );
            }
        }

        // Stable sort: lines with the same timestamp keep their order within a process.
        snapshot.stack_logs.sort_by_key(|(_, line)| line.at);
        snapshot
    }

    /// Check and update docker container status
    #[allow(dead_code)]
    pub fn refresh_docker_status(&self, id: &str) {
        refresh_docker_status_inner(
            id,
            &self.processes,
            &self.log_directory,
            &self.event_tx,
            &self.event_version,
            &self.error_version,
            &self.process_error_versions,
        );
    }
}

fn refresh_resource_usage(processes: &Arc<Mutex<HashMap<String, ProcessState>>>) -> bool {
    let now = Instant::now();
    let processor_count = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .max(1) as f64;
    let mut updated = false;
    let mut processes = processes.lock().unwrap();
    #[cfg(unix)]
    if processes.values().any(|state| {
        state.config.process_type == ProcessType::Process
            && state.status == ProcessStatus::Running
            && state.child.is_some()
    }) {
        crate::platform::refresh_process_resources();
    }

    for state in processes.values_mut() {
        if state.config.process_type != ProcessType::Process
            || state.status != ProcessStatus::Running
        {
            updated |= clear_resource_usage(state);
            continue;
        }

        let Some(pid) = state.child.as_ref().map(|child| child.id()) else {
            updated |= clear_resource_usage(state);
            continue;
        };

        let Some(sample) = sample_process_resources(state, pid) else {
            updated |= clear_resource_usage(state);
            continue;
        };

        let cpu_percent = match (state.resource_sample, sample.cpu_time_100ns) {
            (Some(previous), Some(cpu_time_100ns)) => {
                let elapsed_seconds = now
                    .saturating_duration_since(previous.measured_at)
                    .as_secs_f64();
                if elapsed_seconds > 0.0 {
                    let cpu_delta_seconds = cpu_time_100ns.saturating_sub(previous.cpu_time_100ns)
                        as f64
                        / CPU_TIME_UNITS_PER_SECOND;
                    Some(
                        ((cpu_delta_seconds / elapsed_seconds) / processor_count * 100.0)
                            .clamp(0.0, 100.0) as f32,
                    )
                } else {
                    state.resource_usage.cpu_percent
                }
            }
            _ => state.resource_usage.cpu_percent,
        };

        if let Some(cpu_time_100ns) = sample.cpu_time_100ns {
            state.resource_sample = Some(ResourceSample {
                measured_at: now,
                cpu_time_100ns,
            });
        }

        let new_usage = ProcessResourceUsage {
            cpu_percent,
            memory_bytes: sample.memory_bytes,
        };

        if resource_usage_changed(state.resource_usage, new_usage) {
            state.resource_usage = new_usage;
            updated = true;
        }
    }

    // One sample per process per tick, zero while it is not running, so every sparkline
    // shares the same time axis.
    for state in processes.values_mut() {
        state.resource_history.push_back(state.resource_usage);
        while state.resource_history.len() > RESOURCE_HISTORY_LIMIT {
            state.resource_history.pop_front();
        }
        updated |= state.status == ProcessStatus::Running;
    }

    updated
}

fn clear_resource_usage(state: &mut ProcessState) -> bool {
    let changed =
        state.resource_usage != ProcessResourceUsage::default() || state.resource_sample.is_some();
    state.resource_usage = ProcessResourceUsage::default();
    state.resource_sample = None;
    changed
}

fn resource_usage_changed(previous: ProcessResourceUsage, next: ProcessResourceUsage) -> bool {
    previous.memory_bytes != next.memory_bytes
        || match (previous.cpu_percent, next.cpu_percent) {
            (Some(previous), Some(next)) => (previous - next).abs() >= 0.05,
            (None, None) => false,
            _ => true,
        }
}

#[cfg(not(any(windows, unix)))]
fn sample_process_resources(
    _state: &ProcessState,
    _root_pid: u32,
) -> Option<ResourceCounterSample> {
    None
}

#[cfg(unix)]
fn sample_process_resources(_state: &ProcessState, root_pid: u32) -> Option<ResourceCounterSample> {
    let (cpu_time_100ns, memory_bytes) = crate::platform::process_group_resources(root_pid)?;
    Some(ResourceCounterSample {
        cpu_time_100ns: Some(cpu_time_100ns),
        memory_bytes: Some(memory_bytes),
    })
}

#[cfg(windows)]
fn sample_process_resources(state: &ProcessState, root_pid: u32) -> Option<ResourceCounterSample> {
    let pids = state
        .job
        .as_ref()
        .and_then(|job| query_job_process_ids(job.handle))
        .filter(|pids| !pids.is_empty())
        .unwrap_or_else(|| vec![root_pid]);

    let cpu_time_100ns = state
        .job
        .as_ref()
        .and_then(|job| query_job_cpu_time(job.handle))
        .or_else(|| sum_process_cpu_time(&pids));
    let memory_bytes = sum_process_memory(&pids);

    if cpu_time_100ns.is_none() && memory_bytes.is_none() {
        None
    } else {
        Some(ResourceCounterSample {
            cpu_time_100ns,
            memory_bytes,
        })
    }
}

#[cfg(windows)]
fn query_job_cpu_time(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows_sys::Win32::System::JobObjects::{
        JobObjectBasicAccountingInformation, QueryInformationJobObject,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    };

    let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        QueryInformationJobObject(
            handle,
            JobObjectBasicAccountingInformation,
            &mut accounting as *mut _ as *mut _,
            std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };

    if ok == 0 {
        None
    } else {
        let total = accounting
            .TotalUserTime
            .saturating_add(accounting.TotalKernelTime);
        u64::try_from(total).ok()
    }
}

#[cfg(windows)]
fn query_job_process_ids(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<Vec<u32>> {
    use windows_sys::Win32::System::JobObjects::{
        JobObjectBasicProcessIdList, QueryInformationJobObject, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    };

    let mut capacity = 16usize;
    while capacity <= 4096 {
        let byte_len = std::mem::size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()
            + capacity
                .saturating_sub(1)
                .saturating_mul(std::mem::size_of::<usize>());
        let mut buffer = vec![0u8; byte_len];
        let ok = unsafe {
            QueryInformationJobObject(
                handle,
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr() as *mut _,
                byte_len as u32,
                std::ptr::null_mut(),
            )
        };

        if ok == 0 {
            capacity = capacity.saturating_mul(2);
            continue;
        }

        let list = unsafe { &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST) };
        let assigned_count = list.NumberOfAssignedProcesses as usize;
        let listed_count = list.NumberOfProcessIdsInList as usize;

        if assigned_count > listed_count && assigned_count > capacity {
            capacity = assigned_count.next_power_of_two().min(4096);
            continue;
        }

        let count = listed_count.min(assigned_count).min(capacity);
        let process_ids = unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count) };
        return Some(
            process_ids
                .iter()
                .filter_map(|pid| u32::try_from(*pid).ok())
                .collect(),
        );
    }

    None
}

#[cfg(windows)]
fn sum_process_cpu_time(pids: &[u32]) -> Option<u64> {
    let mut total = 0u64;
    let mut sampled = false;

    for &pid in pids {
        if let Some(handle) = open_process_for_metrics(pid) {
            if let Some(cpu_time) = process_cpu_time(handle.raw()) {
                total = total.saturating_add(cpu_time);
                sampled = true;
            }
        }
    }

    sampled.then_some(total)
}

#[cfg(windows)]
fn sum_process_memory(pids: &[u32]) -> Option<u64> {
    let mut total = 0u64;
    let mut sampled = false;

    for &pid in pids {
        if let Some(handle) = open_process_for_metrics(pid) {
            if let Some(bytes) = process_working_set_bytes(handle.raw()) {
                total = total.saturating_add(bytes);
                sampled = true;
            }
        }
    }

    sampled.then_some(total)
}

#[cfg(windows)]
fn process_cpu_time(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;

    let mut creation_time: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit_time: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel_time: FILETIME = unsafe { std::mem::zeroed() };
    let mut user_time: FILETIME = unsafe { std::mem::zeroed() };

    let ok = unsafe {
        GetProcessTimes(
            handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
    };

    if ok == 0 {
        None
    } else {
        Some(filetime_to_u64(kernel_time).saturating_add(filetime_to_u64(user_time)))
    }
}

#[cfg(windows)]
fn process_working_set_bytes(handle: windows_sys::Win32::Foundation::HANDLE) -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };

    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;

    let ok = unsafe { K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) };
    if ok == 0 {
        None
    } else {
        Some(counters.WorkingSetSize as u64)
    }
}

#[cfg(windows)]
fn filetime_to_u64(value: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

#[cfg(windows)]
struct ScopedProcessHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ScopedProcessHandle {
    fn raw(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.0
    }
}

#[cfg(windows)]
impl Drop for ScopedProcessHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn open_process_for_metrics(pid: u32) -> Option<ScopedProcessHandle> {
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    let handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        None
    } else {
        Some(ScopedProcessHandle(handle))
    }
}

fn append_runtime_log(
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    process_id: &str,
    line: String,
    is_stderr: bool,
) -> (bool, bool) {
    let (disk_log, formatted, has_error) = {
        let mut processes = processes.lock().unwrap();
        let Some(state) = processes.get_mut(process_id) else {
            return (false, false);
        };

        let sanitized = sanitize_runtime_log_line(&line);
        let formatted = if is_stderr {
            format!("[stderr] {}", sanitized)
        } else {
            sanitized
        };
        let has_error = line_has_error(&formatted);
        let disk_log = state.disk_log.clone();
        push_in_memory_log(state, formatted.clone());
        (disk_log, formatted, has_error)
    };

    if let Some(file) = disk_log {
        write_disk_log_line(&file, &formatted);
    }

    (true, has_error)
}

fn log_process_state_event(state: &mut ProcessState, message: String) {
    if let Some(file) = state.disk_log.clone() {
        write_disk_log_line(&file, &message);
    }
    push_in_memory_log(state, message);
}

fn push_in_memory_log(state: &mut ProcessState, line: String) {
    push_log_line(state, line, None);
}

fn push_log_line(state: &mut ProcessState, line: String, spans: LogSpans) {
    state.logs.push(LogLine {
        text: line,
        at: chrono::Local::now(),
        spans,
    });
    state.log_total += 1;
    if state.logs.len() > IN_MEMORY_LOG_LIMIT {
        state.logs.remove(0);
    }
}

fn exit_text(exit: ExitSummary) -> String {
    match exit.code {
        Some(code) => format!("Exited with code {code}"),
        None if exit.success => "Exited".to_string(),
        None => "Ended by a signal".to_string(),
    }
}

fn write_disk_log_line(file: &SharedLogFile, line: &str) {
    if let Ok(mut file) = file.lock() {
        let _ = writeln!(file, "{}", line);
        let _ = file.flush();
    }
}

fn sanitize_runtime_log_line(line: &str) -> String {
    let mut sanitized = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut prev = '\0';
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' || (prev == '\u{1b}' && next == '\\') {
                            break;
                        }
                        prev = next;
                    }
                }
                _ => {}
            }
            continue;
        }

        if matches!(ch, '\0'..='\u{08}' | '\u{0b}'..='\u{1a}' | '\u{1c}'..='\u{1f}' | '\u{7f}') {
            continue;
        }

        sanitized.push(ch);
    }

    sanitized
}

fn create_disk_log_session(
    base_directory: &str,
    config: &ProcessConfig,
) -> Result<SharedLogFile, String> {
    let process_directory = resolve_log_root(base_directory)
        .join(PROCESS_LOG_FOLDER_NAME)
        .join(sanitize_path_component(if config.name.trim().is_empty() {
            &config.id
        } else {
            &config.name
        }));

    fs::create_dir_all(&process_directory)
        .map_err(|err| format!("Failed to create log folder: {}", err))?;

    let session_path = next_log_file_path(&process_directory);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&session_path)
        .map_err(|err| format!("Failed to open session log: {}", err))?;

    rotate_process_logs(&process_directory, config.log_rotation_count.max(1))?;

    Ok(Arc::new(Mutex::new(file)))
}

fn rotate_process_logs(process_directory: &Path, keep_count: usize) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(process_directory)
        .map_err(|err| format!("Failed to read log folder: {}", err))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("log"))
        .collect();

    entries.sort();

    let remove_count = entries.len().saturating_sub(keep_count);
    for path in entries.into_iter().take(remove_count) {
        fs::remove_file(&path)
            .map_err(|err| format!("Failed to remove old log '{}': {}", path.display(), err))?;
    }

    Ok(())
}

fn next_log_file_path(process_directory: &Path) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let base = process_directory.join(format!("{}.log", timestamp));
    if !base.exists() {
        return base;
    }

    for suffix in 1..1000 {
        let candidate = process_directory.join(format!("{}_{}.log", timestamp, suffix));
        if !candidate.exists() {
            return candidate;
        }
    }

    process_directory.join(format!(
        "{}_{}.log",
        timestamp,
        chrono::Local::now().timestamp_millis()
    ))
}

fn resolve_log_root(directory: &str) -> PathBuf {
    let trimmed = directory.trim();
    let exe_dir = crate::platform::data_directory();

    if trimmed.is_empty() || trimmed == "." {
        return exe_dir;
    }

    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        path
    } else {
        exe_dir.join(path)
    }
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            _ => ch,
        })
        .collect();
    let trimmed = sanitized.trim().trim_matches('.');
    if trimmed.is_empty() {
        "Unnamed Process".to_string()
    } else {
        trimmed.to_string()
    }
}

fn process_snapshot_from_state(state: &ProcessState) -> ProcessRuntimeSnapshot {
    let (status, status_detail) = status_parts(&state.status);
    let pid = state.child.as_ref().map(|child| child.id());

    ProcessRuntimeSnapshot {
        id: state.config.id.clone(),
        name: state.config.name.clone(),
        process_type: state.config.process_type.to_string(),
        status,
        status_detail,
        pid,
        cpu_percent: state.resource_usage.cpu_percent,
        memory_bytes: state.resource_usage.memory_bytes,
        auto_start: state.config.auto_start,
        startup_delay_seconds: state.config.startup_delay_seconds,
        auto_restart: state.config.auto_restart,
        respond_to_start_all: state.config.respond_to_start_all,
        respond_to_stop_all: state.config.respond_to_stop_all,
        respond_to_restart_all: state.config.respond_to_restart_all,
        command: state.config.command.clone(),
        working_directory: state.config.working_directory.clone(),
    }
}

fn status_parts(status: &ProcessStatus) -> (String, Option<String>) {
    match status {
        ProcessStatus::Stopped => ("Stopped".to_string(), None),
        ProcessStatus::Running => ("Running".to_string(), None),
        ProcessStatus::Starting => ("Starting".to_string(), None),
        ProcessStatus::Stopping => ("Stopping".to_string(), None),
        ProcessStatus::Error(message) => ("Error".to_string(), Some(message.clone())),
    }
}

fn bump_event(event_tx: &watch::Sender<u64>, event_version: &Arc<AtomicU64>) {
    let next = event_version
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    let _ = event_tx.send(next);
}

fn bump_error(
    error_version: &Arc<AtomicU64>,
    process_error_versions: &Arc<Mutex<HashMap<String, u64>>>,
    process_id: &str,
) {
    let _ = error_version
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    let mut versions = process_error_versions.lock().unwrap();
    let entry = versions.entry(process_id.to_string()).or_insert(0);
    *entry = entry.wrapping_add(1);
}

fn launch_process(
    id: &str,
    config: &ProcessConfig,
    start_generation: u64,
    processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    log_directory: Arc<Mutex<String>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
    error_version: Arc<AtomicU64>,
    process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
) {
    match config.process_type {
        ProcessType::Process => ProcessManager::start_system_process(
            id,
            config,
            start_generation,
            processes,
            log_directory,
            event_tx,
            event_version,
            error_version,
            process_error_versions,
        ),
        ProcessType::Docker => ProcessManager::start_docker_container(
            id,
            config,
            start_generation,
            processes,
            log_directory,
            event_tx,
            event_version,
            error_version,
            process_error_versions,
        ),
    }
}

fn wait_for_startup_delay(
    id: &str,
    config: &ProcessConfig,
    start_generation: u64,
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    event_tx: &watch::Sender<u64>,
    event_version: &Arc<AtomicU64>,
) -> bool {
    if !start_request_is_current(id, start_generation, processes) {
        return false;
    }

    let delay = config.startup_delay_seconds;
    if delay == 0 {
        return true;
    }

    {
        let mut processes = processes.lock().unwrap();
        let Some(state) = processes.get_mut(id) else {
            return false;
        };
        if !start_request_is_current_state(state, start_generation) {
            return false;
        }
        log_process_state_event(
            state,
            format!("[Startup delay: waiting {} second(s)]", delay),
        );
    }
    bump_event(event_tx, event_version);

    for _ in 0..delay {
        thread::sleep(Duration::from_secs(1));
        if !start_request_is_current(id, start_generation, processes) {
            return false;
        }
    }

    true
}

fn start_request_is_current(
    id: &str,
    start_generation: u64,
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
) -> bool {
    let processes = processes.lock().unwrap();
    processes
        .get(id)
        .is_some_and(|state| start_request_is_current_state(state, start_generation))
}

fn start_request_is_current_state(state: &ProcessState, start_generation: u64) -> bool {
    state.start_generation == start_generation
        && state.child.is_none()
        && matches!(state.status, ProcessStatus::Starting)
}

fn stop_process_inner(
    id: &str,
    cause: &str,
    processes_arc: Arc<Mutex<HashMap<String, ProcessState>>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
) {
    let id_owned = id.to_string();

    let mut child_to_kill: Option<ManagedChild> = None;
    let mut terminal_to_finish: Option<SharedTerminal> = None;
    #[cfg(windows)]
    let mut job_to_close: Option<JobHandle> = None;
    let mut docker_container: Option<String> = None;
    let mut docker_was_live = false;

    {
        let mut processes = processes_arc.lock().unwrap();
        if let Some(state) = processes.get_mut(id) {
            state.suppress_restart_once = true;
            state.start_generation = state.start_generation.wrapping_add(1);
            state.stop_cause = cause.to_string();
            if state.restart_pending {
                state.restart_pending = false;
                state.record(ActivityKind::Stopped, "Managed restart cancelled", cause);
            }
            match state.config.process_type {
                ProcessType::Process => {
                    if let Some(child) = state.child.take() {
                        state.status = ProcessStatus::Stopping;
                        child_to_kill = Some(child);
                        terminal_to_finish = state.terminal.clone();
                        #[cfg(windows)]
                        {
                            job_to_close = state.job.take();
                        }
                    } else {
                        if state.status == ProcessStatus::Starting {
                            state.record(ActivityKind::Stopped, "Start cancelled", cause);
                        }
                        state.status = ProcessStatus::Stopped;
                        #[cfg(windows)]
                        {
                            state.job = None;
                        }
                        let _ = clear_resource_usage(state);
                    }
                }
                ProcessType::Docker => {
                    docker_was_live = matches!(
                        state.status,
                        ProcessStatus::Running | ProcessStatus::Starting
                    );
                    state.status = ProcessStatus::Stopping;
                    docker_container = Some(state.config.command.clone());
                }
            }
        } else {
            return;
        }
    }

    bump_event(&event_tx, &event_version);

    if let Some(mut child) = child_to_kill {
        thread::spawn(move || {
            #[cfg(windows)]
            let pid = child.id();
            let mut stop_error: Option<String> = None;

            #[cfg(windows)]
            {
                // Kill from the managed root while it still exists.  Releasing a
                // kill-on-close Job Object first can terminate the wrapper (for
                // example, PowerShell) before `taskkill /T` gets to walk its
                // descendants, leaving a child that did not inherit the job
                // orphaned.  The job is still closed immediately afterward as a
                // second cleanup mechanism for every process that did inherit it.
                if let Err(e) =
                    kill_tree_before_closing_job(|| kill_process_tree(pid), || drop(job_to_close))
                {
                    stop_error = Some(e);
                    let _ = child.kill();
                }
            }
            #[cfg(unix)]
            {
                if let Err(e) = child.stop_group() {
                    stop_error = Some(e.to_string());
                    let _ = child.kill();
                }
            }
            #[cfg(not(any(windows, unix)))]
            {
                if let Err(e) = child.kill() {
                    stop_error = Some(e.to_string());
                }
            }

            child.wait();
            if let Some(terminal) = &terminal_to_finish {
                finish_terminal(terminal, &processes_arc, &id_owned);
            }

            let mut processes = processes_arc.lock().unwrap();
            if let Some(state) = processes.get_mut(&id_owned) {
                state.child = None;
                if let Some(err) = stop_error {
                    log_process_state_event(state, format!("[Stop error: {}]", err));
                }
                log_process_state_event(state, "[Process stopped]".to_string());
                state.mark_stopped();
                state.status = ProcessStatus::Stopped;
                state.disk_log = None;
                let _ = clear_resource_usage(state);
            }
            bump_event(&event_tx, &event_version);
        });
        return;
    }

    if let Some(container_name) = docker_container {
        thread::spawn(move || {
            let mut cmd = Command::new("docker");
            cmd.args(["stop", &container_name]);

            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }

            let output = cmd.output();

            let mut processes = processes_arc.lock().unwrap();
            if let Some(state) = processes.get_mut(&id_owned) {
                match output {
                    Ok(out) if out.status.success() => {
                        state.status = ProcessStatus::Stopped;
                        log_process_state_event(
                            state,
                            format!("[Docker container '{}' stopped]", container_name),
                        );
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        log_process_state_event(state, format!("[Stop error: {}]", stderr));
                        state.status = ProcessStatus::Stopped;
                    }
                    Err(e) => {
                        log_process_state_event(state, format!("[Stop error: {}]", e));
                        state.status = ProcessStatus::Stopped;
                    }
                }
                if docker_was_live {
                    state.mark_stopped();
                }
                state.disk_log = None;
                let _ = clear_resource_usage(state);
            }
            bump_event(&event_tx, &event_version);
        });
    }
}

enum SchedulerAction {
    Start {
        id: String,
        config: ProcessConfig,
        start_generation: u64,
    },
    Stop {
        id: String,
        cause: &'static str,
    },
}

fn run_scheduler_tick(
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    log_directory: &Arc<Mutex<String>>,
    event_tx: &watch::Sender<u64>,
    event_version: &Arc<AtomicU64>,
    error_version: &Arc<AtomicU64>,
    process_error_versions: &Arc<Mutex<HashMap<String, u64>>>,
    schedule_state: &Arc<Mutex<HashMap<String, ProcessScheduleState>>>,
) {
    let now = chrono::Local::now();
    let day_index = now.weekday().num_days_from_monday() as usize;
    let hour = now.hour();
    let minute = now.minute();
    let minute_key = now.timestamp() / 60;
    let mut actions = Vec::new();
    let mut updated = false;

    {
        let mut processes_guard = processes.lock().unwrap();
        let mut schedule_guard = schedule_state.lock().unwrap();

        schedule_guard.retain(|id, _| processes_guard.contains_key(id));

        for (id, state) in processes_guard.iter_mut() {
            let schedule_runtime = schedule_guard.entry(id.clone()).or_default();

            if state.config.auto_restart && state.config.restart_schedule.enabled {
                let active = state.config.restart_schedule.active_at(day_index, hour);
                let previous_active = schedule_runtime.last_restart_schedule_active;

                if active && previous_active != Some(true) && process_is_dormant(state) {
                    state.begin_start("Active hours began");
                    log_process_state_event(
                        state,
                        "[Managed restart schedule became active. Starting...]".to_string(),
                    );
                    actions.push(SchedulerAction::Start {
                        id: id.clone(),
                        config: state.config.clone(),
                        start_generation: state.start_generation,
                    });
                    updated = true;
                } else if !active
                    && previous_active == Some(true)
                    && state.config.restart_schedule.stop_when_inactive
                    && !process_is_dormant(state)
                {
                    log_process_state_event(
                        state,
                        "[Managed restart schedule became inactive. Stopping...]".to_string(),
                    );
                    actions.push(SchedulerAction::Stop {
                        id: id.clone(),
                        cause: "Active hours ended",
                    });
                    updated = true;
                }

                schedule_runtime.last_restart_schedule_active = Some(active);
            } else {
                schedule_runtime.last_restart_schedule_active = None;
            }

            if state.config.scheduled_run.due_at(day_index, hour, minute)
                && schedule_runtime.last_scheduled_run_minute != Some(minute_key)
            {
                schedule_runtime.last_scheduled_run_minute = Some(minute_key);
                if process_is_dormant(state) {
                    state.begin_start("Schedule");
                    log_process_state_event(
                        state,
                        "[Scheduled run triggered. Starting...]".to_string(),
                    );
                    actions.push(SchedulerAction::Start {
                        id: id.clone(),
                        config: state.config.clone(),
                        start_generation: state.start_generation,
                    });
                    updated = true;
                }
            }
        }
    }

    if updated {
        bump_event(event_tx, event_version);
    }

    for action in actions {
        match action {
            SchedulerAction::Start {
                id,
                config,
                start_generation,
            } => launch_process(
                &id,
                &config,
                start_generation,
                processes.clone(),
                log_directory.clone(),
                event_tx.clone(),
                event_version.clone(),
                error_version.clone(),
                process_error_versions.clone(),
            ),
            SchedulerAction::Stop { id, cause } => stop_process_inner(
                &id,
                cause,
                processes.clone(),
                event_tx.clone(),
                event_version.clone(),
            ),
        }
    }
}

fn process_is_dormant(state: &ProcessState) -> bool {
    state.child.is_none()
        && matches!(
            state.status,
            ProcessStatus::Stopped | ProcessStatus::Error(_)
        )
}

fn managed_restart_active_now(config: &ProcessConfig) -> bool {
    let now = chrono::Local::now();
    config
        .restart_schedule
        .active_at(now.weekday().num_days_from_monday() as usize, now.hour())
}

fn schedule_managed_restart(
    id: String,
    processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    log_directory: Arc<Mutex<String>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
    error_version: Arc<AtomicU64>,
    process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
) {
    // Reuse the same attention signal that error events use.
    bump_error(&error_version, &process_error_versions, &id);

    thread::spawn(move || {
        thread::sleep(std::time::Duration::from_secs(1));

        let (config, start_generation) = {
            let mut processes = processes.lock().unwrap();
            let Some(state) = processes.get_mut(&id) else {
                return;
            };

            state.restart_pending = false;
            if state.suppress_restart_once {
                state.suppress_restart_once = false;
                return;
            }

            if !state.config.auto_restart
                || !managed_restart_active_now(&state.config)
                || state.child.is_some()
                || matches!(
                    state.status,
                    ProcessStatus::Running | ProcessStatus::Starting | ProcessStatus::Stopping
                )
            {
                return;
            }

            let cause = format!("Managed restart #{}", state.start_count.max(1));
            let start_generation = state.begin_start(cause);
            (state.config.clone(), start_generation)
        };

        bump_event(&event_tx, &event_version);

        launch_process(
            &id,
            &config,
            start_generation,
            processes.clone(),
            log_directory.clone(),
            event_tx.clone(),
            event_version.clone(),
            error_version.clone(),
            process_error_versions.clone(),
        );
    });
}

fn line_has_error(line: &str) -> bool {
    let trimmed = line.trim();
    let content = if let Some(rest) = trimmed.strip_prefix("[stderr]") {
        rest.trim_start()
    } else {
        trimmed
    };
    contains_error_indicator(content)
}

#[cfg(windows)]
pub(crate) struct JobHandle {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
unsafe impl Send for JobHandle {}

#[cfg(windows)]
unsafe impl Sync for JobHandle {}

#[cfg(windows)]
fn create_job() -> Result<JobHandle, String> {
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    unsafe {
        let handle = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if result == 0 {
            windows_sys::Win32::Foundation::CloseHandle(handle);
            return Err(std::io::Error::last_os_error().to_string());
        }

        Ok(JobHandle { handle })
    }
}

#[cfg(windows)]
fn assign_job(job: &JobHandle, child: &ManagedChild) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    let handle = match child {
        ManagedChild::Piped(child) => child.as_raw_handle(),
        ManagedChild::Terminal { child, .. } => child
            .as_raw_handle()
            .ok_or_else(|| "The terminal did not give a process handle".to_string())?,
    };
    let result = unsafe { AssignProcessToJobObject(job.handle, handle) };
    if result == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn build_command(
    program: &str,
    args: &[String],
    _working_directory: &str,
) -> Result<(Command, String), String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    Ok((cmd, program.to_string()))
}

/// The program and arguments to run, as `build_command` resolves them, and a label for the log.
#[cfg(not(windows))]
fn command_parts(
    program: &str,
    args: &[String],
    _working_directory: &str,
) -> Result<(String, Vec<String>, String), String> {
    Ok((program.to_string(), args.to_vec(), program.to_string()))
}

#[cfg(windows)]
fn command_parts(
    program: &str,
    args: &[String],
    working_directory: &str,
) -> Result<(String, Vec<String>, String), String> {
    let resolved = resolve_program(program, working_directory)?;
    if resolved.is_cmd_script {
        let mut all = vec!["/C".to_string(), resolved.path.clone()];
        all.extend(args.iter().cloned());
        Ok(("cmd".to_string(), all, format!("cmd /C {}", resolved.path)))
    } else {
        Ok((resolved.path.clone(), args.to_vec(), resolved.path))
    }
}

/// The size new terminals start at, in cells: the console's log area, as the UI last measured it.
static TERMINAL_SIZE: Mutex<(usize, usize)> = Mutex::new((120, 32));

/// Called by the UI with the size of its log area, so programs format for what is visible.
pub fn set_terminal_size(cols: usize, rows: usize) {
    *TERMINAL_SIZE.lock().unwrap() = (cols.clamp(40, 500), rows.clamp(10, 300));
}

/// Spawns a command in a pseudo-terminal. Returns the child, the terminal's master end, and the
/// reader and writer for its output and input.
fn spawn_in_terminal(
    program: &str,
    args: &[String],
    working_directory: &str,
    (cols, rows): (usize, usize),
) -> Result<
    (
        ManagedChild,
        Box<dyn portable_pty::MasterPty + Send>,
        Box<dyn std::io::Read + Send>,
        Box<dyn Write + Send>,
    ),
    String,
> {
    let pair = portable_pty::native_pty_system()
        .openpty(portable_pty::PtySize {
            rows: rows as u16,
            cols: cols as u16,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("Could not open a terminal: {e}"))?;
    let mut cmd = portable_pty::CommandBuilder::new(program);
    cmd.args(args);
    // The same environment a piped process gets (portable-pty would otherwise mix in the
    // registry's), plus what tells programs they have a capable terminal.
    cmd.env_clear();
    for (key, value) in std::env::vars_os() {
        cmd.env(key, value);
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let cwd = if working_directory.is_empty() {
        std::env::current_dir().map_err(|e| e.to_string())?
    } else {
        PathBuf::from(working_directory)
    };
    cmd.cwd(cwd);
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let pid = child.process_id().unwrap_or(0);
    let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    Ok((
        ManagedChild::Terminal { child, pid },
        pair.master,
        reader,
        writer,
    ))
}

/// A terminal's lines going into a process's logs.
struct StateSink<'a> {
    state: &'a mut ProcessState,
    error: bool,
}

impl LogSink for StateSink<'_> {
    fn push(&mut self, text: String, spans: LogSpans) -> u64 {
        push_log_line(self.state, text, spans);
        self.state.log_total
    }

    fn amend(&mut self, number: u64, text: String, spans: LogSpans) {
        let back = self.state.log_total.saturating_sub(number) as usize;
        let len = self.state.logs.len();
        if back < len {
            let line = &mut self.state.logs[len - 1 - back];
            line.text = text;
            line.spans = spans;
        }
    }

    fn settle(&mut self, text: &str) {
        if let Some(file) = &self.state.disk_log {
            write_disk_log_line(file, text);
        }
        self.error |= line_has_error(text);
    }
}

/// Brings a process's log up to date with its terminal; `Some(had_error)` when it did. The
/// caller holds the terminal's lock, so the lock order is always terminal, then processes.
fn sync_terminal(
    session: &mut TerminalSession,
    terminal: &SharedTerminal,
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    id: &str,
    finish: bool,
) -> Option<bool> {
    let mut processes = processes.lock().unwrap();
    let state = processes.get_mut(id)?;
    // A newer start has its own terminal; this one no longer writes to the log.
    if !state
        .terminal
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, terminal))
    {
        return None;
    }
    let mut sink = StateSink {
        state,
        error: false,
    };
    session.sync(&mut sink, Instant::now(), finish);
    let StateSink { state, error } = sink;
    state.terminal_alt = session.alt_active() && !finish;
    Some(error)
}

/// Ends a process's terminal: closes it, waits for its last output, and makes every row final.
fn finish_terminal(
    terminal: &SharedTerminal,
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    id: &str,
) -> bool {
    let detached = terminal.lock().unwrap().detach();
    detached.close(TERMINAL_DRAIN);
    let mut session = terminal.lock().unwrap();
    sync_terminal(&mut session, terminal, processes, id, true).unwrap_or(false)
}

/// Reads a terminal's output into its emulator and the process's log until the terminal closes.
#[allow(clippy::too_many_arguments)]
fn pump_terminal(
    id: String,
    terminal: SharedTerminal,
    mut reader: Box<dyn std::io::Read + Send>,
    done: std::sync::mpsc::Sender<()>,
    processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
    error_version: Arc<AtomicU64>,
    process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
) {
    thread::spawn(move || {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let read = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let synced = {
                let mut session = terminal.lock().unwrap();
                session.feed(&buffer[..read]);
                sync_terminal(&mut session, &terminal, &processes, &id, false)
            };
            if let Some(had_error) = synced {
                if had_error {
                    bump_error(&error_version, &process_error_versions, &id);
                }
                bump_event(&event_tx, &event_version);
            }
        }
        let _ = done.send(());
    });
}

/// Where a started process's output comes from.
enum Output {
    Piped {
        stdout: Option<std::process::ChildStdout>,
        stderr: Option<std::process::ChildStderr>,
    },
    Terminal {
        reader: Box<dyn std::io::Read + Send>,
        terminal: SharedTerminal,
        done: std::sync::mpsc::Sender<()>,
    },
}

impl Output {
    fn terminal(&self) -> Option<SharedTerminal> {
        match self {
            Output::Piped { .. } => None,
            Output::Terminal { terminal, .. } => Some(terminal.clone()),
        }
    }

    /// Lets go of a start that was cancelled before it was published.
    fn discard(self) {
        if let Output::Terminal {
            reader, terminal, ..
        } = self
        {
            // Close our end of the output first, so closing the terminal cannot wait on it.
            drop(reader);
            let detached = terminal.lock().unwrap().detach();
            detached.close(Duration::ZERO);
        }
    }
}

fn spawn_piped(
    program: &str,
    args: &[String],
    working_dir: &str,
) -> Result<(ManagedChild, Output, String), String> {
    // Build command (direct spawn; on Windows, .cmd/.bat are routed through cmd)
    let (mut cmd, program_label) = build_command(program, args, working_dir)?;

    if !working_dir.is_empty() {
        println!("[DEBUG] Setting current_dir to: {}", working_dir);
        cmd.current_dir(working_dir);
    }

    // Explicitly inherit the parent process's environment variables.
    // This is critical on Windows when using CREATE_NO_WINDOW, as the
    // spawned process may otherwise receive an incomplete PATH that
    // doesn't include user-specific directories (e.g., where npm lives).
    cmd.envs(std::env::vars());

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Hide console window on Windows
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    println!("[DEBUG] About to spawn command...");
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let output = Output::Piped {
        stdout: child.stdout.take(),
        stderr: child.stderr.take(),
    };
    Ok((ManagedChild::Piped(child), output, program_label))
}

fn spawn_terminal(
    program: &str,
    args: &[String],
    working_dir: &str,
) -> Result<(ManagedChild, Output, String), String> {
    let (program, args, program_label) = command_parts(program, args, working_dir)?;
    let size = *TERMINAL_SIZE.lock().unwrap();
    let (child, master, reader, writer) = spawn_in_terminal(&program, &args, working_dir, size)?;
    let (done, reader_done) = std::sync::mpsc::channel();
    let input = crate::terminal::spawn_input_writer(writer);
    let terminal = Arc::new(Mutex::new(TerminalSession::new(
        size.0,
        size.1,
        master,
        input,
        reader_done,
    )));
    let output = Output::Terminal {
        reader,
        terminal,
        done,
    };
    Ok((child, output, program_label))
}

fn fail_start(
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    id: &str,
    error: &str,
    error_version: &Arc<AtomicU64>,
    process_error_versions: &Arc<Mutex<HashMap<String, u64>>>,
    event_tx: &watch::Sender<u64>,
    event_version: &Arc<AtomicU64>,
) {
    {
        let mut processes = processes.lock().unwrap();
        if let Some(state) = processes.get_mut(id) {
            state.status = ProcessStatus::Error(error.to_string());
            let _ = clear_resource_usage(state);
            log_process_state_event(state, format!("[Failed to start: {}]", error));
            state.mark_failed(error);
        }
    }
    bump_error(error_version, process_error_versions, id);
    bump_event(event_tx, event_version);
}

/// Reads one of a piped process's streams into its log, a line at a time.
#[allow(clippy::too_many_arguments)]
fn pump_pipe(
    reader: Box<dyn std::io::Read + Send>,
    is_stderr: bool,
    id: String,
    processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
    error_version: Arc<AtomicU64>,
    process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
) {
    let stream = if is_stderr { "stderr" } else { "stdout" };
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            let read = match reader.read_until(b'\n', &mut buffer) {
                Ok(read) => read,
                Err(err) => {
                    let (updated, has_error) = append_runtime_log(
                        &processes,
                        &id,
                        format!("[{stream} reader error: {}]", err),
                        true,
                    );
                    if updated {
                        if has_error {
                            bump_error(&error_version, &process_error_versions, &id);
                        }
                        bump_event(&event_tx, &event_version);
                    }
                    break;
                }
            };
            if read == 0 {
                break;
            }
            while matches!(buffer.last(), Some(b'\n' | b'\r')) {
                buffer.pop();
            }
            let line = String::from_utf8_lossy(&buffer).into_owned();
            if line.is_empty() {
                continue;
            }
            let (updated, has_error) = append_runtime_log(&processes, &id, line, is_stderr);
            if updated {
                if has_error {
                    bump_error(&error_version, &process_error_versions, &id);
                }
                bump_event(&event_tx, &event_version);
            }
        }
    });
}

/// Watches a started process until it exits: records the exit, schedules a managed restart, and
/// keeps a terminal's rows settling while the program is quiet.
#[allow(clippy::too_many_arguments)]
fn monitor_child(
    id: String,
    start_generation: u64,
    processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    log_directory: Arc<Mutex<String>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
    error_version: Arc<AtomicU64>,
    process_error_versions: Arc<Mutex<HashMap<String, u64>>>,
) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(500));
        let terminal = {
            let processes = processes.lock().unwrap();
            match processes.get(&id) {
                Some(state) if state.start_generation == start_generation => state.terminal.clone(),
                _ => break,
            }
        };
        if let Some(terminal) = &terminal {
            let mut session = terminal.lock().unwrap();
            if session.is_live()
                && sync_terminal(&mut session, terminal, &processes, &id, false) == Some(true)
            {
                bump_error(&error_version, &process_error_versions, &id);
            }
        }

        let ended = {
            let mut processes = processes.lock().unwrap();
            let Some(state) = processes.get_mut(&id) else {
                break;
            };
            if state.start_generation != start_generation {
                break;
            }
            let Some(child) = state.child.as_mut() else {
                break;
            };
            #[cfg(unix)]
            let process_group = child.id();
            match child.try_wait() {
                Ok(None) => continue,
                ended => {
                    #[cfg(unix)]
                    let _ = crate::platform::kill_process_group(process_group);
                    ended
                }
            }
        };

        // A terminal hands over its last output before the exit is recorded.
        let mut had_error = match &terminal {
            Some(terminal) => finish_terminal(terminal, &processes, &id),
            None => false,
        };
        let mut should_schedule_restart = false;
        {
            let mut processes = processes.lock().unwrap();
            let Some(state) = processes.get_mut(&id) else {
                break;
            };
            // Stopped meanwhile: the stop records it.
            if state.start_generation != start_generation {
                break;
            }
            match ended {
                Ok(Some(status)) => {
                    log_process_state_event(
                        state,
                        format!("[Process exited with: {}]", status.text),
                    );
                    let exit = ExitSummary {
                        code: status.code,
                        success: status.success,
                    };
                    state.last_exit = Some(exit);
                    state.running_since = None;
                    state.record(ActivityKind::Exited, exit_text(exit), "");
                    if state.config.auto_restart
                        && managed_restart_active_now(&state.config)
                        && !state.suppress_restart_once
                    {
                        log_process_state_event(
                            state,
                            "[Managed process went down. Restarting...]".to_string(),
                        );
                        state.restart_pending = true;
                        should_schedule_restart = true;
                    }
                    state.status = ProcessStatus::Stopped;
                    state.suppress_restart_once = false;
                }
                Ok(None) => {}
                Err(e) => {
                    state.status = ProcessStatus::Error(e.to_string());
                    state.running_since = None;
                    state.record(
                        ActivityKind::Failed,
                        format!("Lost track of the process: {e}"),
                        "",
                    );
                    had_error = true;
                }
            }
            state.child = None;
            state.disk_log = None;
            let _ = clear_resource_usage(state);
            #[cfg(windows)]
            {
                state.job = None;
            }
        }
        if had_error {
            bump_error(&error_version, &process_error_versions, &id);
        }
        bump_event(&event_tx, &event_version);
        if should_schedule_restart {
            schedule_managed_restart(
                id.clone(),
                processes.clone(),
                log_directory.clone(),
                event_tx.clone(),
                event_version.clone(),
                error_version.clone(),
                process_error_versions.clone(),
            );
        }
        break;
    });
}

#[cfg(windows)]
struct ResolvedProgram {
    path: String,
    is_cmd_script: bool,
}

#[cfg(windows)]
fn build_command(
    program: &str,
    args: &[String],
    working_directory: &str,
) -> Result<(Command, String), String> {
    let resolved = resolve_program(program, working_directory)?;
    if resolved.is_cmd_script {
        // Don't pre-quote! Let Rust's Command API handle argument quoting.
        // Previously we used build_cmdline() which quoted the path, then
        // cmd.args([\"/S\", \"/C\", &cmdline]) caused Rust to quote the whole
        // cmdline again, resulting in literal backslash-quote characters.
        let mut cmd = Command::new("cmd");
        cmd.arg("/C");
        cmd.arg(&resolved.path);
        cmd.args(args);
        Ok((cmd, format!("cmd /C {}", resolved.path)))
    } else {
        let mut cmd = Command::new(&resolved.path);
        cmd.args(args);
        Ok((cmd, resolved.path))
    }
}

#[cfg(windows)]
fn resolve_program(program: &str, working_directory: &str) -> Result<ResolvedProgram, String> {
    use std::env;
    use std::path::Path;

    let program = program.trim();
    if program.is_empty() {
        return Err("Command is empty".to_string());
    }

    let path = Path::new(program);
    let has_separator = program.contains('\\') || program.contains('/');
    let extensions = supported_extensions();

    if has_separator || path.is_absolute() {
        return resolve_with_extensions(path);
    }

    let working_directory = working_directory.trim();
    // Support pasted quoted working directories like "C:\\path with spaces".
    let working_directory = working_directory.trim_matches('"');
    if !working_directory.is_empty() {
        let working_directory = Path::new(working_directory);
        if let Some(resolved) = resolve_in_dir(&working_directory, program, &extensions) {
            return Ok(resolved);
        }
    }

    let path_env = env::var_os("PATH").unwrap_or_default();
    for dir in env::split_paths(&path_env) {
        if let Some(resolved) = resolve_in_dir(&dir, program, &extensions) {
            return Ok(resolved);
        }
    }

    Err(format!(
        "Program not found or not executable: {} (expected .exe/.com/.cmd/.bat on working directory or PATH)",
        program
    ))
}

#[cfg(windows)]
fn resolve_with_extensions(path: &std::path::Path) -> Result<ResolvedProgram, String> {
    if path.extension().and_then(|e| e.to_str()).is_some() {
        if let Some(resolved) = resolve_path_candidate(path) {
            return Ok(resolved);
        }
        return Err(format!(
            "Program not found or not executable: {}",
            path.display()
        ));
    }

    for ext in supported_extensions() {
        let candidate = path.with_extension(ext.trim_start_matches('.'));
        if let Some(resolved) = resolve_path_candidate(&candidate) {
            return Ok(resolved);
        }
    }

    Err(format!(
        "Program not found or not executable: {} (expected .exe/.com/.cmd/.bat)",
        path.display()
    ))
}

#[cfg(windows)]
fn resolve_in_dir(
    dir: &std::path::Path,
    program: &str,
    extensions: &[String],
) -> Option<ResolvedProgram> {
    let base = dir.join(program);
    if std::path::Path::new(program).extension().is_some() {
        return resolve_path_candidate(&base);
    }

    for ext in extensions {
        let trimmed = ext.trim_start_matches('.');
        let candidate = base.with_extension(trimmed);
        if let Some(resolved) = resolve_path_candidate(&candidate) {
            return Some(resolved);
        }
    }
    None
}

#[cfg(windows)]
fn resolve_path_candidate(path: &std::path::Path) -> Option<ResolvedProgram> {
    if path.exists() && path.is_file() {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !is_supported_extension(&ext) {
            return None;
        }
        let is_cmd_script = ext == "cmd" || ext == "bat";
        return Some(ResolvedProgram {
            path: path.to_string_lossy().to_string(),
            is_cmd_script,
        });
    }
    None
}

#[cfg(windows)]
fn supported_extensions() -> Vec<String> {
    vec![
        ".exe".to_string(),
        ".com".to_string(),
        ".cmd".to_string(),
        ".bat".to_string(),
    ]
}

#[cfg(windows)]
fn is_supported_extension(ext: &str) -> bool {
    matches!(ext, "exe" | "com" | "cmd" | "bat")
}

#[cfg(unix)]
fn parse_command(command: &str) -> Result<(String, Vec<String>), String> {
    // Shell-style argument quoting without interpreting or executing shell code.
    let mut quote = None;
    let mut escaped = false;
    for c in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if quote == Some(c) {
            quote = None;
            continue;
        }
        if quote.is_none() {
            match c {
                '\'' | '"' => quote = Some(c),
                '|' | '&' | '<' | '>' | ';' => return Err("Shell operators are not supported without a shell. Use a script or an explicit shell command.".to_string()),
                _ => {}
            }
        }
    }
    let mut args = shell_words::split(command).map_err(|e| e.to_string())?;
    if args.is_empty() || args[0].is_empty() {
        return Err("Command is empty".to_string());
    }
    let program = args.remove(0);
    Ok((program, args))
}

#[cfg(not(unix))]
fn parse_command(command: &str) -> Result<(String, Vec<String>), String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let trimmed = command.trim();

    if trimmed.is_empty() {
        return Err("Command is empty".to_string());
    }

    let mut chars = trimmed.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
            }
            // Reject shell operators when not quoted to avoid silent misbehavior.
            '|' | '&' | '<' | '>' if !in_quotes => {
                return Err("Shell operators are not supported without a shell. Use a script or remove operators.".to_string());
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    args.push(current);
                    current = String::new();
                }
            }
            '\\' => {
                if let Some('"') = chars.peek().copied() {
                    let _ = chars.next();
                    current.push('"');
                } else {
                    current.push('\\');
                }
            }
            _ => current.push(c),
        }
    }

    if in_quotes {
        return Err("Unclosed quote in command".to_string());
    }

    if !current.is_empty() {
        args.push(current);
    }

    if args.is_empty() {
        return Err("Command is empty".to_string());
    }

    let program = args.remove(0);
    Ok((program, args))
}

#[cfg(windows)]
fn kill_tree_before_closing_job<K, C>(kill_tree: K, close_job: C) -> Result<(), String>
where
    K: FnOnce() -> Result<(), String>,
    C: FnOnce(),
{
    let tree_result = kill_tree();
    close_job();
    tree_result
}

#[cfg(windows)]
fn kill_process_tree(pid: u32) -> Result<(), String> {
    let mut cmd = Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000);
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to invoke taskkill: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("taskkill failed: {}", stderr.trim()))
    }
}

fn refresh_docker_status_inner(
    id: &str,
    processes: &Arc<Mutex<HashMap<String, ProcessState>>>,
    log_directory: &Arc<Mutex<String>>,
    event_tx: &watch::Sender<u64>,
    event_version: &Arc<AtomicU64>,
    error_version: &Arc<AtomicU64>,
    process_error_versions: &Arc<Mutex<HashMap<String, u64>>>,
) {
    let container_name = {
        let processes = processes.lock().unwrap();
        if let Some(state) = processes.get(id) {
            if state.config.process_type == ProcessType::Docker {
                Some(state.config.command.clone())
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some(container_name) = container_name {
        let mut cmd = Command::new("docker");
        cmd.args(["inspect", "-f", "{{.State.Running}}", &container_name]);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }

        if let Ok(output) = cmd.output() {
            let is_running = String::from_utf8_lossy(&output.stdout)
                .trim()
                .eq_ignore_ascii_case("true");

            let mut updated = false;
            let mut should_schedule_restart = false;
            {
                let mut processes = processes.lock().unwrap();
                if let Some(state) = processes.get_mut(id) {
                    if is_running {
                        // A container being stopped still reports running until docker stop
                        // returns; that is not a start.
                        if !matches!(
                            state.status,
                            ProcessStatus::Running | ProcessStatus::Stopping
                        ) {
                            if state.start_cause.is_empty() {
                                state.start_cause = "Found running".to_string();
                            }
                            state.mark_running(None);
                            updated = true;
                        }
                    } else if state.config.auto_restart
                        && managed_restart_active_now(&state.config)
                        && !state.suppress_restart_once
                        && state.status == ProcessStatus::Running
                    {
                        log_process_state_event(
                            state,
                            "[Container stopped unexpectedly. Restarting...]".to_string(),
                        );
                        state.running_since = None;
                        state.record(ActivityKind::Exited, "Container stopped", "");
                        state.status = ProcessStatus::Stopped;
                        state.restart_pending = true;
                        should_schedule_restart = true;
                        updated = true;
                    } else {
                        if state.status == ProcessStatus::Running {
                            state.running_since = None;
                            state.record(ActivityKind::Exited, "Container stopped", "");
                        }
                        if state.status != ProcessStatus::Stopped {
                            state.status = ProcessStatus::Stopped;
                            updated = true;
                        }
                        if state.suppress_restart_once {
                            state.suppress_restart_once = false;
                        }
                    }

                    if is_running && state.suppress_restart_once {
                        state.suppress_restart_once = false;
                        updated = true;
                    }
                }
            }

            if updated {
                bump_event(event_tx, event_version);
            }
            if should_schedule_restart {
                schedule_managed_restart(
                    id.to_string(),
                    processes.clone(),
                    log_directory.clone(),
                    event_tx.clone(),
                    event_version.clone(),
                    error_version.clone(),
                    process_error_versions.clone(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{sanitize_runtime_log_line, ProcessManager, StackControlAction};
    use crate::config::{ProcessConfig, ProcessType};

    #[cfg(windows)]
    use super::kill_tree_before_closing_job;
    #[cfg(windows)]
    use std::cell::RefCell;

    #[cfg(windows)]
    fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }

    #[cfg(unix)]
    struct UnixFixture {
        manager: ProcessManager,
        id: String,
        directory: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl UnixFixture {
        fn new(script: &str, auto_restart: bool) -> Self {
            let directory = std::env::temp_dir().join(format!("pm-unix-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("worker.sh"), script).unwrap();
            let manager = ProcessManager::new();
            let mut config = ProcessConfig::new(
                "Unix regression".to_string(),
                "/bin/sh worker.sh".to_string(),
                directory.to_string_lossy().into_owned(),
                ProcessType::Process,
            );
            config.auto_restart = auto_restart;
            let id = config.id.clone();
            manager.add_process(config);
            manager.start_process(&id);
            Self {
                manager,
                id,
                directory,
            }
        }

        fn child_pid(&self) -> u32 {
            assert!(unix_wait_until(|| std::fs::read_to_string(
                self.directory.join("child.pid")
            )
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .is_some()));
            let content = std::fs::read_to_string(self.directory.join("child.pid")).unwrap();
            content.trim().parse().unwrap()
        }
    }

    #[cfg(unix)]
    impl Drop for UnixFixture {
        fn drop(&mut self) {
            self.manager.stop_non_docker();
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[cfg(unix)]
    fn unix_wait_until(mut condition: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while std::time::Instant::now() < deadline {
            if condition() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }

    #[cfg(unix)]
    fn unix_process_running(pid: u32) -> bool {
        let output = std::process::Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .output()
            .unwrap();
        let status = String::from_utf8_lossy(&output.stdout);
        output.status.success() && !status.trim().is_empty() && !status.trim().starts_with('Z')
    }

    #[cfg(unix)]
    #[test]
    fn unix_stop_and_app_shutdown_kill_stubborn_descendants() {
        for shutdown in [false, true] {
            let fixture = UnixFixture::new(
                "trap '' TERM\nsleep 60 &\necho $! > child.pid\nwait\n",
                false,
            );
            let child_pid = fixture.child_pid();
            assert!(unix_process_running(child_pid));
            assert!(unix_wait_until(
                || fixture.manager.get_status(&fixture.id) == Some(super::ProcessStatus::Running)
            ));
            let root_pid = fixture
                .manager
                .get_process_snapshot(&fixture.id)
                .unwrap()
                .pid
                .unwrap();
            if shutdown {
                fixture.manager.stop_non_docker();
            } else {
                fixture.manager.stop_process(&fixture.id);
            }
            assert!(unix_wait_until(
                || fixture.manager.get_status(&fixture.id) == Some(super::ProcessStatus::Stopped)
            ));
            assert!(unix_wait_until(|| !unix_process_running(child_pid)));
            assert!(!unix_process_running(root_pid));
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_parent_exit_cleans_up_background_children() {
        let fixture = UnixFixture::new("sleep 60 &\necho $! > child.pid\nexit 0\n", false);
        let child_pid = fixture.child_pid();
        assert!(unix_wait_until(
            || fixture.manager.get_status(&fixture.id) == Some(super::ProcessStatus::Stopped)
        ));
        assert!(unix_wait_until(|| !unix_process_running(child_pid)));
    }

    #[cfg(unix)]
    #[test]
    fn unix_resources_include_children_and_cpu_time() {
        let fixture = UnixFixture::new(
            "/bin/sh -c 'while :; do :; done' &\necho $! > child.pid\nwait\n",
            false,
        );
        let _ = fixture.child_pid();
        assert!(unix_wait_until(
            || fixture.manager.get_status(&fixture.id) == Some(super::ProcessStatus::Running)
        ));
        let root_pid = fixture
            .manager
            .get_process_snapshot(&fixture.id)
            .unwrap()
            .pid
            .unwrap();
        crate::platform::refresh_process_resources();
        let first = crate::platform::process_group_resources(root_pid).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        crate::platform::refresh_process_resources();
        let next = crate::platform::process_group_resources(root_pid).unwrap();
        assert!(next.1 > 0, "missing group memory");
        assert!(next.0 > first.0, "missing child CPU time");
        super::refresh_resource_usage(&fixture.manager.processes);
        std::thread::sleep(std::time::Duration::from_millis(300));
        super::refresh_resource_usage(&fixture.manager.processes);
        let snapshot = fixture.manager.get_process_snapshot(&fixture.id).unwrap();
        assert!(snapshot.cpu_percent.unwrap() > 0.0);
        assert!(snapshot.memory_bytes.unwrap() > 0);
    }

    #[cfg(unix)]
    #[test]
    fn unix_managed_restart_and_manual_cancellation() {
        for code in [0, 7] {
            let fixture =
                UnixFixture::new(&format!("echo launched >> launches\nexit {code}\n"), true);
            assert!(unix_wait_until(|| std::fs::read_to_string(
                fixture.directory.join("launches")
            )
            .unwrap_or_default()
            .lines()
            .count()
                >= 3));
            fixture.manager.stop_process(&fixture.id);
            let generation =
                fixture.manager.processes.lock().unwrap()[&fixture.id].start_generation;
            std::thread::sleep(std::time::Duration::from_millis(1600));
            let processes = fixture.manager.processes.lock().unwrap();
            let state = &processes[&fixture.id];
            assert_eq!(state.status, super::ProcessStatus::Stopped);
            assert!(state.child.is_none());
            assert_eq!(state.start_generation, generation);
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_command_quoting_supports_shell_wrappers_and_empty_arguments() {
        let (program, args) =
            super::parse_command("/bin/sh -c 'echo hello && echo world' \"\" 'two words'").unwrap();
        assert_eq!(program, "/bin/sh");
        assert_eq!(args, ["-c", "echo hello && echo world", "", "two words"]);
        assert!(super::parse_command("echo hello && echo world").is_err());
        assert!(super::parse_command("echo hello; echo world").is_err());
        assert!(super::parse_command("echo 'unclosed").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn terminal_processes_log_their_screen_and_keep_colour() {
        let script =
            std::env::temp_dir().join(format!("procman-term-{}.cmd", uuid::Uuid::new_v4()));
        // A coloured line, a "progress bar" redrawn with carriage returns, and a final line.
        std::fs::write(
            &script,
            "@echo off\r\necho \x1b[31mred line\x1b[0m\r\n<nul set /p=step 1\r\n\
             <nul set /p=\r\r\n<nul set /p=step 2 of 2\r\necho.\r\necho done\r\nexit /B 3\r\n",
        )
        .unwrap();
        let manager = ProcessManager::new();
        let mut config = ProcessConfig::new(
            "terminal".to_string(),
            format!("\"{}\"", script.display()),
            String::new(),
            ProcessType::Process,
        );
        config.terminal_mode = crate::config::TerminalMode::Auto;
        let id = config.id.clone();
        manager.add_process(config);
        manager.start_process(&id);
        let exited = wait_until(|| {
            manager
                .get_recent_logs(&id, 100)
                .unwrap_or_default()
                .iter()
                .any(|line| line.starts_with("[Process exited"))
        });
        let logs = manager.get_recent_logs(&id, 100).unwrap_or_default();
        let _ = std::fs::remove_file(&script);
        assert!(exited, "never exited: {logs:?}");
        assert!(logs.iter().any(|l| l == "red line"), "{logs:?}");
        assert!(logs.iter().any(|l| l == "done"), "{logs:?}");
        assert!(logs.contains(&"[Process exited with: exit code: 3]".to_string()));
        let red = manager.processes.lock().unwrap()[&id]
            .logs
            .iter()
            .find(|l| l.text == "red line")
            .and_then(|l| l.spans.clone());
        assert_eq!(
            red.as_deref().map(|s| s[0].style.fg),
            Some(crate::terminal::TermColor::Indexed(1))
        );
        assert!(manager
            .terminal(&id)
            .is_some_and(|t| !t.lock().unwrap().is_live()));
    }

    #[cfg(windows)]
    #[test]
    fn managed_restart_relaunches_after_clean_and_error_exits() {
        for exit_code in [0, 7] {
            let marker =
                std::env::temp_dir().join(format!("procman-restart-{}.txt", uuid::Uuid::new_v4()));
            let script = marker.with_extension("cmd");
            std::fs::write(
                &script,
                format!(
                    "@echo launched>>\"{}\"\r\n@exit /B {}\r\n",
                    marker.display(),
                    exit_code
                ),
            )
            .unwrap();
            let manager = ProcessManager::new();
            let mut config = ProcessConfig::new(
                "restart regression".to_string(),
                format!("\"{}\"", script.display()),
                String::new(),
                ProcessType::Process,
            );
            config.auto_restart = true;
            let id = config.id.clone();
            manager.add_process(config);
            manager.start_process(&id);
            let relaunched = wait_until(|| {
                std::fs::read_to_string(&marker)
                    .unwrap_or_default()
                    .lines()
                    .count()
                    >= 3
            });
            manager.stop_process(&id);
            let _ = std::fs::remove_file(&marker);
            let _ = std::fs::remove_file(&script);
            assert!(
                relaunched,
                "exit code {exit_code} did not repeatedly relaunch"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn manual_stop_cancels_pending_managed_restart() {
        let manager = ProcessManager::new();
        let mut config = ProcessConfig::new(
            "cancel restart regression".to_string(),
            "cmd.exe /D /C exit /B 0".to_string(),
            String::new(),
            ProcessType::Process,
        );
        config.auto_restart = true;
        let id = config.id.clone();
        manager.add_process(config);
        manager.start_process(&id);
        assert!(wait_until(|| {
            manager.processes.lock().unwrap()[&id]
                .logs
                .iter()
                .any(|line| line.text.contains("Restarting..."))
        }));
        manager.stop_process(&id);
        let generation = manager.processes.lock().unwrap()[&id].start_generation;
        std::thread::sleep(std::time::Duration::from_millis(1600));
        let processes = manager.processes.lock().unwrap();
        let state = &processes[&id];
        assert_eq!(state.status, super::ProcessStatus::Stopped);
        assert!(state.child.is_none());
        assert_eq!(state.start_generation, generation);
    }

    #[test]
    fn strips_ansi_csi_sequences() {
        let line = "\u{1b}[32mready in\u{1b}[39m \u{1b}[1m406\u{1b}[22m ms";
        assert_eq!(sanitize_runtime_log_line(line), "ready in 406 ms");
    }

    #[test]
    fn strips_ansi_osc_sequences() {
        let line = "\u{1b}]0;Process Manager\u{7}server started";
        assert_eq!(sanitize_runtime_log_line(line), "server started");
    }

    #[test]
    fn selected_stack_controls_use_the_matching_process_preference() {
        fn process(id: &str, start: bool, stop: bool, restart: bool) -> ProcessConfig {
            let mut process = ProcessConfig::new(
                id.to_string(),
                "cmd.exe".to_string(),
                String::new(),
                ProcessType::Process,
            );
            process.id = id.to_string();
            process.respond_to_start_all = start;
            process.respond_to_stop_all = stop;
            process.respond_to_restart_all = restart;
            process
        }

        let manager = ProcessManager::new();
        for process in [
            process("start", true, false, false),
            process("stop", false, true, false),
            process("restart", false, false, true),
            process("all", true, true, true),
        ] {
            manager.add_process(process);
        }
        let selected_ids = vec![
            "start".to_string(),
            "stop".to_string(),
            "restart".to_string(),
            "all".to_string(),
        ];

        assert_eq!(
            manager
                .process_ids_responding_to_stack_control(&selected_ids, StackControlAction::Start),
            vec!["start", "all"]
        );
        assert_eq!(
            manager
                .process_ids_responding_to_stack_control(&selected_ids, StackControlAction::Stop),
            vec!["stop", "all"]
        );
        assert_eq!(
            manager.process_ids_responding_to_stack_control(
                &selected_ids,
                StackControlAction::Restart
            ),
            vec!["restart", "all"]
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_stop_kills_the_tree_before_releasing_the_job() {
        let events = RefCell::new(Vec::new());

        let result = kill_tree_before_closing_job(
            || {
                events.borrow_mut().push("tree");
                Ok(())
            },
            || events.borrow_mut().push("job"),
        );

        assert!(result.is_ok());
        assert_eq!(*events.borrow(), vec!["tree", "job"]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_stop_still_releases_the_job_after_tree_kill_failure() {
        let events = RefCell::new(Vec::new());

        let result = kill_tree_before_closing_job(
            || {
                events.borrow_mut().push("tree");
                Err("taskkill failed".to_string())
            },
            || events.borrow_mut().push("job"),
        );

        assert_eq!(result.unwrap_err(), "taskkill failed");
        assert_eq!(*events.borrow(), vec!["tree", "job"]);
    }
}
