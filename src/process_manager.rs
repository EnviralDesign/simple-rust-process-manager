//! Process management logic for starting, stopping, and monitoring processes.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc,
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread;

use tokio::sync::watch;

use crate::config::{ProcessConfig, ProcessType};

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

/// Runtime state for a single process
pub struct ProcessState {
    pub config: ProcessConfig,
    pub status: ProcessStatus,
    pub logs: Vec<String>,
    pub child: Option<Child>,
    #[cfg(windows)]
    pub job: Option<JobHandle>,
}

impl ProcessState {
    pub fn new(config: ProcessConfig) -> Self {
        Self {
            config,
            status: ProcessStatus::Stopped,
            logs: Vec::new(),
            child: None,
            #[cfg(windows)]
            job: None,
        }
    }
}

/// Manages all running processes
pub struct ProcessManager {
    pub processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
    error_version: Arc<AtomicU64>,
    background_started: AtomicBool,
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
            event_tx,
            event_version: Arc::new(AtomicU64::new(0)),
            error_version: Arc::new(AtomicU64::new(0)),
            background_started: AtomicBool::new(false),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.event_tx.subscribe()
    }

    pub fn error_version(&self) -> u64 {
        self.error_version.load(Ordering::Relaxed)
    }

    fn notify(&self) {
        bump_event(&self.event_tx, &self.event_version);
    }

    pub fn start_background_tasks(&self) {
        if self.background_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let processes = self.processes.clone();
        let event_tx = self.event_tx.clone();
        let event_version = self.event_version.clone();

        thread::spawn(move || loop {
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
                refresh_docker_status_inner(&id, &processes, &event_tx, &event_version);
            }
        });
    }

    /// Initialize process states from config
    pub fn init_from_config(&self, configs: &[ProcessConfig]) {
        let mut processes = self.processes.lock().unwrap();
        for config in configs {
            if !processes.contains_key(&config.id) {
                processes.insert(config.id.clone(), ProcessState::new(config.clone()));
            }
        }
    }

    /// Add a new process
    pub fn add_process(&self, config: ProcessConfig) {
        let mut processes = self.processes.lock().unwrap();
        processes.insert(config.id.clone(), ProcessState::new(config));
        self.notify();
    }

    /// Remove a process (stops it first if running)
    pub fn remove_process(&self, id: &str) {
        self.stop_process(id);
        let mut processes = self.processes.lock().unwrap();
        processes.remove(id);
        self.notify();
    }

    /// Start a process
    pub fn start_process(&self, id: &str) {
        println!("[DEBUG] start_process called with id: {}", id);
        let processes_arc = self.processes.clone();
        let event_tx = self.event_tx.clone();
        let event_version = self.event_version.clone();
        let error_version = self.error_version.clone();
        
        // Get config and update status
        let config = {
            let mut processes = processes_arc.lock().unwrap();
            println!("[DEBUG] Got lock, looking for process id: {}", id);
            if let Some(state) = processes.get_mut(id) {
                println!("[DEBUG] Found process: {}, status: {:?}", state.config.name, state.status);
                if state.status == ProcessStatus::Running {
                    println!("[DEBUG] Already running, returning");
                    return; // Already running
                }
                state.status = ProcessStatus::Starting;
                state.logs.clear();
                bump_event(&event_tx, &event_version);
                state.config.clone()
            } else {
                println!("[DEBUG] Process not found in manager!");
                return;
            }
        };

        let id_owned = id.to_string();
        println!("[DEBUG] Starting process type: {:?}, command: {}", config.process_type, config.command);

        match config.process_type {
            ProcessType::Process => {
                self.start_system_process(
                    &id_owned,
                    &config,
                    processes_arc,
                    event_tx,
                    event_version,
                    error_version,
                );
            }
            ProcessType::Docker => {
                self.start_docker_container(
                    &id_owned,
                    &config,
                    processes_arc,
                    event_tx,
                    event_version,
                    error_version,
                );
            }
        }
    }

    fn start_system_process(
        &self,
        id: &str,
        config: &ProcessConfig,
        processes_arc: Arc<Mutex<HashMap<String, ProcessState>>>,
        event_tx: watch::Sender<u64>,
        event_version: Arc<AtomicU64>,
        error_version: Arc<AtomicU64>,
    ) {
        let id_owned = id.to_string();
        let command = config.command.clone();
        let working_dir = config.working_directory.clone();

        thread::spawn(move || {
            println!("[DEBUG] Thread spawned for command: {}", command);
            println!("[DEBUG] Working dir: '{}'", working_dir);
            
            let (program, args) = match parse_command(&command) {
                Ok((program, args)) => (program, args),
                Err(e) => {
                    let mut processes = processes_arc.lock().unwrap();
                    if let Some(state) = processes.get_mut(&id_owned) {
                        state.status = ProcessStatus::Error(e.clone());
                        state.logs.push(format!("[Failed to start: {}]", e));
                    }
                    bump_error(&error_version);
                    bump_event(&event_tx, &event_version);
                    return;
                }
            };

            // Build command (direct spawn; on Windows, .cmd/.bat are routed through cmd)
            let (mut cmd, program_label) = match build_command(&program, &args) {
                Ok(result) => result,
                Err(e) => {
                    let mut processes = processes_arc.lock().unwrap();
                    if let Some(state) = processes.get_mut(&id_owned) {
                        state.status = ProcessStatus::Error(e.clone());
                        state.logs.push(format!("[Failed to start: {}]", e));
                    }
                    bump_error(&error_version);
                    bump_event(&event_tx, &event_version);
                    return;
                }
            };

            if !working_dir.is_empty() {
                println!("[DEBUG] Setting current_dir to: {}", working_dir);
                cmd.current_dir(&working_dir);
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
            match cmd.spawn() {
                Ok(mut child) => {
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
                    // Capture stdout
                    let stdout = child.stdout.take();
                    let stderr = child.stderr.take();

                    {
                        let mut processes = processes_arc.lock().unwrap();
                        if let Some(state) = processes.get_mut(&id_owned) {
                            state.status = ProcessStatus::Running;
                            state.logs.push(format!("[Started with PID {}]", child.id()));
                            println!("[DEBUG] Status set to Running");
                            state.child = Some(child);
                            #[cfg(windows)]
                            {
                                state.job = job.take();
                            }
                        }
                    }
                    bump_event(&event_tx, &event_version);

                    // Stream stdout in background
                    if let Some(stdout) = stdout {
                        let processes_clone = processes_arc.clone();
                        let id_clone = id_owned.clone();
                        let event_tx = event_tx.clone();
                        let event_version = event_version.clone();
                        let error_version = error_version.clone();
                        thread::spawn(move || {
                            let reader = BufReader::new(stdout);
                            for line in reader.lines().map_while(Result::ok) {
                                let mut updated = false;
                                let mut has_error = false;
                                {
                                    let mut processes = processes_clone.lock().unwrap();
                                    if let Some(state) = processes.get_mut(&id_clone) {
                                        state.logs.push(line);
                                        if let Some(last) = state.logs.last() {
                                            has_error = line_has_error(last);
                                        }
                                        // Keep last 1000 lines
                                        if state.logs.len() > 1000 {
                                            state.logs.remove(0);
                                        }
                                        updated = true;
                                    }
                                }
                                if updated {
                                    if has_error {
                                        bump_error(&error_version);
                                    }
                                    bump_event(&event_tx, &event_version);
                                }
                            }
                        });
                    }

                    // Stream stderr in background
                    if let Some(stderr) = stderr {
                        let processes_clone = processes_arc.clone();
                        let id_clone = id_owned.clone();
                        let event_tx = event_tx.clone();
                        let event_version = event_version.clone();
                        let error_version = error_version.clone();
                        thread::spawn(move || {
                            let reader = BufReader::new(stderr);
                            for line in reader.lines().map_while(Result::ok) {
                                let mut updated = false;
                                let mut has_error = false;
                                {
                                    let mut processes = processes_clone.lock().unwrap();
                                    if let Some(state) = processes.get_mut(&id_clone) {
                                        let formatted = format!("[stderr] {}", line);
                                        has_error = line_has_error(&line);
                                        state.logs.push(formatted);
                                        if state.logs.len() > 1000 {
                                            state.logs.remove(0);
                                        }
                                        updated = true;
                                    }
                                }
                                if updated {
                                    if has_error {
                                        bump_error(&error_version);
                                    }
                                    bump_event(&event_tx, &event_version);
                                }
                            }
                        });
                    }

                    // Monitor process exit
                    let processes_monitor = processes_arc.clone();
                    let id_monitor = id_owned.clone();
                    let event_tx = event_tx.clone();
                    let event_version = event_version.clone();
                    let error_version = error_version.clone();
                    thread::spawn(move || {
                        loop {
                            thread::sleep(std::time::Duration::from_millis(500));
                            let mut updated = false;
                            let mut had_error = false;
                            let mut should_break = false;
                            {
                                let mut processes = processes_monitor.lock().unwrap();
                                if let Some(state) = processes.get_mut(&id_monitor) {
                                    if let Some(ref mut child) = state.child {
                                        match child.try_wait() {
                                            Ok(Some(status)) => {
                                                state.logs.push(format!("[Process exited with: {}]", status));
                                                state.status = ProcessStatus::Stopped;
                                                state.child = None;
                                                #[cfg(windows)]
                                                {
                                                    state.job = None;
                                                }
                                                updated = true;
                                                should_break = true;
                                            }
                                            Ok(None) => {
                                                // Still running
                                            }
                                            Err(e) => {
                                                state.status = ProcessStatus::Error(e.to_string());
                                                state.child = None;
                                                #[cfg(windows)]
                                                {
                                                    state.job = None;
                                                }
                                                updated = true;
                                                had_error = true;
                                                should_break = true;
                                            }
                                        }
                                    } else {
                                        should_break = true;
                                    }
                                } else {
                                    should_break = true;
                                }
                            }
                            if updated {
                                if had_error {
                                    bump_error(&error_version);
                                }
                                bump_event(&event_tx, &event_version);
                            }
                            if should_break {
                                break;
                            }
                        }
                    });
                }
                Err(e) => {
                    let mut processes = processes_arc.lock().unwrap();
                    if let Some(state) = processes.get_mut(&id_owned) {
                        state.status = ProcessStatus::Error(e.to_string());
                        state.logs.push(format!("[Failed to start: {}]", e));
                    }
                    bump_error(&error_version);
                    bump_event(&event_tx, &event_version);
                }
            }
        });
    }

    fn start_docker_container(
        &self,
        id: &str,
        config: &ProcessConfig,
        processes_arc: Arc<Mutex<HashMap<String, ProcessState>>>,
        event_tx: watch::Sender<u64>,
        event_version: Arc<AtomicU64>,
        error_version: Arc<AtomicU64>,
    ) {
        let id_owned = id.to_string();
        let container_name = config.command.clone();

        thread::spawn(move || {
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
                        if output.status.success() {
                            state.status = ProcessStatus::Running;
                            state.logs.push(format!("[Docker container '{}' started]", container_name));
                        } else {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            state.status = ProcessStatus::Error(stderr.to_string());
                            state.logs.push(format!("[Failed to start: {}]", stderr));
                            bump_error(&error_version);
                        }
                    }
                    bump_event(&event_tx, &event_version);
                }
                Err(e) => {
                    let mut processes = processes_arc.lock().unwrap();
                    if let Some(state) = processes.get_mut(&id_owned) {
                        state.status = ProcessStatus::Error(e.to_string());
                        state.logs.push(format!("[Failed to start docker: {}]", e));
                    }
                    bump_error(&error_version);
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
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        let mut updated = false;
                        let mut has_error = false;
                        let mut should_break = false;
                        {
                            let mut processes = processes_arc.lock().unwrap();
                            if let Some(state) = processes.get_mut(&id_owned) {
                                if state.status != ProcessStatus::Running {
                                    should_break = true;
                                } else {
                                    has_error = line_has_error(&line);
                                    state.logs.push(line);
                                    if state.logs.len() > 1000 {
                                        state.logs.remove(0);
                                    }
                                    updated = true;
                                }
                            } else {
                                should_break = true;
                            }
                        }
                        if updated {
                            if has_error {
                                bump_error(&error_version);
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

    /// Stop a process
    pub fn stop_process(&self, id: &str) {
        let processes_arc = self.processes.clone();
        let event_tx = self.event_tx.clone();
        let event_version = self.event_version.clone();
        let id_owned = id.to_string();

        let mut child_to_kill: Option<Child> = None;
        #[cfg(windows)]
        let mut job_to_close: Option<JobHandle> = None;
        let mut docker_container: Option<String> = None;

        {
            let mut processes = processes_arc.lock().unwrap();
            if let Some(state) = processes.get_mut(id) {
                match state.config.process_type {
                    ProcessType::Process => {
                        if let Some(child) = state.child.take() {
                            state.status = ProcessStatus::Stopping;
                            child_to_kill = Some(child);
                            #[cfg(windows)]
                            {
                                job_to_close = state.job.take();
                            }
                        } else {
                            state.status = ProcessStatus::Stopped;
                            #[cfg(windows)]
                            {
                                state.job = None;
                            }
                        }
                    }
                    ProcessType::Docker => {
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
                let pid = child.id();
                let mut stop_error: Option<String> = None;

                #[cfg(windows)]
                {
                    let had_job = job_to_close.is_some();
                    if let Some(job) = job_to_close {
                        drop(job);
                    }
                    if let Err(e) = kill_process_tree(pid) {
                        if !had_job {
                            stop_error = Some(e);
                            let _ = child.kill();
                        }
                    }
                }
                #[cfg(not(windows))]
                {
                    if let Err(e) = child.kill() {
                        stop_error = Some(e.to_string());
                    }
                }

                let _ = child.wait();

                let mut processes = processes_arc.lock().unwrap();
                if let Some(state) = processes.get_mut(&id_owned) {
                    state.child = None;
                    if let Some(err) = stop_error {
                        state.logs.push(format!("[Stop error: {}]", err));
                    }
                    state.logs.push("[Process stopped]".to_string());
                    state.status = ProcessStatus::Stopped;
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
                            state.logs.push(format!("[Docker container '{}' stopped]", container_name));
                        }
                        Ok(out) => {
                            let stderr = String::from_utf8_lossy(&out.stderr);
                            state.logs.push(format!("[Stop error: {}]", stderr));
                            state.status = ProcessStatus::Stopped;
                        }
                        Err(e) => {
                            state.logs.push(format!("[Stop error: {}]", e));
                            state.status = ProcessStatus::Stopped;
                        }
                    }
                }
                bump_event(&event_tx, &event_version);
            });
        }
    }

    /// Restart a process
    pub fn restart_process(&self, id: &str) {
        self.stop_process(id);
        thread::sleep(std::time::Duration::from_millis(500));
        self.start_process(id);
    }

    /// Start all processes
    pub fn start_all(&self) {
        let ids: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes.keys().cloned().collect()
        };
        for id in ids {
            self.start_process(&id);
        }
    }

    /// Stop all processes
    pub fn stop_all(&self) {
        let ids: Vec<String> = {
            let processes = self.processes.lock().unwrap();
            processes.keys().cloned().collect()
        };
        for id in ids {
            self.stop_process(&id);
        }
    }

    /// Restart all processes
    pub fn restart_all(&self) {
        self.stop_all();
        thread::sleep(std::time::Duration::from_millis(500));
        self.start_all();
    }

    /// Stop all non-Docker processes (called on app shutdown)
    pub fn stop_non_docker(&self) {
        let mut processes = self.processes.lock().unwrap();
        for state in processes.values_mut() {
            if state.config.process_type == ProcessType::Process {
                if let Some(ref mut child) = state.child {
                    let pid = child.id();
                    #[cfg(windows)]
                    {
                        if let Some(job) = state.job.take() {
                            drop(job);
                        } else {
                            let _ = kill_process_tree(pid);
                        }
                    }
                    #[cfg(not(windows))]
                    {
                        let _ = child.kill();
                    }
                    let _ = child.wait();
                    state.child = None;
                }
                #[cfg(windows)]
                {
                    state.job = None;
                }
                state.status = ProcessStatus::Stopped;
            }
        }
        self.notify();
    }

    /// Get status of a process
    pub fn get_status(&self, id: &str) -> Option<ProcessStatus> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).map(|s| s.status.clone())
    }

    /// Get logs for a process
    pub fn get_logs(&self, id: &str) -> Vec<String> {
        let processes = self.processes.lock().unwrap();
        processes.get(id).map(|s| s.logs.clone()).unwrap_or_default()
    }

    /// Check and update docker container status
    #[allow(dead_code)]
    pub fn refresh_docker_status(&self, id: &str) {
        refresh_docker_status_inner(
            id,
            &self.processes,
            &self.event_tx,
            &self.event_version,
        );
    }
}

fn bump_event(event_tx: &watch::Sender<u64>, event_version: &Arc<AtomicU64>) {
    let next = event_version.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    let _ = event_tx.send(next);
}

fn bump_error(error_version: &Arc<AtomicU64>) {
    let _ = error_version.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
}

fn line_has_error(line: &str) -> bool {
    let trimmed = line.trim();
    let content = if let Some(rest) = trimmed.strip_prefix("[stderr]") {
        rest.trim_start()
    } else {
        trimmed
    };
    let lower = content.to_ascii_lowercase();
    lower.contains("error")
        || lower.contains("critical")
        || lower.contains("fatal")
        || lower.contains("panic")
        || lower.contains("traceback")
        || lower.contains("exception")
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
fn assign_job(job: &JobHandle, child: &Child) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    let handle = child.as_raw_handle();
    let result = unsafe { AssignProcessToJobObject(job.handle, handle) };
    if result == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn build_command(program: &str, args: &[String]) -> Result<(Command, String), String> {
    let resolved = resolve_program(program)?;
    if resolved.is_cmd_script {
        // Don't pre-quote! Let Rust's Command API handle argument quoting.
        // Previously we used build_cmdline() which quoted the path, then
        // cmd.args(["/S", "/C", &cmdline]) caused Rust to quote the whole
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

#[cfg(not(windows))]
fn build_command(program: &str, args: &[String]) -> Result<(Command, String), String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    Ok((cmd, program.to_string()))
}

#[cfg(windows)]
struct ResolvedProgram {
    path: String,
    is_cmd_script: bool,
}

#[cfg(windows)]
fn resolve_program(program: &str) -> Result<ResolvedProgram, String> {
    use std::env;
    use std::path::Path;

    let program = program.trim();
    if program.is_empty() {
        return Err("Command is empty".to_string());
    }

    let path = Path::new(program);
    let has_separator = program.contains('\\') || program.contains('/');

    if has_separator || path.is_absolute() {
        return resolve_with_extensions(path);
    }

    let extensions = supported_extensions();

    let path_env = env::var_os("PATH").unwrap_or_default();
    for dir in env::split_paths(&path_env) {
        if let Some(resolved) = resolve_in_dir(&dir, program, &extensions) {
            return Ok(resolved);
        }
    }

    Err(format!(
        "Program not found or not executable: {} (expected .exe/.com/.cmd/.bat on PATH)",
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
fn resolve_in_dir(dir: &std::path::Path, program: &str, extensions: &[String]) -> Option<ResolvedProgram> {
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

#[cfg(windows)]
fn build_cmdline(script_path: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_cmd_arg(script_path));
    parts.extend(args.iter().map(|arg| quote_cmd_arg(arg)));
    parts.join(" ")
}

#[cfg(windows)]
fn quote_cmd_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = arg.chars().any(|c| c.is_whitespace() || c == '"');
    if !needs_quotes {
        return arg.to_string();
    }
    let escaped = arg.replace('"', "\\\"");
    format!("\"{}\"", escaped)
}

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
    event_tx: &watch::Sender<u64>,
    event_version: &Arc<AtomicU64>,
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
            {
                let mut processes = processes.lock().unwrap();
                if let Some(state) = processes.get_mut(id) {
                    let next_status = if is_running {
                        ProcessStatus::Running
                    } else {
                        ProcessStatus::Stopped
                    };

                    if state.status != next_status {
                        state.status = next_status;
                        updated = true;
                    }
                }
            }

            if updated {
                bump_event(event_tx, event_version);
            }
        }
    }
}
