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

use crate::config::RemoteControlConfig;
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

    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
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
    port: u16,
}

pub struct RestServerController {
    manager: Arc<ProcessManager>,
    stack_name: Arc<RwLock<String>>,
    desired_config: Mutex<RemoteControlConfig>,
    active_server: Mutex<Option<ActiveServer>>,
    generation: Arc<AtomicU64>,
    snapshot_tx: watch::Sender<RestServerSnapshot>,
}

impl RestServerController {
    pub fn new(manager: Arc<ProcessManager>) -> Self {
        let default_remote = RemoteControlConfig::default();
        let (snapshot_tx, _snapshot_rx) =
            watch::channel(RestServerSnapshot::disabled(default_remote.port));
        Self {
            manager,
            stack_name: Arc::new(RwLock::new(String::new())),
            desired_config: Mutex::new(default_remote),
            active_server: Mutex::new(None),
            generation: Arc::new(AtomicU64::new(0)),
            snapshot_tx,
        }
    }

    pub fn snapshot(&self) -> RestServerSnapshot {
        self.snapshot_tx.borrow().clone()
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
            port: remote_control.port,
        };
        let router = Router::new()
            .route("/health", get(health))
            .route("/processes", get(list_processes))
            .route("/processes/{id}", get(get_process))
            .route("/processes/{id}/logs", get(get_process_logs))
            .route("/processes/{id}/start", post(start_process))
            .route("/processes/{id}/stop", post(stop_process))
            .route("/processes/{id}/restart", post(restart_process))
            .route("/stack/start", post(start_stack))
            .route("/stack/stop", post(stop_stack))
            .route("/stack/restart", post(restart_stack))
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
                description: "Returns all managed processes with ids and runtime status.",
            },
            EndpointDoc {
                method: "GET",
                path: "/processes/{id}",
                description: "Returns one managed process by stable id.",
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
                description: "Starts all managed processes and containers.",
            },
            EndpointDoc {
                method: "POST",
                path: "/stack/stop",
                description: "Stops all managed processes and containers.",
            },
            EndpointDoc {
                method: "POST",
                path: "/stack/restart",
                description: "Restarts the full stack.",
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
        ],
        usage_notes: vec![
            "Control endpoints are fire-and-poll. After a POST, poll GET /processes.",
            "Always target individual components by stable id, not display name.",
            "Fetch recent output with GET /processes/{id}/logs?limit=N when an agent needs tail logs.",
            "This server binds only to 127.0.0.1 and is reachable only from the same machine.",
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

fn stack_ack(action: &'static str) -> AckResponse {
    AckResponse {
        ok: true,
        scope: "stack",
        action,
        target_id: None,
        message: format!("{} requested", capitalize(action)),
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

pub fn build_agent_bootstrap(
    stack_name: &str,
    remote_control: &RemoteControlConfig,
    snapshot: &RestServerSnapshot,
    processes: &[ProcessRuntimeSnapshot],
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
        "2. Call GET /processes to discover process ids and current statuses.".to_string(),
        "3. Call GET /processes/{id}/logs?limit=200 to fetch the latest log tail for a component."
            .to_string(),
        "4. Use POST /stack/start, /stack/stop, or /stack/restart for the full stack.".to_string(),
        "5. Use POST /processes/{id}/start, /stop, or /restart for a single component.".to_string(),
        "6. After any POST, poll GET /processes until the desired state is visible.".to_string(),
        String::new(),
        "Endpoint Topology".to_string(),
        "- GET /health".to_string(),
        "- GET /processes".to_string(),
        "- GET /processes/{id}".to_string(),
        "- GET /processes/{id}/logs?limit=N".to_string(),
        "- GET /topology".to_string(),
        "- POST /stack/start".to_string(),
        "- POST /stack/stop".to_string(),
        "- POST /stack/restart".to_string(),
        "- POST /processes/{id}/start".to_string(),
        "- POST /processes/{id}/stop".to_string(),
        "- POST /processes/{id}/restart".to_string(),
        String::new(),
        "Known Processes".to_string(),
    ];

    if processes.is_empty() {
        lines.push("- No managed processes are configured yet.".to_string());
    } else {
        for process in processes {
            lines.push(format!(
                "- {} | id={} | type={} | status={}",
                process.name, process.id, process.process_type, process.status
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
    }

    lines.join("\n")
}
