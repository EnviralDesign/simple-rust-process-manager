use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, watch};

use crate::config::{AppConfig, ProcessGroupConfig, RemoteControlConfig};
use crate::process_manager::{ProcessCounts, ProcessManager, ProcessRuntimeSnapshot};

pub const REST_HOST: &str = "127.0.0.1";
const DEFAULT_LOG_LIMIT: usize = 200;
const MAX_LOG_LIMIT: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestServerState {
    Disabled,
    Starting,
    Running,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestServerSnapshot {
    pub state: RestServerState,
    pub host: String,
    pub port: u16,
    pub message: Option<String>,
}

impl RestServerSnapshot {
    pub fn disabled(port: u16) -> Self {
        Self {
            state: RestServerState::Disabled,
            host: REST_HOST.to_string(),
            port,
            message: None,
        }
    }

    pub fn starting(port: u16) -> Self {
        Self {
            state: RestServerState::Starting,
            host: REST_HOST.to_string(),
            port,
            message: None,
        }
    }

    pub fn running(port: u16) -> Self {
        Self {
            state: RestServerState::Running,
            host: REST_HOST.to_string(),
            port,
            message: None,
        }
    }

    pub fn error(port: u16, message: impl Into<String>) -> Self {
        Self {
            state: RestServerState::Error,
            host: REST_HOST.to_string(),
            port,
            message: Some(message.into()),
        }
    }

    pub fn status_label(&self) -> &'static str {
        match self.state {
            RestServerState::Disabled => "Off",
            RestServerState::Starting => "Starting",
            RestServerState::Running => "On",
            RestServerState::Error => "Error",
        }
    }
}

struct ActiveServer {
    shutdown_tx: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
struct ApiState {
    manager: Arc<ProcessManager>,
    stack_name: Arc<RwLock<String>>,
    config_reload_tx: watch::Sender<ConfigReloadEvent>,
    port: u16,
}

#[derive(Clone, Debug, Default)]
pub struct ConfigReloadEvent {
    pub version: u64,
    pub config: Option<AppConfig>,
    pub process_id: Option<String>,
}

pub struct RestServerController {
    manager: Arc<ProcessManager>,
    stack_name: Arc<RwLock<String>>,
    desired_config: Mutex<RemoteControlConfig>,
    active_server: Mutex<Option<ActiveServer>>,
    generation: Arc<AtomicU64>,
    snapshot_tx: watch::Sender<RestServerSnapshot>,
    config_reload_tx: watch::Sender<ConfigReloadEvent>,
}

impl RestServerController {
    pub fn new(manager: Arc<ProcessManager>) -> Self {
        let default_remote = RemoteControlConfig::default();
        let (snapshot_tx, _snapshot_rx) =
            watch::channel(RestServerSnapshot::disabled(default_remote.port));
        let (config_reload_tx, _config_reload_rx) = watch::channel(ConfigReloadEvent::default());
        Self {
            manager,
            stack_name: Arc::new(RwLock::new(String::new())),
            desired_config: Mutex::new(default_remote),
            active_server: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
            snapshot_tx,
            config_reload_tx,
        }
    }

    pub fn snapshot(&self) -> RestServerSnapshot {
        self.snapshot_tx.borrow().clone()
    }

    pub fn config_reload_event(&self) -> ConfigReloadEvent {
        self.config_reload_tx.borrow().clone()
    }

    pub fn apply_config(&self, stack_name: String, remote_control: RemoteControlConfig) {
        if let Ok(mut guard) = self.stack_name.write() {
            *guard = stack_name;
        }

        let mut desired = self.desired_config.lock().unwrap();
        if *desired == remote_control {
            return;
        }
        *desired = remote_control.clone();
        drop(desired);

        let generation = self
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);

        self.stop_active_server();

        if !remote_control.enabled {
            self.publish_snapshot(RestServerSnapshot::disabled(remote_control.port));
            return;
        }

        self.publish_snapshot(RestServerSnapshot::starting(remote_control.port));

        let listener = match bind_listener(remote_control.port) {
            Ok(listener) => listener,
            Err(err) => {
                self.publish_snapshot(RestServerSnapshot::error(remote_control.port, err));
                return;
            }
        };

        let app_state = ApiState {
            manager: self.manager.clone(),
            stack_name: self.stack_name.clone(),
            config_reload_tx: self.config_reload_tx.clone(),
            port: remote_control.port,
        };
        let router = Router::new()
            .route("/health", get(health))
            .route("/groups", get(list_groups))
            .route("/groups/{id}", get(get_group))
            .route("/groups/{id}/start", post(start_group))
            .route("/groups/{id}/stop", post(stop_group))
            .route("/groups/{id}/restart", post(restart_group))
            .route("/processes", get(list_processes))
            .route("/processes/{id}", get(get_process))
            .route("/processes/{id}/logs", get(get_process_logs))
            .route("/processes/{id}/start", post(start_process))
            .route("/processes/{id}/stop", post(stop_process))
            .route("/processes/{id}/restart", post(restart_process))
            .route("/processes/{id}/reload", post(reload_process))
            .route("/stack/start", post(start_stack))
            .route("/stack/stop", post(stop_stack))
            .route("/stack/restart", post(restart_stack))
            .route("/stack/reload", post(reload_stack))
            .route("/topology", get(topology))
            .with_state(app_state);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let snapshot_tx = self.snapshot_tx.clone();
        let generation_ref = self.generation.clone();
        let port = remote_control.port;

        let task = tokio::spawn(async move {
            let serve_result = axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;

            if let Err(err) = serve_result {
                if generation_ref.load(Ordering::SeqCst) == generation {
                    let _ = snapshot_tx.send(RestServerSnapshot::error(port, err.to_string()));
                }
            }
        });

        let mut active_server = self.active_server.lock().unwrap();
        *active_server = Some(ActiveServer { shutdown_tx, task });
        drop(active_server);

        self.publish_snapshot(RestServerSnapshot::running(remote_control.port));
    }

    pub fn shutdown(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.stop_active_server();

        let desired = self.desired_config.lock().unwrap().clone();
        self.publish_snapshot(RestServerSnapshot::disabled(desired.port));
    }

    fn stop_active_server(&self) {
        let mut active_server = self.active_server.lock().unwrap();
        if let Some(server) = active_server.take() {
            let _ = server.shutdown_tx.send(());
            server.task.abort();
        }
    }

    fn publish_snapshot(&self, snapshot: RestServerSnapshot) {
        let _ = self.snapshot_tx.send(snapshot);
    }
}

fn bind_listener(port: u16) -> Result<tokio::net::TcpListener, String> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut last_error = None;

    for _ in 0..5 {
        match StdTcpListener::bind(address) {
            Ok(listener) => {
                listener
                    .set_nonblocking(true)
                    .map_err(|err| format!("Failed to configure listener: {}", err))?;
                return tokio::net::TcpListener::from_std(listener)
                    .map_err(|err| format!("Failed to register listener: {}", err));
            }
            Err(err) => {
                last_error = Some(err.to_string());
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    Err(format!(
        "Could not bind {}:{} ({})",
        REST_HOST,
        port,
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

#[derive(Serialize)]
struct BindInfo {
    host: &'static str,
    port: u16,
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    stack_name: String,
    bind: BindInfo,
    server_enabled: bool,
    process_counts: ProcessCounts,
}

#[derive(Serialize)]
struct AckResponse {
    ok: bool,
    scope: &'static str,
    action: &'static str,
    target_id: Option<String>,
    message: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    ok: bool,
    message: String,
}

#[derive(Serialize)]
struct TopologyResponse {
    base_url: String,
    bind: BindInfo,
    read_endpoints: Vec<EndpointDoc>,
    control_endpoints: Vec<EndpointDoc>,
    usage_notes: Vec<&'static str>,
}

#[derive(Serialize)]
struct EndpointDoc {
    method: &'static str,
    path: &'static str,
    description: &'static str,
}

#[derive(Deserialize)]
struct LogQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct ProcessLogsResponse {
    ok: bool,
    process_id: String,
    limit: usize,
    returned_lines: usize,
    total_available_lines: usize,
    lines: Vec<String>,
}

#[derive(Serialize)]
struct GroupRuntimeSnapshot {
    id: String,
    name: String,
    expanded: bool,
    process_ids: Vec<String>,
    process_count: usize,
    status: String,
    process_counts: ProcessCounts,
    cpu_percent: Option<f32>,
    memory_bytes: Option<u64>,
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        stack_name: current_stack_name(&state.stack_name),
        bind: BindInfo {
            host: REST_HOST,
            port: state.port,
        },
        server_enabled: true,
        process_counts: state.manager.get_counts(),
    })
}

async fn list_processes(State(state): State<ApiState>) -> Json<Vec<ProcessRuntimeSnapshot>> {
    Json(state.manager.list_processes())
}

async fn get_process(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    match state.manager.get_process_snapshot(&id) {
        Some(process) => (StatusCode::OK, Json(process)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                ok: false,
                message: format!("Unknown process id '{}'", id),
            }),
        )
            .into_response(),
    }
}

async fn get_process_logs(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(query): Query<LogQuery>,
) -> impl IntoResponse {
    let limit = normalize_log_limit(query.limit);
    let total_available_lines = match state.manager.get_log_count(&id) {
        Some(count) => count,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    ok: false,
                    message: format!("Unknown process id '{}'", id),
                }),
            )
                .into_response();
        }
    };

    let lines = state
        .manager
        .get_recent_logs(&id, limit)
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(ProcessLogsResponse {
            ok: true,
            process_id: id,
            limit,
            returned_lines: lines.len(),
            total_available_lines,
            lines,
        }),
    )
        .into_response()
}

async fn list_groups(State(state): State<ApiState>) -> impl IntoResponse {
    let config = match AppConfig::load_from_disk() {
        Ok(mut config) => {
            config.normalize();
            config
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    ok: false,
                    message: format!("Failed to read process groups: {}", err),
                }),
            )
                .into_response();
        }
    };

    let processes = state.manager.list_processes();
    let snapshots = group_snapshots(&config.groups, &processes);
    (StatusCode::OK, Json(snapshots)).into_response()
}

async fn get_group(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    let config = match AppConfig::load_from_disk() {
        Ok(mut config) => {
            config.normalize();
            config
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    ok: false,
                    message: format!("Failed to read process groups: {}", err),
                }),
            )
                .into_response();
        }
    };

    let Some(group) = config.get_group(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                ok: false,
                message: format!("Unknown group id '{}'", id),
            }),
        )
            .into_response();
    };

    let processes = state.manager.list_processes();
    (StatusCode::OK, Json(group_snapshot(group, &processes))).into_response()
}

async fn start_group(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    group_action(&state.manager, id, "start")
}

async fn stop_group(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    group_action(&state.manager, id, "stop")
}

async fn restart_group(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    group_action(&state.manager, id, "restart")
}

async fn start_stack(State(state): State<ApiState>) -> Json<AckResponse> {
    state.manager.start_all();
    Json(stack_ack("start"))
}

async fn stop_stack(State(state): State<ApiState>) -> Json<AckResponse> {
    state.manager.stop_all();
    Json(stack_ack("stop"))
}

async fn restart_stack(State(state): State<ApiState>) -> Json<AckResponse> {
    state.manager.restart_all();
    Json(stack_ack("restart"))
}

async fn reload_stack(State(state): State<ApiState>) -> impl IntoResponse {
    let mut config = match AppConfig::load_from_disk() {
        Ok(config) => config,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    ok: false,
                    message: format!("Failed to reload process configuration: {}", err),
                }),
            )
                .into_response();
        }
    };

    let count = config.processes.len();
    config.normalize();
    if let Ok(mut stack_name) = state.stack_name.write() {
        *stack_name = config.stack_name.clone();
    }
    state
        .manager
        .set_log_directory(config.log_directory.clone());
    state.manager.reload_from_config(&config.processes);
    publish_config_reload(&state.config_reload_tx, config, None);
    Json(stack_ack_with_message(
        "reload",
        format!(
            "Reload requested from disk for {count} process(es). All managed processes were stopped first.",
        )
    ))
        .into_response()
}

async fn start_process(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    process_action(&state.manager, id, "start", ProcessManager::start_process)
}

async fn stop_process(State(state): State<ApiState>, Path(id): Path<String>) -> impl IntoResponse {
    process_action(&state.manager, id, "stop", ProcessManager::stop_process)
}

async fn restart_process(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    process_action(
        &state.manager,
        id,
        "restart",
        ProcessManager::restart_process,
    )
}

async fn reload_process(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut config = match AppConfig::load_from_disk() {
        Ok(config) => config,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    ok: false,
                    message: format!("Failed to reload process configuration: {}", err),
                }),
            )
                .into_response();
        }
    };

    config.normalize();

    let updated_process = match config.processes.iter().find(|process| process.id == id) {
        Some(process) => process.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    ok: false,
                    message: format!("Process id '{}' not found in processes.json", id),
                }),
            )
                .into_response();
        }
    };

    if !state
        .manager
        .reload_process_from_config(updated_process.clone())
    {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                ok: false,
                message: format!("Unknown process id '{}'", id),
            }),
        )
            .into_response();
    }

    publish_config_reload(&state.config_reload_tx, config, Some(id.clone()));

    (
        StatusCode::OK,
        Json(AckResponse {
            ok: true,
            scope: "process",
            action: "reload",
            target_id: Some(id),
            message: format!(
                "Reloaded process '{}' from processes.json.",
                updated_process.name
            ),
        }),
    )
        .into_response()
}

fn publish_config_reload(
    tx: &watch::Sender<ConfigReloadEvent>,
    config: AppConfig,
    process_id: Option<String>,
) {
    tx.send_modify(move |event| {
        *event = ConfigReloadEvent {
            version: event.version.wrapping_add(1),
            config: Some(config),
            process_id,
        };
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_reload_events_advance_and_preserve_scope() {
        let (tx, rx) = watch::channel(ConfigReloadEvent::default());
        drop(rx);

        publish_config_reload(&tx, AppConfig::default(), None);
        let stack_event = tx.borrow().clone();
        assert_eq!(stack_event.version, 1);
        assert!(stack_event.config.is_some());
        assert_eq!(stack_event.process_id, None);

        publish_config_reload(&tx, AppConfig::default(), Some("worker".to_string()));
        let process_event = tx.borrow().clone();
        assert_eq!(process_event.version, 2);
        assert_eq!(process_event.process_id.as_deref(), Some("worker"));
    }
}

async fn topology(State(state): State<ApiState>) -> Json<TopologyResponse> {
    Json(TopologyResponse {
        base_url: format!("http://{}:{}", REST_HOST, state.port),
        bind: BindInfo {
            host: REST_HOST,
            port: state.port,
        },
        read_endpoints: vec![
            EndpointDoc {
                method: "GET",
                path: "/health",
                description: "Returns stack metadata and aggregate process counts.",
            },
            EndpointDoc {
                method: "GET",
                path: "/processes",
                description: "Returns all managed processes with ids, runtime status, PID, CPU percent, and RAM bytes.",
            },
            EndpointDoc {
                method: "GET",
                path: "/groups",
                description: "Returns configured process groups with member ids, aggregate status, CPU percent, and RAM bytes.",
            },
            EndpointDoc {
                method: "GET",
                path: "/groups/{id}",
                description: "Returns one process group by stable id with aggregate status, CPU percent, and RAM bytes.",
            },
            EndpointDoc {
                method: "GET",
                path: "/processes/{id}",
                description: "Returns one managed process by stable id, including PID, CPU percent, and RAM bytes.",
            },
            EndpointDoc {
                method: "GET",
                path: "/processes/{id}/logs?limit=N",
                description:
                    "Returns the last N log lines for one managed process. Default 200, max 1000.",
            },
            EndpointDoc {
                method: "GET",
                path: "/topology",
                description: "Returns a self-description of the API surface.",
            },
        ],
        control_endpoints: vec![
            EndpointDoc {
                method: "POST",
                path: "/stack/start",
                description: "Starts entries that opt into Start All.",
            },
            EndpointDoc {
                method: "POST",
                path: "/stack/stop",
                description: "Stops entries that opt into Stop All.",
            },
            EndpointDoc {
                method: "POST",
                path: "/stack/restart",
                description: "Restarts entries that opt into Restart All.",
            },
            EndpointDoc {
                method: "POST",
                path: "/stack/reload",
                description: "Reloads processes from processes.json. Stops all managed processes first.",
            },
            EndpointDoc {
                method: "POST",
                path: "/groups/{id}/start",
                description: "Starts group members that opt into Start All.",
            },
            EndpointDoc {
                method: "POST",
                path: "/groups/{id}/stop",
                description: "Stops group members that opt into Stop All.",
            },
            EndpointDoc {
                method: "POST",
                path: "/groups/{id}/restart",
                description: "Restarts group members that opt into Restart All.",
            },
            EndpointDoc {
                method: "POST",
                path: "/processes/{id}/start",
                description: "Starts a single managed process or container.",
            },
            EndpointDoc {
                method: "POST",
                path: "/processes/{id}/stop",
                description: "Stops a single managed process or container.",
            },
            EndpointDoc {
                method: "POST",
                path: "/processes/{id}/restart",
                description: "Restarts a single managed process or container.",
            },
            EndpointDoc {
                method: "POST",
                path: "/processes/{id}/reload",
                description: "Reloads one managed process from processes.json.",
            },
        ],
        usage_notes: vec![
            "Control endpoints are fire-and-poll. After a POST, poll GET /processes.",
            "Always target individual components and groups by stable id, not display name.",
            "Fetch recent output with GET /processes/{id}/logs?limit=N when an agent needs tail logs.",
            "Group membership lives in processes.json. To regroup entries, edit the groups array and then call POST /stack/reload.",
            "This server binds only to 127.0.0.1 and is reachable only from the same machine.",
            "POST /stack/reload stops all managed processes first, regardless of status or stack-control settings.",
            "POST /processes/{id}/reload only refreshes config for one process.",
        ],
    })
}

fn normalize_log_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LOG_LIMIT).clamp(1, MAX_LOG_LIMIT)
}

fn process_action(
    manager: &Arc<ProcessManager>,
    id: String,
    action: &'static str,
    action_fn: fn(&ProcessManager, &str),
) -> axum::response::Response {
    if manager.get_process_snapshot(&id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                ok: false,
                message: format!("Unknown process id '{}'", id),
            }),
        )
            .into_response();
    }

    action_fn(manager.as_ref(), &id);

    (
        StatusCode::OK,
        Json(AckResponse {
            ok: true,
            scope: "process",
            action,
            target_id: Some(id),
            message: format!("{} requested", capitalize(action)),
        }),
    )
        .into_response()
}

fn group_action(
    manager: &Arc<ProcessManager>,
    id: String,
    action: &'static str,
) -> axum::response::Response {
    let config = match AppConfig::load_from_disk() {
        Ok(mut config) => {
            config.normalize();
            config
        }
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    ok: false,
                    message: format!("Failed to read process groups: {}", err),
                }),
            )
                .into_response();
        }
    };

    let Some(group) = config.get_group(&id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                ok: false,
                message: format!("Unknown group id '{}'", id),
            }),
        )
            .into_response();
    };

    let process_ids = group.process_ids.clone();
    let group_name = group.name.clone();

    let affected_count = match action {
        "start" => manager.start_processes_respecting_start_all(&process_ids),
        "stop" => manager.stop_processes_respecting_stop_all(&process_ids),
        "restart" => manager.restart_processes_respecting_restart_all(&process_ids),
        _ => 0,
    };

    (
        StatusCode::OK,
        Json(AckResponse {
            ok: true,
            scope: "group",
            action,
            target_id: Some(id),
            message: format!(
                "{} requested for group '{}' ({} eligible of {} process(es))",
                capitalize(action),
                group_name,
                affected_count,
                process_ids.len()
            ),
        }),
    )
        .into_response()
}

fn group_snapshots(
    groups: &[ProcessGroupConfig],
    processes: &[ProcessRuntimeSnapshot],
) -> Vec<GroupRuntimeSnapshot> {
    let process_by_id: HashMap<&str, &ProcessRuntimeSnapshot> = processes
        .iter()
        .map(|process| (process.id.as_str(), process))
        .collect();

    groups
        .iter()
        .map(|group| group_snapshot_from_map(group, &process_by_id))
        .collect()
}

fn group_snapshot(
    group: &ProcessGroupConfig,
    processes: &[ProcessRuntimeSnapshot],
) -> GroupRuntimeSnapshot {
    let process_by_id: HashMap<&str, &ProcessRuntimeSnapshot> = processes
        .iter()
        .map(|process| (process.id.as_str(), process))
        .collect();
    group_snapshot_from_map(group, &process_by_id)
}

fn group_snapshot_from_map(
    group: &ProcessGroupConfig,
    process_by_id: &HashMap<&str, &ProcessRuntimeSnapshot>,
) -> GroupRuntimeSnapshot {
    let mut counts = ProcessCounts {
        total: group.process_ids.len(),
        ..ProcessCounts::default()
    };
    let mut cpu_total = 0.0f32;
    let mut has_cpu = false;
    let mut memory_total = 0u64;
    let mut has_memory = false;

    for process_id in &group.process_ids {
        match process_by_id.get(process_id.as_str()) {
            Some(process) => {
                match process.status.as_str() {
                    "Running" => counts.running += 1,
                    "Starting" => counts.starting += 1,
                    "Stopping" => counts.stopping += 1,
                    status if status.starts_with("Error") => counts.error += 1,
                    _ => counts.stopped += 1,
                }
                if let Some(cpu_percent) = process.cpu_percent {
                    cpu_total += cpu_percent;
                    has_cpu = true;
                }
                if let Some(memory_bytes) = process.memory_bytes {
                    memory_total = memory_total.saturating_add(memory_bytes);
                    has_memory = true;
                }
            }
            None => counts.stopped += 1,
        }
    }

    let status = if counts.error > 0 {
        format!("Error: {} member(s)", counts.error)
    } else if counts.stopping > 0 {
        "Stopping".to_string()
    } else if counts.starting > 0 {
        "Starting".to_string()
    } else if counts.running > 0 {
        "Running".to_string()
    } else {
        "Stopped".to_string()
    };

    GroupRuntimeSnapshot {
        id: group.id.clone(),
        name: group.name.clone(),
        expanded: group.expanded,
        process_ids: group.process_ids.clone(),
        process_count: group.process_ids.len(),
        status,
        process_counts: counts,
        cpu_percent: has_cpu.then_some(cpu_total),
        memory_bytes: has_memory.then_some(memory_total),
    }
}

fn stack_ack(action: &'static str) -> AckResponse {
    stack_ack_with_message(action, format!("{} requested", capitalize(action)))
}

fn stack_ack_with_message(action: &'static str, message: impl Into<String>) -> AckResponse {
    AckResponse {
        ok: true,
        scope: "stack",
        action,
        target_id: None,
        message: message.into(),
    }
}

fn capitalize(action: &str) -> String {
    let mut chars = action.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn current_stack_name(stack_name: &Arc<RwLock<String>>) -> String {
    stack_name
        .read()
        .map(|name| name.clone())
        .unwrap_or_else(|_| String::new())
}

fn format_optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".to_string())
}

fn format_optional_cpu(value: Option<f32>) -> String {
    value
        .map(|value| format!("{:.1}%", value))
        .unwrap_or_else(|| "--".to_string())
}

fn format_optional_bytes(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "--".to_string())
}

pub fn build_agent_bootstrap(
    stack_name: &str,
    remote_control: &RemoteControlConfig,
    snapshot: &RestServerSnapshot,
    processes: &[ProcessRuntimeSnapshot],
    groups: &[ProcessGroupConfig],
) -> String {
    let mut lines = vec![
        "Local Process Manager Skill".to_string(),
        format!("Stack: {}", stack_name),
        format!("Host: {}", REST_HOST),
        format!("Port: {}", remote_control.port),
        format!("Base URL: http://{}:{}", REST_HOST, remote_control.port),
        format!("Current REST status: {}", snapshot.status_label()),
        "Scope: loopback-only (127.0.0.1); this API is not exposed to the network.".to_string(),
        String::new(),
        "Usage".to_string(),
        "1. Call GET /health to confirm the server is reachable.".to_string(),
        "2. Call GET /processes to discover process ids, current statuses, PID, CPU percent, and RAM bytes.".to_string(),
        "3. Call GET /groups to discover group ids, member process ids, aggregate status, CPU percent, and RAM bytes.".to_string(),
        "4. Call GET /processes/{id}/logs?limit=200 to fetch the latest log tail for a component."
            .to_string(),
        "5. Use POST /stack/reload to reread processes.json from disk. This stops all managed processes first, regardless of status or stack-control settings.".to_string(),
        "6. To regroup entries, edit the groups array in processes.json next to the Process Manager executable, then call POST /stack/reload.".to_string(),
        "7. Use POST /processes/{id}/reload to reread one process from processes.json."
            .to_string(),
        "8. Use POST /stack/start, /stack/stop, or /stack/restart for entries that opt into each stack control."
            .to_string(),
        "9. Use POST /groups/{id}/start, /stop, or /restart for group members that opt into the corresponding stack control."
            .to_string(),
        "10. Use POST /processes/{id}/start, /stop, or /restart for a single component."
            .to_string(),
        "11. After any POST, poll GET /processes or GET /groups until the desired state is visible."
            .to_string(),
        String::new(),
        "Endpoint Topology".to_string(),
        "- GET /health".to_string(),
        "- GET /processes".to_string(),
        "- GET /groups".to_string(),
        "- GET /groups/{id}".to_string(),
        "- GET /processes/{id}".to_string(),
        "- GET /processes/{id}/logs?limit=N".to_string(),
        "- GET /topology".to_string(),
        "- POST /stack/start".to_string(),
        "- POST /stack/stop".to_string(),
        "- POST /stack/restart".to_string(),
        "- POST /stack/reload".to_string(),
        "- POST /groups/{id}/start".to_string(),
        "- POST /groups/{id}/stop".to_string(),
        "- POST /groups/{id}/restart".to_string(),
        "- POST /processes/{id}/start".to_string(),
        "- POST /processes/{id}/stop".to_string(),
        "- POST /processes/{id}/restart".to_string(),
        "- POST /processes/{id}/reload".to_string(),
        String::new(),
        "Known Groups".to_string(),
    ];

    if groups.is_empty() {
        lines.push("- No process groups are configured yet.".to_string());
    } else {
        let group_snapshots = group_snapshots(groups, processes);
        for group in group_snapshots {
            lines.push(format!(
                "- {} | id={} | status={} | members={} | member_ids={} | cpu={} | ram_bytes={}",
                group.name,
                group.id,
                group.status,
                group.process_count,
                group.process_ids.join(","),
                format_optional_cpu(group.cpu_percent),
                format_optional_bytes(group.memory_bytes)
            ));
        }
    }

    lines.extend([String::new(), "Known Processes".to_string()]);

    if processes.is_empty() {
        lines.push("- No managed processes are configured yet.".to_string());
    } else {
        for process in processes {
            lines.push(format!(
                "- {} | id={} | type={} | status={} | pid={} | cpu={} | ram_bytes={} | auto_start={} | startup_delay_seconds={} | auto_restart={} | stack_start={} | stack_stop={} | stack_restart={}",
                process.name,
                process.id,
                process.process_type,
                process.status,
                format_optional_u32(process.pid),
                format_optional_cpu(process.cpu_percent),
                format_optional_bytes(process.memory_bytes),
                process.auto_start,
                process.startup_delay_seconds,
                process.auto_restart,
                process.respond_to_start_all,
                process.respond_to_stop_all,
                process.respond_to_restart_all
            ));
        }
    }

    lines.push(String::new());
    if !remote_control.enabled {
        lines.push(
            "Note: the local REST server is currently disabled. Ask the operator to enable Local API in the Process Manager header before calling it."
                .to_string(),
        );
    } else if snapshot.state == RestServerState::Error {
        lines.push(format!(
            "Note: the API is configured as enabled but is currently reporting an error: {}",
            snapshot
                .message
                .clone()
                .unwrap_or_else(|| "unknown error".to_string())
        ));
    } else {
        lines.push(
            "Note: target individual components by stable id rather than by display name."
                .to_string(),
        );
        lines.push(
            "Note: target groups by stable group id; call GET /groups when you need the current group list."
                .to_string(),
        );
        lines.push(
            "Note: regroup by editing the groups array in processes.json, then call POST /stack/reload."
                .to_string(),
        );
        lines.push(
            "Note: POST /processes/{id}/reload updates only that process from processes.json."
                .to_string(),
        );
        lines.push(
            "Note: stack reload (POST /stack/reload) stops all managed processes first, regardless of status or stack-control settings."
                .to_string(),
        );
    }

    lines.join("\n")
}
