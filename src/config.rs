//! Configuration management for the process manager.
//! Handles loading and saving the processes.json file.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

pub const DEFAULT_REMOTE_CONTROL_PORT: u16 = 47_821;
pub const DEFAULT_LOG_ROTATION_COUNT: usize = 10;
pub const DEFAULT_PROCESS_ERROR_FLASH_SECONDS: u64 = 5;
pub const WEEKLY_HOUR_COUNT: usize = 7 * 24;

/// Type of process being managed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProcessType {
    /// A regular system process (shell command)
    Process,
    /// A Docker container
    Docker,
}

impl Default for ProcessType {
    fn default() -> Self {
        Self::Process
    }
}

impl std::fmt::Display for ProcessType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProcessType::Process => write!(f, "Process"),
            ProcessType::Docker => write!(f, "Docker"),
        }
    }
}

/// Optional weekly active-hours gate for managed restart.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedRestartSchedule {
    /// Whether managed restart is limited to the weekly active-hours grid.
    #[serde(default)]
    pub enabled: bool,
    /// Whether to actively stop the process when an active window ends.
    #[serde(default)]
    pub stop_when_inactive: bool,
    /// 168 hourly buckets, Monday 00:00 through Sunday 23:00.
    #[serde(default = "default_weekly_hours")]
    pub hours: Vec<bool>,
}

impl Default for ManagedRestartSchedule {
    fn default() -> Self {
        Self {
            enabled: false,
            stop_when_inactive: false,
            hours: default_weekly_hours(),
        }
    }
}

impl ManagedRestartSchedule {
    pub fn active_at(&self, day_index: usize, hour: u32) -> bool {
        if !self.enabled {
            return true;
        }

        weekly_hour_enabled(&self.hours, day_index, hour)
    }
}

/// Human-readable scheduled-run cadence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScheduledRunMode {
    Hourly,
    EveryNHours,
    Daily,
    SelectedWeekdays,
}

impl Default for ScheduledRunMode {
    fn default() -> Self {
        Self::Daily
    }
}

impl std::fmt::Display for ScheduledRunMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hourly => write!(f, "Hourly"),
            Self::EveryNHours => write!(f, "Every N hours"),
            Self::Daily => write!(f, "Daily"),
            Self::SelectedWeekdays => write!(f, "Selected weekdays"),
        }
    }
}

/// Optional scheduled start trigger. This only starts dormant entries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduledRun {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: ScheduledRunMode,
    /// Local hour used by Daily and SelectedWeekdays modes.
    #[serde(default = "default_scheduled_run_hour")]
    pub hour: u8,
    /// Interval used by EveryNHours mode.
    #[serde(default = "default_scheduled_run_interval_hours")]
    pub interval_hours: u8,
    /// Seven day flags, Monday through Sunday.
    #[serde(default = "default_weekdays")]
    pub weekdays: Vec<bool>,
}

impl Default for ScheduledRun {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ScheduledRunMode::Daily,
            hour: default_scheduled_run_hour(),
            interval_hours: default_scheduled_run_interval_hours(),
            weekdays: default_weekdays(),
        }
    }
}

impl ScheduledRun {
    pub fn due_at(&self, day_index: usize, hour: u32, minute: u32) -> bool {
        if !self.enabled || minute != 0 {
            return false;
        }

        match self.mode {
            ScheduledRunMode::Hourly => true,
            ScheduledRunMode::EveryNHours => {
                let interval = self.interval_hours.clamp(1, 24) as u32;
                hour % interval == 0
            }
            ScheduledRunMode::Daily => hour == self.hour.min(23) as u32,
            ScheduledRunMode::SelectedWeekdays => {
                hour == self.hour.min(23) as u32
                    && self.weekdays.get(day_index).copied().unwrap_or(false)
            }
        }
    }
}

/// Configuration for a single managed process
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessConfig {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Command to run (for Process) or container name (for Docker)
    pub command: String,
    /// Working directory (only used for Process type)
    #[serde(default)]
    pub working_directory: String,
    /// Type of process
    #[serde(default)]
    pub process_type: ProcessType,
    /// Whether to auto-start when manager launches
    #[serde(default)]
    pub auto_start: bool,
    /// Whether to auto-restart when the process exits unexpectedly
    #[serde(default)]
    pub auto_restart: bool,
    /// Optional active-hours gate for managed restart.
    #[serde(default)]
    pub restart_schedule: ManagedRestartSchedule,
    /// Optional scheduled start trigger.
    #[serde(default)]
    pub scheduled_run: ScheduledRun,
    /// Whether Start All should start this process
    #[serde(default = "default_global_control_enabled")]
    pub respond_to_start_all: bool,
    /// Whether Stop All should stop this process
    #[serde(default = "default_global_control_enabled")]
    pub respond_to_stop_all: bool,
    /// Whether Restart All should restart this process
    #[serde(default = "default_global_control_enabled")]
    pub respond_to_restart_all: bool,
    /// Whether to persist process logs to disk
    #[serde(default)]
    pub log_to_disk: bool,
    /// How many session log files to keep for this process
    #[serde(default = "default_log_rotation_count")]
    pub log_rotation_count: usize,
}

impl ProcessConfig {
    pub fn new(
        name: String,
        command: String,
        working_directory: String,
        process_type: ProcessType,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            command,
            working_directory,
            process_type,
            auto_start: false,
            auto_restart: false,
            restart_schedule: ManagedRestartSchedule::default(),
            scheduled_run: ScheduledRun::default(),
            respond_to_start_all: true,
            respond_to_stop_all: true,
            respond_to_restart_all: true,
            log_to_disk: false,
            log_rotation_count: default_log_rotation_count(),
        }
    }
}

fn default_log_rotation_count() -> usize {
    DEFAULT_LOG_ROTATION_COUNT
}

fn default_global_control_enabled() -> bool {
    true
}

pub fn default_weekly_hours() -> Vec<bool> {
    vec![false; WEEKLY_HOUR_COUNT]
}

fn default_weekdays() -> Vec<bool> {
    vec![true, true, true, true, true, false, false]
}

fn default_scheduled_run_hour() -> u8 {
    9
}

fn default_scheduled_run_interval_hours() -> u8 {
    1
}

pub fn weekly_hour_index(day_index: usize, hour: u32) -> Option<usize> {
    if day_index >= 7 || hour >= 24 {
        return None;
    }

    Some(day_index * 24 + hour as usize)
}

pub fn weekly_hour_enabled(hours: &[bool], day_index: usize, hour: u32) -> bool {
    weekly_hour_index(day_index, hour)
        .and_then(|index| hours.get(index).copied())
        .unwrap_or(false)
}

/// Configuration for the optional localhost REST control surface
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteControlConfig {
    /// Whether the local REST server is enabled
    #[serde(default)]
    pub enabled: bool,
    /// TCP port to bind on 127.0.0.1
    #[serde(default = "default_remote_control_port")]
    pub port: u16,
}

impl Default for RemoteControlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_remote_control_port(),
        }
    }
}

fn default_remote_control_port() -> u16 {
    DEFAULT_REMOTE_CONTROL_PORT
}

/// Root configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Name/label for this stack (to identify different instances)
    #[serde(default = "default_stack_name")]
    pub stack_name: String,
    /// Optional localhost REST control server settings
    #[serde(default)]
    pub remote_control: RemoteControlConfig,
    /// Base directory for persisted process logs. Relative paths resolve next to the executable.
    #[serde(default = "default_log_directory")]
    pub log_directory: String,
    /// How long the Processes sidebar softly flashes after a new error arrives. Set to 0 to disable.
    #[serde(default = "default_process_error_flash_seconds")]
    pub process_error_flash_seconds: u64,
    #[serde(default)]
    pub processes: Vec<ProcessConfig>,
}

fn default_stack_name() -> String {
    "My Stack".to_string()
}

fn default_log_directory() -> String {
    ".".to_string()
}

fn default_process_error_flash_seconds() -> u64 {
    DEFAULT_PROCESS_ERROR_FLASH_SECONDS
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            stack_name: default_stack_name(),
            remote_control: RemoteControlConfig::default(),
            log_directory: default_log_directory(),
            process_error_flash_seconds: default_process_error_flash_seconds(),
            processes: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Get the path to the config file (next to the executable)
    pub fn config_path() -> PathBuf {
        let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        let exe_dir = exe_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        exe_dir.join("processes.json")
    }

    /// Load config from file, creating default if not found
    pub fn load() -> Self {
        let path = Self::config_path();

        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<Self>(&content) {
                    Ok(config) => {
                        let _ = config.save();
                        return config;
                    }
                    Err(e) => {
                        eprintln!("Failed to parse config: {}", e);
                    }
                },
                Err(e) => {
                    eprintln!("Failed to read config: {}", e);
                }
            }
        }

        // Return default config
        let config = Self::default();
        let _ = config.save(); // Try to save default
        config
    }

    /// Save config to file
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(&path, content).map_err(|e| format!("Failed to write config: {}", e))?;

        Ok(())
    }

    /// Add a new process configuration
    pub fn add_process(&mut self, config: ProcessConfig) {
        self.processes.push(config);
    }

    /// Remove a process by ID
    pub fn remove_process(&mut self, id: &str) {
        self.processes.retain(|p| p.id != id);
    }

    /// Get a process by ID
    pub fn get_process(&self, id: &str) -> Option<&ProcessConfig> {
        self.processes.iter().find(|p| p.id == id)
    }

    /// Update a process configuration
    #[allow(dead_code)]
    pub fn update_process(&mut self, id: &str, updated: ProcessConfig) {
        if let Some(process) = self.processes.iter_mut().find(|p| p.id == id) {
            *process = updated;
        }
    }

    /// Move a process one slot earlier in the list.
    pub fn move_process_up(&mut self, id: &str) -> bool {
        let Some(index) = self.processes.iter().position(|process| process.id == id) else {
            return false;
        };
        if index == 0 {
            return false;
        }

        self.processes.swap(index, index - 1);
        true
    }

    /// Move a process one slot later in the list.
    pub fn move_process_down(&mut self, id: &str) -> bool {
        let Some(index) = self.processes.iter().position(|process| process.id == id) else {
            return false;
        };
        if index + 1 >= self.processes.len() {
            return false;
        }

        self.processes.swap(index, index + 1);
        true
    }

    /// Move a process to a specific slot in the list.
    pub fn move_process_to_index(&mut self, id: &str, target_index: usize) -> bool {
        let Some(index) = self.processes.iter().position(|process| process.id == id) else {
            return false;
        };
        if index == target_index || target_index >= self.processes.len() {
            return false;
        }

        let process = self.processes.remove(index);
        self.processes.insert(target_index, process);
        true
    }
}
