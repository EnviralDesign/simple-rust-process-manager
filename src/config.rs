//! Configuration management for the process manager.
//! Handles loading and saving the processes.json file.

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const DEFAULT_REMOTE_CONTROL_PORT: u16 = 47_821;
pub const DEFAULT_LOG_ROTATION_COUNT: usize = 10;
pub const DEFAULT_PROCESS_ERROR_FLASH_SECONDS: u64 = 5;
pub const DEFAULT_STARTUP_DELAY_SECONDS: u64 = 0;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedRestartSchedule {
    /// Whether managed restart is limited to the weekly active-hours grid.
    pub enabled: bool,
    /// Whether to actively stop the process when an active window ends.
    pub stop_when_inactive: bool,
    /// 168 hourly buckets, Monday 00:00 through Sunday 23:00.
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

    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }

    fn active_hour_indices(&self) -> Vec<usize> {
        self.hours
            .iter()
            .enumerate()
            .filter_map(|(index, enabled)| enabled.then_some(index))
            .collect()
    }
}

impl Serialize for ManagedRestartSchedule {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let active_hours = self.active_hour_indices();
        let field_count = usize::from(self.enabled)
            + usize::from(self.stop_when_inactive)
            + usize::from(!active_hours.is_empty());
        let mut state = serializer.serialize_struct("ManagedRestartSchedule", field_count)?;

        if self.enabled {
            state.serialize_field("enabled", &true)?;
        }
        if self.stop_when_inactive {
            state.serialize_field("stop_when_inactive", &true)?;
        }
        if !active_hours.is_empty() {
            state.serialize_field("active_hours", &active_hours)?;
        }

        state.end()
    }
}

impl<'de> Deserialize<'de> for ManagedRestartSchedule {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = ManagedRestartScheduleWire::deserialize(deserializer)?;
        let mut schedule = ManagedRestartSchedule {
            enabled: wire.enabled,
            stop_when_inactive: wire.stop_when_inactive,
            hours: wire.hours.unwrap_or_else(default_weekly_hours),
        };

        if let Some(active_hours) = wire.active_hours {
            schedule.hours = weekly_hours_from_indices(&active_hours);
        }

        normalize_weekly_hours(&mut schedule.hours);
        Ok(schedule)
    }
}

#[derive(Debug, Default, Deserialize)]
struct ManagedRestartScheduleWire {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    stop_when_inactive: bool,
    #[serde(default)]
    hours: Option<Vec<bool>>,
    #[serde(default)]
    active_hours: Option<Vec<usize>>,
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "is_default_scheduled_run_mode")]
    pub mode: ScheduledRunMode,
    /// Local hour used by Daily and SelectedWeekdays modes.
    #[serde(
        default = "default_scheduled_run_hour",
        skip_serializing_if = "is_default_scheduled_run_hour_value"
    )]
    pub hour: u8,
    /// Interval used by EveryNHours mode.
    #[serde(
        default = "default_scheduled_run_interval_hours",
        skip_serializing_if = "is_default_scheduled_run_interval_hours_value"
    )]
    pub interval_hours: u8,
    /// Seven day flags, Monday through Sunday.
    #[serde(
        default = "default_weekdays",
        skip_serializing_if = "is_default_weekdays_value"
    )]
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
    pub fn is_disabled(&self) -> bool {
        !self.enabled
    }

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
    /// Seconds to wait before honoring any start request for this process.
    #[serde(default = "default_startup_delay_seconds")]
    pub startup_delay_seconds: u64,
    /// Whether to auto-restart when the process exits unexpectedly
    #[serde(default)]
    pub auto_restart: bool,
    /// Optional active-hours gate for managed restart.
    #[serde(
        default,
        deserialize_with = "deserialize_default_on_null",
        skip_serializing_if = "ManagedRestartSchedule::is_disabled"
    )]
    pub restart_schedule: ManagedRestartSchedule,
    /// Optional scheduled start trigger.
    #[serde(
        default,
        deserialize_with = "deserialize_default_on_null",
        skip_serializing_if = "ScheduledRun::is_disabled"
    )]
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

/// One-layer grouping metadata for processes in the sidebar and REST API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessGroupConfig {
    /// Unique identifier
    pub id: String,
    /// Display name
    pub name: String,
    /// Stable process ids that belong to this group.
    #[serde(default)]
    pub process_ids: Vec<String>,
    /// Whether the sidebar folder is expanded.
    #[serde(default = "default_group_expanded")]
    pub expanded: bool,
}

impl ProcessGroupConfig {
    pub fn new(name: String, process_ids: Vec<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            process_ids,
            expanded: true,
        }
    }
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
            startup_delay_seconds: default_startup_delay_seconds(),
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

    pub fn normalize(&mut self) {
        normalize_weekly_hours(&mut self.restart_schedule.hours);
        normalize_weekdays(&mut self.scheduled_run.weekdays);
        self.scheduled_run.hour = self.scheduled_run.hour.min(23);
        self.scheduled_run.interval_hours = self.scheduled_run.interval_hours.clamp(1, 24);
        if self.log_rotation_count == 0 {
            self.log_rotation_count = default_log_rotation_count();
        }
    }
}

fn default_log_rotation_count() -> usize {
    DEFAULT_LOG_ROTATION_COUNT
}

fn default_global_control_enabled() -> bool {
    true
}

fn default_group_expanded() -> bool {
    true
}

fn default_startup_delay_seconds() -> u64 {
    DEFAULT_STARTUP_DELAY_SECONDS
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

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default_scheduled_run_mode(mode: &ScheduledRunMode) -> bool {
    mode == &ScheduledRunMode::default()
}

fn is_default_scheduled_run_hour_value(hour: &u8) -> bool {
    *hour == default_scheduled_run_hour()
}

fn is_default_scheduled_run_interval_hours_value(interval_hours: &u8) -> bool {
    *interval_hours == default_scheduled_run_interval_hours()
}

fn is_default_weekdays_value(weekdays: &[bool]) -> bool {
    weekdays == default_weekdays()
}

fn deserialize_default_on_null<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

fn normalize_weekly_hours(hours: &mut Vec<bool>) {
    if hours.len() < WEEKLY_HOUR_COUNT {
        hours.resize(WEEKLY_HOUR_COUNT, false);
    } else if hours.len() > WEEKLY_HOUR_COUNT {
        hours.truncate(WEEKLY_HOUR_COUNT);
    }
}

fn weekly_hours_from_indices(indices: &[usize]) -> Vec<bool> {
    let mut hours = default_weekly_hours();
    for index in indices
        .iter()
        .copied()
        .filter(|index| *index < WEEKLY_HOUR_COUNT)
    {
        hours[index] = true;
    }
    hours
}

fn normalize_weekdays(days: &mut Vec<bool>) {
    if days.len() < 7 {
        days.resize(7, false);
    } else if days.len() > 7 {
        days.truncate(7);
    }
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
    /// Base directory for persisted process logs. Relative paths resolve beside processes.json.
    #[serde(default = "default_log_directory")]
    pub log_directory: String,
    /// How long the Processes sidebar softly flashes after a new error arrives. Set to 0 to disable.
    #[serde(default = "default_process_error_flash_seconds")]
    pub process_error_flash_seconds: u64,
    /// One-layer sidebar groups. A process can belong to at most one group.
    #[serde(default)]
    pub groups: Vec<ProcessGroupConfig>,
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
            groups: Vec::new(),
            processes: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Config lives beside a portable binary/app, or in Application Support for an installed Mac app.
    pub fn config_path() -> PathBuf {
        crate::platform::data_directory().join("processes.json")
    }

    /// Load config from file, creating default if not found or if parsing fails.
    pub fn load() -> Self {
        match Self::load_from_disk() {
            Ok(config) => {
                let _ = config.save();
                return config;
            }
            Err(err) => {
                eprintln!("Failed to load config from disk: {}", err);
            }
        }

        // Return default config
        let mut config = Self::default();
        config.normalize();
        let _ = config.save(); // Try to save default
        config
    }

    /// Load config from disk without mutating state or creating fallback values.
    pub fn load_from_disk() -> Result<Self, String> {
        let path = Self::config_path();
        Self::load_from_path(&path)
    }

    /// Load config from an explicit path without mutating state or creating fallback values.
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        if !path.exists() {
            return Err("processes.json was not found.".to_string());
        }

        let content =
            fs::read_to_string(&path).map_err(|err| format!("Failed to read config: {}", err))?;
        let mut config = serde_json::from_str::<Self>(&content)
            .map_err(|err| format!("Failed to parse config: {}", err))?;
        config.normalize();
        Ok(config)
    }

    /// Normalize loaded or edited config so older process files round-trip into the current schema.
    pub fn normalize(&mut self) {
        if self.log_directory.trim().is_empty() {
            self.log_directory = default_log_directory();
        }
        if self.remote_control.port == 0 {
            self.remote_control.port = default_remote_control_port();
        }
        for process in &mut self.processes {
            process.normalize();
        }
        self.normalize_groups();
    }

    fn normalize_groups(&mut self) {
        let valid_process_ids: HashSet<String> = self
            .processes
            .iter()
            .map(|process| process.id.clone())
            .collect();
        let mut assigned_process_ids = HashSet::new();
        let mut group_ids = HashSet::new();

        for group in &mut self.groups {
            if group.id.trim().is_empty() || !group_ids.insert(group.id.clone()) {
                group.id = Uuid::new_v4().to_string();
                group_ids.insert(group.id.clone());
            }
            if group.name.trim().is_empty() {
                group.name = "Process Group".to_string();
            }

            group.process_ids.retain(|process_id| {
                valid_process_ids.contains(process_id)
                    && assigned_process_ids.insert(process_id.clone())
            });
        }

        self.groups.retain(|group| !group.process_ids.is_empty());
    }

    /// Save config to file
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {e}"))?;
        }
        self.save_to_path(&path)
    }

    /// Save config to an explicit path after normalizing into the current schema.
    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let mut normalized = self.clone();
        normalized.normalize();
        let content = serde_json::to_string_pretty(&normalized)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        fs::write(path, content).map_err(|e| format!("Failed to write config: {}", e))?;

        Ok(())
    }

    /// Add a new process configuration
    pub fn add_process(&mut self, mut config: ProcessConfig) {
        config.normalize();
        self.processes.push(config);
    }

    /// Clone a process directly after its source, preserving its group membership and position.
    pub fn duplicate_process(&mut self, id: &str) -> Option<ProcessConfig> {
        let source_index = self.processes.iter().position(|process| process.id == id)?;
        let mut duplicate = self.processes[source_index].clone();
        duplicate.id = Uuid::new_v4().to_string();
        duplicate.name = format!("{} (dup)", duplicate.name);
        duplicate.normalize();

        self.processes.insert(source_index + 1, duplicate.clone());

        if let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| group.process_ids.iter().any(|process_id| process_id == id))
        {
            let source_member_index = group
                .process_ids
                .iter()
                .position(|process_id| process_id == id)
                .expect("group membership was checked above");
            group
                .process_ids
                .insert(source_member_index + 1, duplicate.id.clone());
        }

        Some(duplicate)
    }

    /// Remove a process by ID
    pub fn remove_process(&mut self, id: &str) {
        self.processes.retain(|p| p.id != id);
        for group in &mut self.groups {
            group.process_ids.retain(|process_id| process_id != id);
        }
        self.groups.retain(|group| !group.process_ids.is_empty());
    }

    /// Get a process by ID
    pub fn get_process(&self, id: &str) -> Option<&ProcessConfig> {
        self.processes.iter().find(|p| p.id == id)
    }

    /// Get a group by ID
    pub fn get_group(&self, id: &str) -> Option<&ProcessGroupConfig> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// Set a group's expanded state.
    pub fn set_group_expanded(&mut self, id: &str, expanded: bool) -> bool {
        let Some(group) = self.groups.iter_mut().find(|group| group.id == id) else {
            return false;
        };
        group.expanded = expanded;
        true
    }

    /// Create a group from existing process ids, removing those processes from any old group.
    pub fn create_group_from_processes(
        &mut self,
        name: String,
        process_ids: &[String],
    ) -> Option<ProcessGroupConfig> {
        let valid_ids: HashSet<&str> = self
            .processes
            .iter()
            .map(|process| process.id.as_str())
            .collect();
        let mut seen = HashSet::new();
        let ordered_ids: Vec<String> = self
            .processes
            .iter()
            .filter(|process| process_ids.iter().any(|id| id == &process.id))
            .filter(|process| valid_ids.contains(process.id.as_str()))
            .filter(|process| seen.insert(process.id.clone()))
            .map(|process| process.id.clone())
            .collect();

        if ordered_ids.len() < 2 {
            return None;
        }

        for group in &mut self.groups {
            group
                .process_ids
                .retain(|process_id| !ordered_ids.iter().any(|id| id == process_id));
        }
        self.groups.retain(|group| !group.process_ids.is_empty());

        let group = ProcessGroupConfig::new(name, ordered_ids);
        self.groups.push(group.clone());
        self.normalize();
        Some(group)
    }

    /// Remove a group while keeping its processes.
    pub fn remove_group(&mut self, id: &str) -> bool {
        let before = self.groups.len();
        self.groups.retain(|group| group.id != id);
        before != self.groups.len()
    }

    /// Return the group containing a process, if any.
    pub fn group_for_process(&self, process_id: &str) -> Option<&ProcessGroupConfig> {
        self.groups
            .iter()
            .find(|group| group.process_ids.iter().any(|id| id == process_id))
    }

    /// Update a process configuration
    #[allow(dead_code)]
    pub fn update_process(&mut self, id: &str, mut updated: ProcessConfig) {
        updated.normalize();
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

    /// Move a process out of any group and place it before another process, or at the end.
    pub fn move_process_to_top_level(&mut self, id: &str, before_process_id: Option<&str>) -> bool {
        if self.get_process(id).is_none() {
            return false;
        }

        let membership_changed = self.remove_process_from_groups(id);
        let moved = if before_process_id == Some(id) {
            false
        } else {
            self.move_process_before_id(id, before_process_id)
        };
        self.groups.retain(|group| !group.process_ids.is_empty());
        membership_changed || moved
    }

    /// Move a process into a group, optionally before an existing member.
    pub fn move_process_into_group(
        &mut self,
        id: &str,
        group_id: &str,
        before_process_id: Option<&str>,
    ) -> bool {
        if self.get_process(id).is_none() || before_process_id == Some(id) {
            return false;
        }

        let Some(group_index) = self.groups.iter().position(|group| group.id == group_id) else {
            return false;
        };

        if before_process_id.is_some_and(|before_id| {
            !self.groups[group_index]
                .process_ids
                .iter()
                .any(|process_id| process_id == before_id)
        }) {
            return false;
        }

        let membership_changed = self.remove_process_from_groups(id);
        let group = &mut self.groups[group_index];
        let insert_index = before_process_id
            .and_then(|before_id| {
                group
                    .process_ids
                    .iter()
                    .position(|process_id| process_id == before_id)
            })
            .unwrap_or(group.process_ids.len());
        group.process_ids.insert(insert_index, id.to_string());
        self.groups
            .retain(|group| group.id == group_id || !group.process_ids.is_empty());

        let moved = if let Some(before_id) = before_process_id {
            self.move_process_before_id(id, Some(before_id))
        } else {
            self.move_process_after_group_members(id, group_id)
        };

        membership_changed || moved
    }

    /// Move a whole group before another top-level process/group, or to the end.
    pub fn move_group_to_top_level(
        &mut self,
        group_id: &str,
        before_process_id: Option<&str>,
    ) -> bool {
        let Some(group) = self.get_group(group_id) else {
            return false;
        };
        if before_process_id.is_some_and(|before_id| {
            group
                .process_ids
                .iter()
                .any(|process_id| process_id == before_id)
        }) {
            return false;
        }

        let member_ids = group.process_ids.clone();
        let member_id_set: HashSet<&str> = member_ids.iter().map(String::as_str).collect();
        let mut removed_by_id = HashMap::new();
        self.processes.retain(|process| {
            if member_id_set.contains(process.id.as_str()) {
                removed_by_id.insert(process.id.clone(), process.clone());
                false
            } else {
                true
            }
        });

        if removed_by_id.is_empty() {
            return false;
        }

        let insert_index = match before_process_id {
            Some(before_id) => {
                let Some(index) = self
                    .processes
                    .iter()
                    .position(|process| process.id == before_id)
                else {
                    self.processes.extend(
                        member_ids
                            .iter()
                            .filter_map(|process_id| removed_by_id.remove(process_id)),
                    );
                    return false;
                };
                index
            }
            None => self.processes.len(),
        };

        for (offset, process) in member_ids
            .iter()
            .filter_map(|process_id| removed_by_id.remove(process_id))
            .enumerate()
        {
            self.processes.insert(insert_index + offset, process);
        }

        true
    }

    fn remove_process_from_groups(&mut self, id: &str) -> bool {
        let mut removed = false;
        for group in &mut self.groups {
            let before = group.process_ids.len();
            group.process_ids.retain(|process_id| process_id != id);
            removed |= before != group.process_ids.len();
        }
        removed
    }

    fn move_process_before_id(&mut self, id: &str, before_process_id: Option<&str>) -> bool {
        let Some(index) = self.processes.iter().position(|process| process.id == id) else {
            return false;
        };
        let process = self.processes.remove(index);

        let insert_index = match before_process_id {
            Some(before_id) => {
                let Some(index) = self
                    .processes
                    .iter()
                    .position(|process| process.id == before_id)
                else {
                    self.processes.insert(index, process);
                    return false;
                };
                index
            }
            None => self.processes.len(),
        };

        self.processes.insert(insert_index, process);
        insert_index != index
    }

    fn move_process_after_group_members(&mut self, id: &str, group_id: &str) -> bool {
        let Some(group) = self.get_group(group_id) else {
            return false;
        };
        let member_ids: HashSet<&str> = group
            .process_ids
            .iter()
            .filter(|process_id| process_id.as_str() != id)
            .map(String::as_str)
            .collect();
        let insert_after = self
            .processes
            .iter()
            .enumerate()
            .filter(|(_, process)| member_ids.contains(process.id.as_str()))
            .map(|(index, _)| index)
            .max();

        let Some(index) = self.processes.iter().position(|process| process.id == id) else {
            return false;
        };
        let process = self.processes.remove(index);
        let insert_index = insert_after
            .map(|after| if index < after { after } else { after + 1 })
            .unwrap_or_else(|| self.processes.len())
            .min(self.processes.len());
        self.processes.insert(insert_index, process);
        insert_index != index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_startup_delay_defaults_to_zero_and_serializes() {
        let raw = r#"{
            "stack_name": "Test Stack",
            "processes": [
                {
                    "id": "process-1",
                    "name": "API",
                    "command": "cargo run"
                }
            ]
        }"#;

        let mut config: AppConfig = serde_json::from_str(raw).expect("config should parse");
        config.normalize();

        assert_eq!(config.processes[0].startup_delay_seconds, 0);
        let value = serde_json::to_value(&config).expect("config should serialize");
        assert_eq!(value["processes"][0]["startup_delay_seconds"], 0);
        assert!(value["groups"]
            .as_array()
            .is_some_and(|groups| groups.is_empty()));
        let process = value["processes"][0].as_object().unwrap();
        assert!(!process.contains_key("restart_schedule"));
        assert!(!process.contains_key("scheduled_run"));
    }

    #[test]
    fn normalize_repairs_process_schema_edges() {
        let mut process = ProcessConfig::new(
            "Worker".to_string(),
            "worker.exe".to_string(),
            String::new(),
            ProcessType::Process,
        );
        process.restart_schedule.hours = vec![true];
        process.scheduled_run.weekdays = vec![true, false];
        process.scheduled_run.hour = 99;
        process.scheduled_run.interval_hours = 0;
        process.log_rotation_count = 0;

        process.normalize();

        assert_eq!(process.restart_schedule.hours.len(), WEEKLY_HOUR_COUNT);
        assert_eq!(process.scheduled_run.weekdays.len(), 7);
        assert_eq!(process.scheduled_run.hour, 23);
        assert_eq!(process.scheduled_run.interval_hours, 1);
        assert_eq!(process.log_rotation_count, DEFAULT_LOG_ROTATION_COUNT);
    }

    #[test]
    fn schedules_serialize_compactly_and_read_legacy_shapes() {
        let raw = r#"{
            "processes": [
                {
                    "id": "process-1",
                    "name": "Worker",
                    "command": "worker.exe",
                    "restart_schedule": {
                        "enabled": true,
                        "stop_when_inactive": true,
                        "hours": [true, false, true]
                    },
                    "scheduled_run": {
                        "enabled": true,
                        "mode": "EveryNHours",
                        "interval_hours": 6
                    }
                },
                {
                    "id": "process-2",
                    "name": "Disabled",
                    "command": "disabled.exe",
                    "restart_schedule": null,
                    "scheduled_run": null
                }
            ]
        }"#;

        let mut config: AppConfig = serde_json::from_str(raw).expect("config should parse");
        config.normalize();

        assert!(config.processes[0].restart_schedule.enabled);
        assert!(config.processes[0].restart_schedule.stop_when_inactive);
        assert!(config.processes[0].restart_schedule.hours[0]);
        assert!(!config.processes[0].restart_schedule.hours[1]);
        assert!(config.processes[0].restart_schedule.hours[2]);
        assert_eq!(
            config.processes[0].restart_schedule.hours.len(),
            WEEKLY_HOUR_COUNT
        );
        assert_eq!(config.processes[0].scheduled_run.interval_hours, 6);
        assert!(!config.processes[1].restart_schedule.enabled);
        assert!(!config.processes[1].scheduled_run.enabled);

        let value = serde_json::to_value(&config).expect("config should serialize");
        let first = value["processes"][0].as_object().unwrap();
        assert_eq!(
            first["restart_schedule"],
            serde_json::json!({
                "enabled": true,
                "stop_when_inactive": true,
                "active_hours": [0, 2]
            })
        );
        assert_eq!(
            first["scheduled_run"],
            serde_json::json!({
                "enabled": true,
                "mode": "EveryNHours",
                "interval_hours": 6
            })
        );

        let second = value["processes"][1].as_object().unwrap();
        assert!(!second.contains_key("restart_schedule"));
        assert!(!second.contains_key("scheduled_run"));

        let reparsed: AppConfig =
            serde_json::from_value(value).expect("compact config should parse again");
        assert_eq!(
            reparsed.processes[0].restart_schedule.hours,
            config.processes[0].restart_schedule.hours
        );
        assert_eq!(
            reparsed.processes[0].scheduled_run,
            config.processes[0].scheduled_run
        );
    }

    #[test]
    fn normalize_groups_drops_unknown_duplicate_and_empty_memberships() {
        let mut config = AppConfig {
            processes: vec![
                ProcessConfig::new(
                    "API".to_string(),
                    "api.exe".to_string(),
                    String::new(),
                    ProcessType::Process,
                ),
                ProcessConfig::new(
                    "Worker".to_string(),
                    "worker.exe".to_string(),
                    String::new(),
                    ProcessType::Process,
                ),
            ],
            ..AppConfig::default()
        };
        let first_id = config.processes[0].id.clone();
        let second_id = config.processes[1].id.clone();
        config.groups = vec![
            ProcessGroupConfig {
                id: "group-1".to_string(),
                name: "Services".to_string(),
                process_ids: vec![first_id.clone(), "missing".to_string(), first_id.clone()],
                expanded: true,
            },
            ProcessGroupConfig {
                id: "group-2".to_string(),
                name: String::new(),
                process_ids: vec![first_id, second_id.clone()],
                expanded: true,
            },
            ProcessGroupConfig {
                id: "group-3".to_string(),
                name: "Empty".to_string(),
                process_ids: Vec::new(),
                expanded: true,
            },
        ];

        config.normalize();

        assert_eq!(config.groups.len(), 2);
        assert_eq!(
            config.groups[0].process_ids,
            vec![config.processes[0].id.clone()]
        );
        assert_eq!(config.groups[1].name, "Process Group");
        assert_eq!(config.groups[1].process_ids, vec![second_id]);
    }

    #[test]
    fn duplicate_process_inserts_a_renamed_clone_after_its_source() {
        fn process(id: &str, name: &str) -> ProcessConfig {
            let mut process = ProcessConfig::new(
                name.to_string(),
                "cmd.exe".to_string(),
                "C:/work".to_string(),
                ProcessType::Process,
            );
            process.id = id.to_string();
            process.auto_start = true;
            process.auto_restart = true;
            process
        }

        let mut config = AppConfig {
            processes: vec![process("a", "API"), process("b", "Worker")],
            ..AppConfig::default()
        };

        let duplicate = config
            .duplicate_process("a")
            .expect("existing process should duplicate");

        assert_ne!(duplicate.id, "a");
        assert_eq!(duplicate.name, "API (dup)");
        assert_eq!(duplicate.command, "cmd.exe");
        assert_eq!(duplicate.working_directory, "C:/work");
        assert!(duplicate.auto_start);
        assert!(duplicate.auto_restart);
        assert_eq!(process_ids(&config), vec!["a", duplicate.id.as_str(), "b"]);
        assert!(config.duplicate_process("missing").is_none());
    }

    #[test]
    fn duplicate_grouped_process_stays_directly_after_its_source_in_the_group() {
        fn process(id: &str) -> ProcessConfig {
            let mut process = ProcessConfig::new(
                id.to_string(),
                "cmd.exe".to_string(),
                String::new(),
                ProcessType::Process,
            );
            process.id = id.to_string();
            process
        }

        let mut config = AppConfig {
            processes: vec![process("a"), process("b"), process("c"), process("d")],
            groups: vec![ProcessGroupConfig {
                id: "group-1".to_string(),
                name: "Services".to_string(),
                process_ids: vec!["b".to_string(), "c".to_string()],
                expanded: true,
            }],
            ..AppConfig::default()
        };

        let duplicate = config
            .duplicate_process("c")
            .expect("group member should duplicate");

        assert_eq!(
            process_ids(&config),
            vec!["a", "b", "c", duplicate.id.as_str(), "d"]
        );
        assert_eq!(
            config.groups[0].process_ids,
            vec!["b".to_string(), "c".to_string(), duplicate.id]
        );
    }

    #[test]
    fn drag_drop_helpers_move_processes_and_groups() {
        fn process(id: &str) -> ProcessConfig {
            let mut process = ProcessConfig::new(
                id.to_string(),
                "cmd.exe".to_string(),
                String::new(),
                ProcessType::Process,
            );
            process.id = id.to_string();
            process
        }

        let mut config = AppConfig {
            processes: vec![
                process("a"),
                process("b"),
                process("c"),
                process("d"),
                process("e"),
            ],
            groups: vec![
                ProcessGroupConfig {
                    id: "group-1".to_string(),
                    name: "Group 1".to_string(),
                    process_ids: vec!["b".to_string(), "c".to_string()],
                    expanded: true,
                },
                ProcessGroupConfig {
                    id: "group-2".to_string(),
                    name: "Group 2".to_string(),
                    process_ids: vec!["d".to_string(), "e".to_string()],
                    expanded: true,
                },
            ],
            ..AppConfig::default()
        };

        assert!(config.move_process_into_group("a", "group-1", Some("c")));
        assert_eq!(
            config.groups[0].process_ids,
            vec!["b".to_string(), "a".to_string(), "c".to_string()]
        );
        assert_eq!(process_ids(&config), vec!["b", "a", "c", "d", "e"]);

        assert!(config.move_process_to_top_level("b", Some("b")));
        assert_eq!(
            config.groups[0].process_ids,
            vec!["a".to_string(), "c".to_string()]
        );
        assert_eq!(process_ids(&config), vec!["b", "a", "c", "d", "e"]);

        assert!(config.move_group_to_top_level("group-2", Some("b")));
        assert_eq!(process_ids(&config), vec!["d", "e", "b", "a", "c"]);
    }

    fn process_ids(config: &AppConfig) -> Vec<&str> {
        config
            .processes
            .iter()
            .map(|process| process.id.as_str())
            .collect()
    }
}
