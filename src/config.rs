//! Configuration management for the process manager.
//! Handles loading and saving the processes.json file.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

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
}

impl ProcessConfig {
    pub fn new(name: String, command: String, working_directory: String, process_type: ProcessType) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            command,
            working_directory,
            process_type,
            auto_start: false,
        }
    }
}

/// Root configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Name/label for this stack (to identify different instances)
    #[serde(default = "default_stack_name")]
    pub stack_name: String,
    pub processes: Vec<ProcessConfig>,
}

fn default_stack_name() -> String {
    "My Stack".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            stack_name: default_stack_name(),
            processes: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Get the path to the config file (next to the executable)
    pub fn config_path() -> PathBuf {
        let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        let exe_dir = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));
        exe_dir.join("processes.json")
    }

    /// Load config from file, creating default if not found
    pub fn load() -> Self {
        let path = Self::config_path();
        
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) => {
                    match serde_json::from_str(&content) {
                        Ok(config) => return config,
                        Err(e) => {
                            eprintln!("Failed to parse config: {}", e);
                        }
                    }
                }
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
        
        fs::write(&path, content)
            .map_err(|e| format!("Failed to write config: {}", e))?;
        
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
}
