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
}

impl ProcessState {
    pub fn new(config: ProcessConfig) -> Self {
        Self {
            config,
            status: ProcessStatus::Stopped,
            logs: Vec::new(),
            child: None,
        }
    }
}

/// Manages all running processes
pub struct ProcessManager {
    pub processes: Arc<Mutex<HashMap<String, ProcessState>>>,
    event_tx: watch::Sender<u64>,
    event_version: Arc<AtomicU64>,
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
            background_started: AtomicBool::new(false),
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.event_tx.subscribe()
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
                );
            }
            ProcessType::Docker => {
                self.start_docker_container(
                    &id_owned,
                    &config,
                    processes_arc,
                    event_tx,
                    event_version,
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
    ) {
        let id_owned = id.to_string();
        let command = config.command.clone();
        let working_dir = config.working_directory.clone();

        thread::spawn(move || {
            println!("[DEBUG] Thread spawned for command: {}", command);
            println!("[DEBUG] Working dir: '{}'", working_dir);
            
            // Build command
            let mut cmd = if cfg!(windows) {
                let mut c = Command::new("cmd");
                c.args(["/C", &command]);
                c
            } else {
                let mut c = Command::new("sh");
                c.args(["-c", &command]);
                c
            };

            if !working_dir.is_empty() {
                println!("[DEBUG] Setting current_dir to: {}", working_dir);
                cmd.current_dir(&working_dir);
            }

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
                    println!("[DEBUG] Command spawned successfully! PID: {:?}", child.id());
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
                        }
                    }
                    bump_event(&event_tx, &event_version);

                    // Stream stdout in background
                    if let Some(stdout) = stdout {
                        let processes_clone = processes_arc.clone();
                        let id_clone = id_owned.clone();
                        let event_tx = event_tx.clone();
                        let event_version = event_version.clone();
                        thread::spawn(move || {
                            let reader = BufReader::new(stdout);
                            for line in reader.lines().map_while(Result::ok) {
                                let mut updated = false;
                                {
                                    let mut processes = processes_clone.lock().unwrap();
                                    if let Some(state) = processes.get_mut(&id_clone) {
                                        state.logs.push(line);
                                        // Keep last 1000 lines
                                        if state.logs.len() > 1000 {
                                            state.logs.remove(0);
                                        }
                                        updated = true;
                                    }
                                }
                                if updated {
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
                        thread::spawn(move || {
                            let reader = BufReader::new(stderr);
                            for line in reader.lines().map_while(Result::ok) {
                                let mut updated = false;
                                {
                                    let mut processes = processes_clone.lock().unwrap();
                                    if let Some(state) = processes.get_mut(&id_clone) {
                                        state.logs.push(format!("[stderr] {}", line));
                                        if state.logs.len() > 1000 {
                                            state.logs.remove(0);
                                        }
                                        updated = true;
                                    }
                                }
                                if updated {
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
                    thread::spawn(move || {
                        loop {
                            thread::sleep(std::time::Duration::from_millis(500));
                            let mut updated = false;
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
                                                updated = true;
                                                should_break = true;
                                            }
                                            Ok(None) => {
                                                // Still running
                                            }
                                            Err(e) => {
                                                state.status = ProcessStatus::Error(e.to_string());
                                                state.child = None;
                                                updated = true;
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
            );
        });
    }

    fn stream_docker_logs(
        id: &str,
        container_name: &str,
        processes_arc: Arc<Mutex<HashMap<String, ProcessState>>>,
        event_tx: watch::Sender<u64>,
        event_version: Arc<AtomicU64>,
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
                        let mut should_break = false;
                        {
                            let mut processes = processes_arc.lock().unwrap();
                            if let Some(state) = processes.get_mut(&id_owned) {
                                if state.status != ProcessStatus::Running {
                                    should_break = true;
                                } else {
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
        let mut processes = self.processes.lock().unwrap();
        if let Some(state) = processes.get_mut(id) {
            match state.config.process_type {
                ProcessType::Process => {
                    if let Some(ref mut child) = state.child {
                        state.status = ProcessStatus::Stopping;
                        let _ = child.kill();
                        let _ = child.wait();
                        state.child = None;
                        state.logs.push("[Process stopped]".to_string());
                    }
                    state.status = ProcessStatus::Stopped;
                    self.notify();
                }
                ProcessType::Docker => {
                    let container_name = state.config.command.clone();
                    state.status = ProcessStatus::Stopping;
                    drop(processes); // Release lock before blocking call

                    let mut cmd = Command::new("docker");
                    cmd.args(["stop", &container_name]);
                    
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        cmd.creation_flags(0x08000000);
                    }

                    let output = cmd.output();
                    
                    let mut processes = self.processes.lock().unwrap();
                    if let Some(state) = processes.get_mut(id) {
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
                    self.notify();
                    return;
                }
            }
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
                    let _ = child.kill();
                    let _ = child.wait();
                    state.child = None;
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
