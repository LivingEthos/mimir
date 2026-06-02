//! Loopback-only HTTP/WebSocket API for Mimir Studio.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path as AxumPath, Query, State, WebSocketUpgrade};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, options, post};
use axum::{Json, Router};
use camino::{Utf8Path, Utf8PathBuf};
use ignore::WalkBuilder;
use mimir_context::WhyResult;
use mimir_runs::{RunDir, RunId};
use mimir_schemas::ContextPacket;
use mimir_session::packet::{ReplayRequestPreview, ShareBundlePreview};
use mimir_session::{
    builtin_command_registry, parse_session_command, CommandSpec, CommandSupport,
    ContextSuggestRequest, DoctorRequest, ExploreRequest, SessionCommand, SessionEvent,
    SessionEventKind, SessionId, SessionMetadata, SessionRecord, SessionStore, TurnRunner,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_ARTIFACT_BYTES: u64 = 512 * 1024;
const MAX_FILE_SEARCH_RESULTS: usize = 100;
const MAX_SYMBOL_SCAN_BYTES: u64 = 128 * 1024;
const MIMIR_WS_PROTOCOL: &str = "mimir.studio.v1";
const MIMIR_WS_TOKEN_PROTOCOL_PREFIX: &str = "mimir-token.";
const TRACE_STATUS_ARTIFACTS: &[&str] = &["trace.spans.jsonl", "trace.json"];
const ALLOWED_VITE_DEV_ORIGINS: &[&str] = &[
    "http://127.0.0.1:5173",
    "http://localhost:5173",
    "http://[::1]:5173",
];

/// Configuration for the local Mimir Studio UI server.
#[derive(Debug, Clone)]
pub struct UiServerConfig {
    workspace_root: Utf8PathBuf,
    token: String,
    studio_assets_root: Option<Utf8PathBuf>,
}

impl UiServerConfig {
    /// Create a UI server config for a workspace.
    #[must_use]
    pub fn new(workspace_root: Utf8PathBuf, token: Option<String>) -> Self {
        Self {
            workspace_root,
            token: token.unwrap_or_else(generate_ui_token),
            studio_assets_root: None,
        }
    }

    /// Use an explicit directory containing a built Mimir Studio `index.html`.
    #[must_use]
    pub fn with_studio_assets_root(mut self, root: impl Into<Utf8PathBuf>) -> Self {
        self.studio_assets_root = Some(root.into());
        self
    }

    /// Return the generated or configured UI token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Return the configured built Studio asset directory, if any.
    #[must_use]
    pub fn studio_assets_root(&self) -> Option<&Utf8Path> {
        self.studio_assets_root.as_deref()
    }

    /// Build display metadata for an already-bound listener.
    ///
    /// # Errors
    /// Returns an error if the listener address cannot be inspected.
    pub fn info_for_listener(&self, listener: &tokio::net::TcpListener) -> Result<UiServerInfo> {
        let addr = listener.local_addr()?;
        Ok(UiServerInfo {
            base_url: format!("http://{addr}"),
            token: self.token.clone(),
        })
    }
}

/// Connection details for the local UI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiServerInfo {
    /// Base HTTP URL, for example `http://127.0.0.1:49152`.
    pub base_url: String,
    /// Short-lived local UI token.
    pub token: String,
}

impl UiServerInfo {
    /// Browser launch URL carrying the per-process UI token.
    #[must_use]
    pub fn launch_url(&self) -> String {
        format!("{}/?token={}", self.base_url, self.token)
    }
}

#[derive(Debug, Clone)]
struct UiState {
    workspace_root: Utf8PathBuf,
    token: String,
    allowed_origins: BTreeSet<String>,
    sessions: SessionStore,
    studio_assets_root: Option<Utf8PathBuf>,
}

#[derive(Debug)]
struct UiError {
    status: StatusCode,
    message: String,
}

impl UiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: mimir_security::redact_secrets(&message.into()),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "missing or invalid UI token")
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        let _ = error;
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
}

impl IntoResponse for UiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionCreateRequest {
    title: Option<String>,
    workspace_root: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionCreateResponse {
    metadata: UiSessionMetadata,
    events: Vec<Value>,
}

#[derive(Debug, Serialize)]
struct SessionLoadResponse {
    metadata: UiSessionMetadata,
    events: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct SessionMessageRequest {
    message: String,
    provider: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct SessionMessageResponse {
    session_id: String,
    command: String,
    result: Value,
    events: Vec<Value>,
}

#[derive(Debug, Serialize)]
struct UiSessionMetadata {
    schema_version: u32,
    session_id: SessionId,
    title: String,
    workspace_name: String,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize)]
struct CancelResponse {
    session_id: String,
    cancelled: bool,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalResponseRequest {
    action: String,
}

#[derive(Debug, Serialize)]
struct ApprovalResponse {
    approval_id: String,
    accepted: bool,
    action: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceStatus {
    workspace_name: String,
    git: GitStatus,
    mimir: MimirStatus,
    providers: Vec<ProviderStatus>,
    commands: Vec<CommandSpec>,
}

#[derive(Debug, Serialize)]
struct GitStatus {
    is_repo: bool,
    branch: Option<String>,
    dirty: bool,
}

#[derive(Debug, Serialize)]
struct MimirStatus {
    initialized: bool,
    config_present: bool,
    checks_loaded: usize,
    sessions_count: usize,
    runs_count: usize,
    recent_runs: Vec<RunSummary>,
}

#[derive(Debug, Serialize)]
struct ProviderStatus {
    provider: String,
    models_count: usize,
    credential_detected: bool,
}

#[derive(Debug, Deserialize)]
struct WorkspaceFilesQuery {
    token: Option<String>,
    q: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct WorkspaceFileSearchResponse {
    results: Vec<WorkspaceFileMatch>,
}

#[derive(Debug, Serialize)]
struct WorkspaceFileMatch {
    path: String,
    kind: String,
    line: Option<u32>,
    symbol: Option<String>,
}

#[derive(Debug, Serialize)]
struct ArtifactListResponse {
    run_id: String,
    trace_status: TraceStatus,
    artifacts: Vec<ArtifactSummary>,
}

#[derive(Debug, Serialize)]
struct ArtifactSummary {
    name: String,
    path: String,
    size_bytes: u64,
    sha256: String,
    checksum_basis: String,
    redacted: bool,
}

#[derive(Debug, Serialize)]
struct ArtifactContentResponse {
    name: String,
    path: String,
    content_type: String,
    sha256: String,
    checksum_basis: String,
    redacted: bool,
    content: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RunSummary {
    run_id: String,
    path: String,
    artifact_count: usize,
    has_context_packet: bool,
    trace_status: TraceStatus,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct TraceStatus {
    state: TraceStatusState,
    redacted: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum TraceStatusState {
    Absent,
    Recorded,
    Unavailable,
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    after: Option<u64>,
}

/// Run the Mimir Studio UI server from an already-bound listener.
///
/// # Errors
/// Returns an error if the listener is not loopback, the workspace cannot be
/// canonicalized, or the HTTP server fails.
pub async fn run_ui_server(
    listener: tokio::net::TcpListener,
    config: UiServerConfig,
) -> Result<()> {
    let addr = listener.local_addr()?;
    if !addr.ip().is_loopback() {
        anyhow::bail!("mimir UI server must bind to a loopback address");
    }

    let base_url = format!("http://{addr}");
    let workspace_root = canonical_utf8(&config.workspace_root)?;
    let studio_assets_root = config
        .studio_assets_root
        .or_else(|| discover_studio_assets_root(&workspace_root));
    let state = UiState {
        sessions: SessionStore::for_workspace(workspace_root.clone()),
        workspace_root,
        token: config.token,
        allowed_origins: allowed_ui_origins(&base_url),
        studio_assets_root,
    };

    let app = ui_router(state);
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

fn ui_router(state: UiState) -> Router {
    Router::new()
        .route("/", get(studio_index))
        .route("/assets/*path", get(studio_asset))
        .route("/v1/sessions", post(create_session).get(list_sessions))
        .route("/v1/sessions/:session_id", get(load_session_endpoint))
        .route(
            "/v1/sessions/:session_id/messages",
            post(submit_session_message),
        )
        .route("/v1/sessions/:session_id/cancel", post(cancel_session))
        .route("/v1/sessions/:session_id/events", get(session_events_ws))
        .route("/v1/approvals/:approval_id/respond", post(respond_approval))
        .route("/v1/runs/:run_id/artifacts", get(list_run_artifacts))
        .route("/v1/runs/:run_id/artifacts/:name", get(fetch_run_artifact))
        .route("/v1/runs/:run_id/replay", get(fetch_run_replay))
        .route(
            "/v1/runs/:run_id/share",
            get(fetch_run_share).post(fetch_run_share),
        )
        .route("/v1/workspace/commands", get(workspace_commands_endpoint))
        .route("/v1/workspace/files", get(workspace_files))
        .route("/v1/workspace/status", get(workspace_status_endpoint))
        .route("/*path", options(preflight).get(studio_fallback))
        .with_state(state)
}

async fn studio_index(
    State(state): State<UiState>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Response, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;

    if let Some(root) = &state.studio_assets_root {
        let path = root.join("index.html");
        if path.is_file() {
            return static_file_response(&path, "text/html; charset=utf-8");
        }
    }

    Ok((
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        fallback_studio_html(),
    )
        .into_response())
}

async fn studio_asset(
    State(state): State<UiState>,
    AxumPath(path): AxumPath<String>,
) -> Result<Response, UiError> {
    let Some(root) = &state.studio_assets_root else {
        return Err(UiError::not_found("Studio assets were not found"));
    };
    let path = safe_static_asset_path(&root.join("assets"), &path)?;
    static_file_response(&path, static_content_type(&path))
}

async fn studio_fallback(
    State(state): State<UiState>,
    AxumPath(path): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Response, UiError> {
    if path == "v1" || path.starts_with("v1/") {
        return Err(UiError::not_found("API endpoint not found"));
    }
    studio_index(State(state), Query(query), headers).await
}

async fn preflight(State(state): State<UiState>, headers: HeaderMap) -> Result<Response, UiError> {
    check_origin(&headers, &state)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    add_cors_headers(response.headers_mut(), &headers, &state);
    Ok(response)
}

async fn create_session(
    State(state): State<UiState>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<SessionCreateRequest>,
) -> Result<Json<SessionCreateResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    let workspace_root = requested_workspace_root(&state, body.workspace_root.as_deref())?;
    let title = body
        .title
        .unwrap_or_else(|| "Mimir Studio session".to_string());
    let session = state
        .sessions
        .create(workspace_root, title)
        .map_err(UiError::internal)?;
    let events = session.read_events().map_err(UiError::internal)?;
    Ok(Json(SessionCreateResponse {
        metadata: ui_session_metadata(session.metadata()),
        events: ui_session_events(&state, &events)?,
    }))
}

async fn list_sessions(
    State(state): State<UiState>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<UiSessionMetadata>>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    Ok(Json(
        state
            .sessions
            .list()
            .map_err(UiError::internal)?
            .iter()
            .map(ui_session_metadata)
            .collect(),
    ))
}

async fn load_session_endpoint(
    State(state): State<UiState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<SessionLoadResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    let session = open_session(&state, &session_id)?;
    let events = session.read_events().map_err(UiError::internal)?;
    Ok(Json(SessionLoadResponse {
        metadata: ui_session_metadata(session.metadata()),
        events: ui_session_events(&state, &events)?,
    }))
}

fn validate_slash_command_contract(input: &str) -> Result<(), UiError> {
    let Some(command_name) = slash_command_name(input) else {
        return Ok(());
    };
    let Some(command) = command_spec(&command_name) else {
        return Err(UiError::bad_request(format!(
            "{command_name} is not a recognized Mimir Studio command. Run /help for the command registry."
        )));
    };
    if command.support == CommandSupport::Backend && command.enabled {
        return Ok(());
    }
    if command.support == CommandSupport::Local {
        return Err(UiError::bad_request(format!(
            "{} is handled locally by Mimir Studio and is not a backend command.",
            command.name
        )));
    }
    let reason = command
        .disabled_reason
        .unwrap_or("this command is visible in Studio but is not executable yet");
    Err(UiError::bad_request(format!(
        "{} is planned but not connected to the Studio backend yet: {reason}",
        command.name
    )))
}

fn slash_command_name(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let command_line = trimmed.strip_prefix('/')?;
    let name = command_line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    Some(if name.is_empty() {
        "/".to_string()
    } else {
        format!("/{name}")
    })
}

fn command_spec(name: &str) -> Option<&'static CommandSpec> {
    builtin_command_registry()
        .iter()
        .find(|command| command.name == name)
}

async fn submit_session_message(
    State(state): State<UiState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<SessionMessageRequest>,
) -> Result<Json<SessionMessageResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    validate_slash_command_contract(&body.message)?;
    let mut session = open_session(&state, &session_id)?;
    let command = parse_session_command(&body.message);
    let runner = TurnRunner::for_workspace(state.workspace_root.clone());

    let (command_name, result, events) = match command {
        SessionCommand::Status => run_status_turn(&state, &mut session)?,
        SessionCommand::Doctor => {
            let result = runner
                .run_doctor(
                    &mut session,
                    DoctorRequest {
                        version: env!("CARGO_PKG_VERSION").to_string(),
                    },
                )
                .map_err(UiError::internal)?;
            (
                "doctor".to_string(),
                serde_json::to_value(&result).map_err(UiError::internal)?,
                result.events,
            )
        }
        SessionCommand::Check => {
            let result = runner.run_check(&mut session).map_err(UiError::internal)?;
            (
                "check".to_string(),
                serde_json::to_value(&result).map_err(UiError::internal)?,
                result.events,
            )
        }
        SessionCommand::Explore { query } if !query.is_empty() => {
            let result = runner
                .run_explore(&mut session, ExploreRequest { query })
                .map_err(UiError::internal)?;
            (
                "explore".to_string(),
                serde_json::to_value(&result).map_err(UiError::internal)?,
                result.events,
            )
        }
        SessionCommand::Context { task } if !task.is_empty() => {
            let result = runner
                .run_context_suggest(
                    &mut session,
                    ContextSuggestRequest {
                        task,
                        provider: body.provider.unwrap_or_else(|| "glm".to_string()),
                        model: body.model,
                    },
                )
                .map_err(UiError::internal)?;
            (
                "context".to_string(),
                serde_json::to_value(&result).map_err(UiError::internal)?,
                result.events,
            )
        }
        SessionCommand::Init => run_init_turn(&state, &mut session)?,
        SessionCommand::Why { path } if !path.is_empty() => {
            run_context_why_turn(&state, &mut session, path)?
        }
        SessionCommand::Runs => run_runs_turn(&state, &mut session)?,
        SessionCommand::Help => {
            let registry = builtin_command_registry();
            let result = json!({
                "commands": registry
                    .iter()
                    .map(|command| command.usage)
                    .collect::<Vec<_>>(),
                "registry": registry.to_vec(),
            });
            let events = append_simple_turn(&mut session, "help", result.clone())?;
            ("help".to_string(), result, events)
        }
        SessionCommand::Explore { .. } => {
            return Err(UiError::bad_request(
                "/explore requires a question, for example /explore how is auth handled",
            ));
        }
        SessionCommand::Context { .. } => {
            return Err(UiError::bad_request(
                "/context requires a task, for example /context explain the session flow",
            ));
        }
        SessionCommand::Why { .. } => {
            return Err(UiError::bad_request(
                "/why requires a workspace path, for example /why crates/mimir-server/src/ui.rs",
            ));
        }
        SessionCommand::Plan { .. } => {
            return Err(UiError::bad_request(
                "/plan is planned but not connected to the Studio backend yet",
            ));
        }
        SessionCommand::Code { .. } => {
            return Err(UiError::bad_request(
                "/code is planned but not connected to the Studio backend yet",
            ));
        }
        SessionCommand::Share { .. } => {
            return Err(UiError::bad_request(
                "/share is handled locally by Mimir Studio and is not a backend command.",
            ));
        }
        SessionCommand::Diff => {
            return Err(UiError::bad_request(
                "/diff is planned but not connected to the Studio backend yet",
            ));
        }
        SessionCommand::Settings | SessionCommand::Resume { .. } => {
            return Err(UiError::bad_request(
                "this command is handled locally by Mimir Studio",
            ));
        }
        SessionCommand::UserPrompt { .. } => {
            return Err(UiError::bad_request(
                "Mimir Studio backend accepts slash commands only. Run /help for the command registry.",
            ));
        }
    };

    let mut result = result;
    mimir_security::redact_json_value(&mut result);
    sanitize_ui_json_value(&state, &mut result);
    let events = ui_session_events(&state, &events)?;

    Ok(Json(SessionMessageResponse {
        session_id,
        command: command_name,
        result,
        events,
    }))
}

async fn cancel_session(
    State(state): State<UiState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<CancelResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    let _session = open_session(&state, &session_id)?;
    Ok(Json(CancelResponse {
        session_id,
        cancelled: false,
        reason: "no active provider-backed turn is running".to_string(),
    }))
}

async fn respond_approval(
    State(state): State<UiState>,
    AxumPath(approval_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
    Json(body): Json<ApprovalResponseRequest>,
) -> Result<Json<ApprovalResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    if !is_safe_segment(&approval_id) {
        return Err(UiError::bad_request("invalid approval id"));
    }
    Ok(Json(ApprovalResponse {
        approval_id,
        accepted: false,
        action: body.action,
        reason: "no pending approval broker is active in Phase 2".to_string(),
    }))
}

async fn workspace_status_endpoint(
    State(state): State<UiState>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<WorkspaceStatus>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    Ok(Json(workspace_status(&state)?))
}

async fn workspace_commands_endpoint(
    State(state): State<UiState>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<CommandSpec>>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    Ok(Json(builtin_command_registry().to_vec()))
}

async fn workspace_files(
    State(state): State<UiState>,
    Query(query): Query<WorkspaceFilesQuery>,
    headers: HeaderMap,
) -> Result<Json<WorkspaceFileSearchResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    let limit = query
        .limit
        .unwrap_or(MAX_FILE_SEARCH_RESULTS)
        .min(MAX_FILE_SEARCH_RESULTS);
    Ok(Json(WorkspaceFileSearchResponse {
        results: search_workspace_files(&state.workspace_root, query.q.as_deref(), limit)
            .map_err(UiError::internal)?,
    }))
}

async fn list_run_artifacts(
    State(state): State<UiState>,
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<ArtifactListResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    let run_dir = safe_run_dir(&state.workspace_root, &run_id)?;
    let entries = fs::read_dir(&run_dir).map_err(UiError::internal)?;
    let mut artifacts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = entry.file_type().map_err(UiError::internal)?;
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_safe_segment(name) || artifact_name_has_secret_like_text(name) {
            continue;
        }
        let artifact_path = safe_child_file(&run_dir, name)?;
        let metadata = fs::metadata(&artifact_path).map_err(UiError::internal)?;
        if metadata.len() > MAX_ARTIFACT_BYTES {
            continue;
        }
        let bytes = fs::read(&artifact_path).map_err(UiError::internal)?;
        let Ok((_, _, redacted_bytes)) = redacted_artifact_content(&state.workspace_root, &bytes)
        else {
            continue;
        };
        if redacted_bytes.len() as u64 > MAX_ARTIFACT_BYTES {
            continue;
        }
        artifacts.push(ArtifactSummary {
            name: name.to_string(),
            path: path_to_workspace_string(&state.workspace_root, artifact_path.as_std_path())?,
            size_bytes: metadata.len(),
            sha256: sha256_hex(&redacted_bytes),
            checksum_basis: "redacted_preview".to_string(),
            redacted: true,
        });
    }
    artifacts.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(Json(ArtifactListResponse {
        run_id,
        trace_status: trace_status_for_run(&run_dir),
        artifacts,
    }))
}

async fn fetch_run_artifact(
    State(state): State<UiState>,
    AxumPath((run_id, name)): AxumPath<(String, String)>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<ArtifactContentResponse>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    if !is_safe_segment(&name) {
        return Err(UiError::bad_request("invalid artifact name"));
    }
    if artifact_name_has_secret_like_text(&name) {
        return Err(UiError::bad_request(
            "artifact name contains secret-like content",
        ));
    }
    let run_dir = safe_run_dir(&state.workspace_root, &run_id)?;
    let artifact_path = safe_child_file(&run_dir, &name)?;
    let metadata = fs::metadata(&artifact_path).map_err(UiError::internal)?;
    if metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(UiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "artifact exceeds UI fetch size cap",
        ));
    }
    let bytes = fs::read(&artifact_path).map_err(UiError::internal)?;
    let (content_type, content, redacted_bytes) =
        redacted_artifact_content(&state.workspace_root, &bytes)?;
    if redacted_bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        return Err(UiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "artifact preview exceeds UI fetch size cap",
        ));
    }
    let sha256 = sha256_hex(&redacted_bytes);
    Ok(Json(ArtifactContentResponse {
        name,
        path: path_to_workspace_string(&state.workspace_root, artifact_path.as_std_path())?,
        content_type,
        sha256,
        checksum_basis: "redacted_preview".to_string(),
        redacted: true,
        content,
    }))
}

async fn fetch_run_replay(
    State(state): State<UiState>,
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<ReplayRequestPreview>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    mimir_session::packet::replay_request_preview_for_run(&state.workspace_root, &run_id)
        .map(Json)
        .map_err(packet_ui_error)
}

async fn fetch_run_share(
    State(state): State<UiState>,
    AxumPath(run_id): AxumPath<String>,
    Query(query): Query<AuthQuery>,
    headers: HeaderMap,
) -> Result<Json<ShareBundlePreview>, UiError> {
    authorize(&headers, query.token.as_deref(), &state)?;
    check_origin(&headers, &state)?;
    mimir_session::packet::share_bundle_preview_for_run(&state.workspace_root, &run_id)
        .map(Json)
        .map_err(packet_ui_error)
}

async fn session_events_ws(
    State(state): State<UiState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<EventQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, UiError> {
    authorize_websocket(&headers, &state)?;
    check_origin(&headers, &state)?;
    let session_id =
        SessionId::parse(session_id).map_err(|_| UiError::bad_request("invalid session id"))?;
    state
        .sessions
        .open(&session_id)
        .map_err(|_| UiError::not_found("session not found"))?;
    Ok(ws
        .protocols([MIMIR_WS_PROTOCOL])
        .on_upgrade(move |socket| stream_session_events(socket, state, session_id, query.after)))
}

async fn stream_session_events(
    mut socket: WebSocket,
    state: UiState,
    session_id: SessionId,
    mut last_seen: Option<u64>,
) {
    loop {
        let session = match state.sessions.open(&session_id) {
            Ok(session) => session,
            Err(error) => {
                tracing::warn!("session event stream open failed: {error}");
                let _ = socket
                    .send(Message::Text(
                        json!({
                            "type": "turn.failed",
                            "payload": {"error": "session event stream unavailable"}
                        })
                        .to_string(),
                    ))
                    .await;
                break;
            }
        };

        match session.read_events() {
            Ok(events) => {
                for event in events {
                    if last_seen.is_none_or(|sequence| event.sequence > sequence) {
                        last_seen = Some(event.sequence);
                        let event = match ui_session_event(&state, &event) {
                            Ok(event) => event,
                            Err(error) => {
                                tracing::warn!("failed to prepare session event for UI: {error:?}");
                                continue;
                            }
                        };
                        let payload = match serde_json::to_string(&event) {
                            Ok(payload) => payload,
                            Err(error) => {
                                tracing::warn!("failed to serialize session event: {error}");
                                continue;
                            }
                        };
                        if socket.send(Message::Text(payload)).await.is_err() {
                            return;
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!("session event stream read failed: {error}");
                let _ = socket
                    .send(Message::Text(
                        json!({
                            "type": "turn.failed",
                            "payload": {"error": "session event stream unavailable"}
                        })
                        .to_string(),
                    ))
                    .await;
                break;
            }
        }

        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(200)) => {}
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

fn run_status_turn(
    state: &UiState,
    session: &mut SessionRecord,
) -> Result<(String, serde_json::Value, Vec<SessionEvent>), UiError> {
    let status = workspace_status(state)?;
    let result = serde_json::to_value(&status).map_err(UiError::internal)?;
    let events = append_status_turn(session, result.clone())?;
    Ok(("status".to_string(), result, events))
}

fn run_init_turn(
    state: &UiState,
    session: &mut SessionRecord,
) -> Result<(String, serde_json::Value, Vec<SessionEvent>), UiError> {
    let created =
        mimir_session::init_project_files(&state.workspace_root).map_err(UiError::internal)?;
    let created_any = !created.is_empty();
    let status = serde_json::to_value(workspace_status(state)?).map_err(UiError::internal)?;
    let result = json!({
        "created": created,
        "status": status.clone(),
    });
    let summary = if created_any {
        "Initialized Mimir project workflow files"
    } else {
        "Mimir project files already initialized"
    };
    let events = append_status_result_turn(session, "init", String::new(), status, summary)?;
    Ok(("init".to_string(), result, events))
}

fn run_context_why_turn(
    state: &UiState,
    session: &mut SessionRecord,
    path: String,
) -> Result<(String, serde_json::Value, Vec<SessionEvent>), UiError> {
    let result = context_why_for_session(state, session, &path)?;
    let status = result
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let events = append_result_turn(
        session,
        "why",
        path,
        result.clone(),
        format!("Context why lookup: {status}"),
    )?;
    Ok(("why".to_string(), result, events))
}

fn run_runs_turn(
    state: &UiState,
    session: &mut SessionRecord,
) -> Result<(String, serde_json::Value, Vec<SessionEvent>), UiError> {
    let runs = list_local_runs(&state.workspace_root)?;
    let result = json!({ "runs": runs });
    let events = append_result_turn(
        session,
        "runs",
        String::new(),
        result.clone(),
        "Listed local Mimir runs",
    )?;
    Ok(("runs".to_string(), result, events))
}

fn append_status_turn(
    session: &mut SessionRecord,
    status: serde_json::Value,
) -> Result<Vec<SessionEvent>, UiError> {
    append_status_result_turn(
        session,
        "status",
        String::new(),
        status,
        "Workspace status loaded",
    )
}

fn append_status_result_turn(
    session: &mut SessionRecord,
    command: &str,
    task: String,
    status: serde_json::Value,
    summary: &str,
) -> Result<Vec<SessionEvent>, UiError> {
    let turn_id = format!("turn-{}", Uuid::new_v4());
    let events = vec![
        session
            .append_event(SessionEventKind::TurnStarted {
                turn_id: turn_id.clone(),
                command: command.to_string(),
                task,
            })
            .map_err(UiError::internal)?,
        session
            .append_event(SessionEventKind::WorkspaceStatusReady { status })
            .map_err(UiError::internal)?,
        session
            .append_event(SessionEventKind::TurnCompleted {
                turn_id,
                summary: summary.to_string(),
            })
            .map_err(UiError::internal)?,
    ];
    Ok(events)
}

fn append_simple_turn(
    session: &mut SessionRecord,
    command: &str,
    payload: serde_json::Value,
) -> Result<Vec<SessionEvent>, UiError> {
    append_result_turn(
        session,
        command,
        String::new(),
        payload,
        format!("{command} completed"),
    )
}

fn append_result_turn(
    session: &mut SessionRecord,
    command: &str,
    task: String,
    payload: serde_json::Value,
    summary: impl Into<String>,
) -> Result<Vec<SessionEvent>, UiError> {
    let turn_id = format!("turn-{}", Uuid::new_v4());
    let events = vec![
        session
            .append_event(SessionEventKind::TurnStarted {
                turn_id: turn_id.clone(),
                command: command.to_string(),
                task,
            })
            .map_err(UiError::internal)?,
        session
            .append_event(SessionEventKind::WorkspaceStatusReady { status: payload })
            .map_err(UiError::internal)?,
        session
            .append_event(SessionEventKind::TurnCompleted {
                turn_id,
                summary: summary.into(),
            })
            .map_err(UiError::internal)?,
    ];
    Ok(events)
}

fn workspace_status(state: &UiState) -> Result<WorkspaceStatus, UiError> {
    let git = git_status(&state.workspace_root);
    let config_present = state.workspace_root.join(".mimir/config.yaml").is_file();
    let checks_loaded = mimir_review::checks::load_checks(&state.workspace_root).len();
    let sessions_count = state.sessions.list().map_err(UiError::internal)?.len();
    let providers = provider_statuses();
    let recent_runs = list_local_runs(&state.workspace_root)?;
    let runs_count = recent_runs.len();
    Ok(WorkspaceStatus {
        workspace_name: workspace_display_name(&state.workspace_root),
        git,
        mimir: MimirStatus {
            initialized: state.workspace_root.join(".mimir").is_dir(),
            config_present,
            checks_loaded,
            sessions_count,
            runs_count,
            recent_runs,
        },
        providers,
        commands: builtin_command_registry().to_vec(),
    })
}

fn context_why_for_session(
    state: &UiState,
    session: &SessionRecord,
    lookup_path: &str,
) -> Result<serde_json::Value, UiError> {
    let events = session.read_events().map_err(UiError::internal)?;
    let Some((run_id, packet_id, packet_path)) = latest_context_packet_ref(&events) else {
        return Err(UiError::bad_request(
            "no context packet is available; run /context <task> first",
        ));
    };

    let packet_path = if packet_path.is_absolute() {
        canonical_utf8(&packet_path).map_err(UiError::internal)?
    } else {
        canonical_utf8(&state.workspace_root.join(packet_path)).map_err(UiError::internal)?
    };
    if !packet_path.starts_with(&state.workspace_root) {
        return Err(UiError::forbidden("context packet path escapes workspace"));
    }

    let packet_bytes = fs::read(&packet_path).map_err(UiError::internal)?;
    let packet: ContextPacket = serde_json::from_slice(&packet_bytes).map_err(UiError::internal)?;
    let source_hash = packet
        .included
        .iter()
        .find(|item| item.path == lookup_path)
        .map(|item| item.source_hash.clone());

    let base = json!({
        "path": lookup_path,
        "run_id": run_id,
        "packet_id": packet_id,
        "packet_hash": packet.packet_hash.clone(),
        "packet_path": path_to_workspace_string(&state.workspace_root, packet_path.as_std_path())?,
        "source_hash": source_hash,
    });

    let mut result = base.as_object().cloned().unwrap_or_default();
    match mimir_context::context_why_packet(&packet, lookup_path) {
        WhyResult::Included {
            reason_code,
            token_count,
        } => {
            result.insert("status".to_string(), json!("included"));
            result.insert("reason_code".to_string(), json!(reason_code));
            result.insert(
                "reason".to_string(),
                json!("included in the context packet"),
            );
            result.insert("token_count".to_string(), json!(token_count));
        }
        WhyResult::Omitted {
            reason,
            token_count,
        } => {
            result.insert("status".to_string(), json!("omitted"));
            result.insert("reason".to_string(), json!(reason));
            result.insert("token_count".to_string(), json!(token_count));
        }
        WhyResult::NotFound => {
            result.insert("status".to_string(), json!("not_found"));
            result.insert(
                "reason".to_string(),
                json!("path was not present in the latest context packet"),
            );
        }
    }

    Ok(serde_json::Value::Object(result))
}

fn latest_context_packet_ref(events: &[SessionEvent]) -> Option<(String, String, Utf8PathBuf)> {
    events.iter().rev().find_map(|event| {
        if let SessionEventKind::ContextPacketReady {
            run_id,
            packet_id,
            packet_path,
            ..
        } = &event.kind
        {
            Some((run_id.clone(), packet_id.clone(), packet_path.clone()))
        } else {
            None
        }
    })
}

fn list_local_runs(workspace_root: &Utf8Path) -> Result<Vec<RunSummary>, UiError> {
    let runs_root = workspace_root.join(".mimir/runs");
    if !runs_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut runs = Vec::new();
    let entries = fs::read_dir(&runs_root).map_err(UiError::internal)?;
    for entry in entries {
        let entry = entry.map_err(UiError::internal)?;
        let file_type = entry.file_type().map_err(UiError::internal)?;
        if !file_type.is_dir() {
            continue;
        }
        let run_id = entry.file_name().to_string_lossy().to_string();
        if !RunId::is_valid(&run_id) {
            continue;
        }
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|path| UiError::internal(format!("path is not UTF-8: {}", path.display())))?;
        let artifact_count = ui_previewable_artifact_count(workspace_root, &path)?;
        let has_context_packet = path.join("context_packet.json").is_file();
        runs.push(RunSummary {
            run_id,
            path: path_to_workspace_string(workspace_root, path.as_std_path())?,
            artifact_count,
            has_context_packet,
            trace_status: trace_status_for_run(&path),
        });
    }
    runs.sort_by(|left, right| right.run_id.cmp(&left.run_id));
    runs.truncate(50);
    Ok(runs)
}

fn trace_status_for_run(run_dir: &Utf8Path) -> TraceStatus {
    for name in TRACE_STATUS_ARTIFACTS {
        let path = run_dir.join(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return TraceStatus {
                    state: TraceStatusState::Unavailable,
                    redacted: false,
                };
            }
            Ok(metadata) if metadata.is_file() => {
                return TraceStatus {
                    state: TraceStatusState::Recorded,
                    redacted: true,
                };
            }
            Ok(_) => {
                return TraceStatus {
                    state: TraceStatusState::Unavailable,
                    redacted: false,
                };
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return TraceStatus {
                    state: TraceStatusState::Unavailable,
                    redacted: false,
                };
            }
        }
    }

    TraceStatus {
        state: TraceStatusState::Absent,
        redacted: false,
    }
}

fn git_status(workspace_root: &Utf8Path) -> GitStatus {
    let is_repo = git_output(workspace_root, ["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|value| value == "true");
    let branch = git_output(workspace_root, ["rev-parse", "--abbrev-ref", "HEAD"]);
    let dirty = git_output(workspace_root, ["status", "--porcelain"])
        .is_some_and(|value| !value.trim().is_empty());
    GitStatus {
        is_repo,
        branch,
        dirty,
    }
}

fn git_output<const N: usize>(workspace_root: &Utf8Path, args: [&str; N]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn provider_statuses() -> Vec<ProviderStatus> {
    let credentials = credential_provider_names();
    mimir_providers::capabilities::provider_capabilities_list()
        .map(|list| {
            list.providers
                .into_iter()
                .map(|provider| ProviderStatus {
                    credential_detected: credentials.contains(provider.provider.as_str()),
                    models_count: provider.models.len(),
                    provider: provider.provider,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn credential_provider_names() -> std::collections::BTreeSet<&'static str> {
    [
        ("ANTHROPIC_API_KEY", "anthropic"),
        ("GLM_API_KEY", "glm"),
        ("ZAI_API_KEY", "glm"),
        ("OPENAI_API_KEY", "openai"),
    ]
    .into_iter()
    .filter_map(|(env_var, provider)| {
        std::env::var(env_var)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|_| provider)
    })
    .collect()
}

fn search_workspace_files(
    workspace_root: &Utf8Path,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<WorkspaceFileMatch>> {
    let query = query.unwrap_or_default().trim().to_lowercase();
    let mut results = Vec::new();
    let mut walker = WalkBuilder::new(workspace_root);
    walker.hidden(false).standard_filters(true);

    for entry in walker.build().filter_map(std::result::Result::ok) {
        if results.len() >= limit {
            break;
        }
        let path = entry.path();
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
            || is_ignored_ui_path(workspace_root, path)
        {
            continue;
        }
        let Ok(rel) = path_to_workspace_string(workspace_root, path) else {
            continue;
        };
        let rel_lower = rel.to_lowercase();
        let path_matches = query.is_empty() || rel_lower.contains(&query);
        if path_matches {
            results.push(WorkspaceFileMatch {
                path: rel.clone(),
                kind: "file".to_string(),
                line: None,
                symbol: None,
            });
        }

        if results.len() >= limit || query.is_empty() {
            continue;
        }
        append_symbol_matches(path, &rel, &query, limit, &mut results)?;
    }

    Ok(results)
}

fn append_symbol_matches(
    path: &Path,
    rel: &str,
    query: &str,
    limit: usize,
    results: &mut Vec<WorkspaceFileMatch>,
) -> Result<()> {
    if !is_source_file(path) || fs::metadata(path)?.len() > MAX_SYMBOL_SCAN_BYTES {
        return Ok(());
    }
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return Ok(()),
    };
    for (index, line) in content.lines().enumerate() {
        if results.len() >= limit {
            break;
        }
        let Some(symbol) = parse_symbol_name(line) else {
            continue;
        };
        if symbol.to_lowercase().contains(query) {
            results.push(WorkspaceFileMatch {
                path: rel.to_string(),
                kind: "symbol".to_string(),
                line: Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
                symbol: Some(symbol),
            });
        }
    }
    Ok(())
}

fn parse_symbol_name(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let prefixes = [
        "pub fn ",
        "fn ",
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
        "function ",
        "export function ",
        "class ",
        "export class ",
        "def ",
    ];
    for prefix in prefixes {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(
                rest.split(|ch: char| {
                    !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '$')
                })
                .next()
                .unwrap_or_default()
                .to_string(),
            )
            .filter(|symbol| !symbol.is_empty());
        }
    }
    None
}

fn is_source_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rs" | "ts" | "tsx" | "js" | "jsx" | "py")
    )
}

fn is_ignored_ui_path(workspace_root: &Utf8Path, path: &Path) -> bool {
    path.strip_prefix(workspace_root.as_std_path())
        .ok()
        .is_some_and(|rel| {
            rel.components().any(|component| {
                let name = component.as_os_str().to_string_lossy();
                matches!(name.as_ref(), ".git" | ".mimir" | "target" | "node_modules")
            })
        })
}

fn safe_run_dir(workspace_root: &Utf8Path, run_id: &str) -> Result<Utf8PathBuf, UiError> {
    let run_id =
        RunId::parse(run_id.to_string()).map_err(|_| UiError::bad_request("invalid run id"))?;
    let mimir_root = workspace_root.join(".mimir");
    let run_dir = RunDir::open(&mimir_root, &run_id).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => UiError::not_found("run not found"),
        std::io::ErrorKind::InvalidInput => UiError::bad_request("invalid run id"),
        std::io::ErrorKind::PermissionDenied => UiError::forbidden("run path escapes workspace"),
        _ => UiError::internal("run directory could not be opened"),
    })?;
    Ok(run_dir.root().clone())
}

fn safe_child_file(parent: &Utf8Path, name: &str) -> Result<Utf8PathBuf, UiError> {
    if !is_safe_segment(name) {
        return Err(UiError::bad_request("invalid file name"));
    }
    let child = parent.join(name);
    if !child.is_file() {
        return Err(UiError::not_found("artifact not found"));
    }
    let canonical_parent = canonical_utf8(parent).map_err(UiError::internal)?;
    let canonical_child = canonical_utf8(&child).map_err(UiError::internal)?;
    if !canonical_child.starts_with(&canonical_parent) {
        return Err(UiError::forbidden("artifact path escapes run directory"));
    }
    Ok(canonical_child)
}

fn redacted_artifact_content(
    workspace_root: &Utf8Path,
    bytes: &[u8],
) -> Result<(String, serde_json::Value, Vec<u8>), UiError> {
    if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(bytes) {
        mimir_security::redact_json_value(&mut json);
        sanitize_ui_json_value_for_workspace(workspace_root, &mut json);
        let redacted_bytes = serde_json::to_vec_pretty(&json).map_err(UiError::internal)?;
        return Ok(("application/json".to_string(), json, redacted_bytes));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        UiError::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "binary artifact preview is not supported",
        )
    })?;
    let redacted = redacted_json_lines(workspace_root, text)
        .unwrap_or_else(|| sanitize_ui_string_for_workspace(workspace_root, text));
    let redacted_bytes = redacted.as_bytes().to_vec();
    Ok((
        "text/plain; charset=utf-8".to_string(),
        serde_json::Value::String(redacted),
        redacted_bytes,
    ))
}

fn redacted_json_lines(workspace_root: &Utf8Path, text: &str) -> Option<String> {
    let mut lines = Vec::new();
    let mut parsed_any = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut value = serde_json::from_str::<Value>(line).ok()?;
        mimir_security::redact_json_value(&mut value);
        sanitize_ui_json_value_for_workspace(workspace_root, &mut value);
        lines.push(serde_json::to_string(&value).ok()?);
        parsed_any = true;
    }

    if !parsed_any {
        return None;
    }

    let mut redacted = lines.join("\n");
    if text.ends_with('\n') {
        redacted.push('\n');
    }
    Some(redacted)
}

fn artifact_name_has_secret_like_text(name: &str) -> bool {
    mimir_security::redact_secrets(name) != name
}

fn ui_previewable_artifact_count(
    workspace_root: &Utf8Path,
    run_dir: &Utf8Path,
) -> Result<usize, UiError> {
    let entries = fs::read_dir(run_dir).map_err(UiError::internal)?;
    let mut count = 0;
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_safe_segment(name) || artifact_name_has_secret_like_text(name) {
            continue;
        }
        let artifact_path = safe_child_file(run_dir, name)?;
        let metadata = fs::metadata(&artifact_path).map_err(UiError::internal)?;
        if metadata.len() > MAX_ARTIFACT_BYTES {
            continue;
        }
        let bytes = fs::read(&artifact_path).map_err(UiError::internal)?;
        if redacted_artifact_content(workspace_root, &bytes)
            .is_ok_and(|(_, _, redacted_bytes)| redacted_bytes.len() as u64 <= MAX_ARTIFACT_BYTES)
        {
            count += 1;
        }
    }
    Ok(count)
}

fn packet_ui_error(error: anyhow::Error) -> UiError {
    tracing::warn!("packet preview failed: {error:#}");
    let message = mimir_security::redact_secrets(&error.to_string());
    if message.contains("invalid run id") {
        UiError::bad_request("invalid run id")
    } else if message.contains("run not found") || message.contains("No packet found") {
        UiError::not_found("run not found")
    } else if message.contains("escapes workspace") {
        UiError::forbidden("run path is not allowed")
    } else {
        UiError::bad_request("packet replay/share preview is unavailable")
    }
}

fn ui_session_metadata(metadata: &SessionMetadata) -> UiSessionMetadata {
    UiSessionMetadata {
        schema_version: metadata.schema_version,
        session_id: metadata.session_id.clone(),
        title: mimir_security::redact_secrets(&metadata.title),
        workspace_name: workspace_display_name(&metadata.workspace_root),
        created_at: metadata.created_at.to_rfc3339(),
        updated_at: metadata.updated_at.to_rfc3339(),
    }
}

fn ui_session_events(state: &UiState, events: &[SessionEvent]) -> Result<Vec<Value>, UiError> {
    events
        .iter()
        .map(|event| ui_session_event(state, event))
        .collect()
}

fn ui_session_event(state: &UiState, event: &SessionEvent) -> Result<Value, UiError> {
    let mut value = serde_json::to_value(event).map_err(UiError::internal)?;
    sanitize_ui_json_value(state, &mut value);
    Ok(value)
}

fn sanitize_ui_json_value(state: &UiState, value: &mut Value) {
    sanitize_ui_json_value_for_workspace(&state.workspace_root, value);
}

fn sanitize_ui_json_value_for_workspace(workspace_root: &Utf8Path, value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map.remove("workspace_root").is_some() {
                map.insert(
                    "workspace_name".to_string(),
                    Value::String(workspace_display_name(workspace_root)),
                );
            }

            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let Some(child) = map.get_mut(&key) else {
                    continue;
                };
                if is_path_field(&key) {
                    if let Some(path) = child.as_str() {
                        *child = Value::String(ui_display_path_string_for_workspace(
                            workspace_root,
                            path,
                        ));
                        continue;
                    }
                }
                sanitize_ui_json_value_for_workspace(workspace_root, child);
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_ui_json_value_for_workspace(workspace_root, item);
            }
        }
        Value::String(text) => {
            let sanitized = sanitize_ui_string_for_workspace(workspace_root, text);
            if sanitized != *text {
                *text = sanitized;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_path_field(key: &str) -> bool {
    matches!(
        key,
        "path" | "packet_path" | "evidence_path" | "artifact_path" | "source_path"
    )
}

fn ui_display_path_string_for_workspace(workspace_root: &Utf8Path, value: &str) -> String {
    let redacted = mimir_security::redact_secrets(value);
    let candidate = Path::new(&redacted);
    if candidate.is_absolute() {
        if let Ok(relative) = candidate.strip_prefix(workspace_root.as_std_path()) {
            let relative = relative.to_string_lossy().replace('\\', "/");
            return if relative.is_empty() {
                ".".to_string()
            } else {
                relative
            };
        }
        return candidate
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("[external]/{name}"))
            .unwrap_or_else(|| "[external path]".to_string());
    }
    sanitize_ui_string_for_workspace(workspace_root, &redacted)
}

fn sanitize_ui_string_for_workspace(workspace_root: &Utf8Path, value: &str) -> String {
    mimir_security::redact_secrets(value).replace(
        workspace_root.as_str(),
        &format!("<workspace:{}>", workspace_display_name(workspace_root)),
    )
}

fn open_session(state: &UiState, session_id: &str) -> Result<SessionRecord, UiError> {
    let session_id = SessionId::parse(session_id.to_string())
        .map_err(|_| UiError::bad_request("invalid session id"))?;
    state
        .sessions
        .open(&session_id)
        .map_err(|_| UiError::not_found("session not found"))
}

fn requested_workspace_root(
    state: &UiState,
    requested: Option<&str>,
) -> Result<Utf8PathBuf, UiError> {
    let Some(requested) = requested else {
        return Ok(state.workspace_root.clone());
    };
    let requested = canonical_utf8(Utf8Path::new(requested)).map_err(UiError::internal)?;
    if requested != state.workspace_root {
        return Err(UiError::forbidden(
            "UI sessions are restricted to the server workspace root",
        ));
    }
    Ok(requested)
}

fn authorize(
    headers: &HeaderMap,
    token_query: Option<&str>,
    state: &UiState,
) -> Result<(), UiError> {
    let header_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .or_else(|| {
            headers
                .get("x-mimir-token")
                .and_then(|value| value.to_str().ok())
        });
    let token = header_token.or(token_query);
    if token == Some(state.token.as_str()) {
        Ok(())
    } else {
        Err(UiError::unauthorized())
    }
}

fn authorize_websocket(headers: &HeaderMap, state: &UiState) -> Result<(), UiError> {
    if websocket_protocol_authorized(headers, &state.token) {
        Ok(())
    } else {
        Err(UiError::unauthorized())
    }
}

fn websocket_protocol_authorized(headers: &HeaderMap, token: &str) -> bool {
    let expected = websocket_token_protocol(token);
    headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|protocols| {
            protocols
                .split(',')
                .map(str::trim)
                .any(|protocol| protocol == expected)
        })
}

fn websocket_token_protocol(token: &str) -> String {
    let encoded = token
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{MIMIR_WS_TOKEN_PROTOCOL_PREFIX}{encoded}")
}

fn check_origin(headers: &HeaderMap, state: &UiState) -> Result<(), UiError> {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };
    if state.allowed_origins.contains(origin) {
        Ok(())
    } else {
        Err(UiError::forbidden(
            "browser origin is not allowed for this Mimir Studio instance",
        ))
    }
}

fn allowed_ui_origins(base_url: &str) -> BTreeSet<String> {
    let mut origins = BTreeSet::from([base_url.trim_end_matches('/').to_string()]);
    origins.extend(
        ALLOWED_VITE_DEV_ORIGINS
            .iter()
            .map(|origin| origin.to_string()),
    );
    origins
}

fn add_cors_headers(headers: &mut HeaderMap, request_headers: &HeaderMap, state: &UiState) {
    if let Some(origin) = request_headers.get(header::ORIGIN).filter(|origin| {
        origin
            .to_str()
            .is_ok_and(|value| state.allowed_origins.contains(value))
    }) {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET,POST,OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization,content-type,x-mimir-token"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
}

fn discover_studio_assets_root(workspace_root: &Utf8Path) -> Option<Utf8PathBuf> {
    std::env::var("MIMIR_STUDIO_DIST")
        .ok()
        .and_then(|path| Utf8PathBuf::from_path_buf(Path::new(&path).to_path_buf()).ok())
        .filter(|path| is_studio_assets_root(path))
        .or_else(|| {
            let workspace_dist = workspace_root.join("apps/studio/dist");
            is_studio_assets_root(&workspace_dist).then_some(workspace_dist)
        })
}

fn is_studio_assets_root(path: &Utf8Path) -> bool {
    path.join("index.html").is_file()
}

fn safe_static_asset_path(root: &Utf8Path, requested: &str) -> Result<Utf8PathBuf, UiError> {
    if requested.is_empty() || requested.contains('\0') {
        return Err(UiError::bad_request("invalid asset path"));
    }

    let mut path = root.to_path_buf();
    for component in Path::new(requested).components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment
                    .to_str()
                    .ok_or_else(|| UiError::bad_request("asset path is not UTF-8"))?;
                path.push(segment);
            }
            _ => return Err(UiError::bad_request("invalid asset path")),
        }
    }

    if !path.is_file() {
        return Err(UiError::not_found("asset not found"));
    }

    let canonical_root = canonical_utf8(root).map_err(UiError::internal)?;
    let canonical_path = canonical_utf8(&path).map_err(UiError::internal)?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(UiError::forbidden("asset path escapes Studio assets"));
    }
    Ok(canonical_path)
}

fn static_file_response(path: &Utf8Path, content_type: &'static str) -> Result<Response, UiError> {
    let bytes = fs::read(path).map_err(UiError::internal)?;
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

fn static_content_type(path: &Utf8Path) -> &'static str {
    match path.extension().unwrap_or_default() {
        "css" => "text/css; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "wasm" => "application/wasm",
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

fn fallback_studio_html() -> &'static str {
    r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Mimir Studio</title>
    <style>
      :root { color-scheme: dark; font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #111315; color: #eef0f2; }
      body { margin: 0; min-height: 100vh; display: grid; place-items: center; }
      main { width: min(720px, calc(100vw - 40px)); border: 1px solid #2a3036; border-radius: 8px; padding: 24px; background: #171a1e; }
      h1 { margin: 0 0 10px; font-size: 1.15rem; letter-spacing: 0; }
      p { margin: 0 0 12px; color: #b5bcc5; line-height: 1.5; }
      code { color: #d9edf8; }
    </style>
  </head>
  <body>
    <main>
      <h1>Mimir Studio assets are not built</h1>
      <p>The local API is running on loopback, but this binary could not find <code>apps/studio/dist/index.html</code>.</p>
      <p>Build the Studio bundle with <code>cd apps/studio && pnpm build</code>, or set <code>MIMIR_STUDIO_DIST</code> to a built Studio directory before launching.</p>
    </main>
  </body>
</html>"#
}

fn is_safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('\0')
}

fn canonical_utf8(path: &Utf8Path) -> Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(
        fs::canonicalize(path).with_context(|| format!("failed to canonicalize {}", path))?,
    )
    .map_err(|path| anyhow!("path is not UTF-8: {}", path.display()))
}

fn path_to_workspace_string(workspace_root: &Utf8Path, path: &Path) -> Result<String, UiError> {
    let rel = path
        .strip_prefix(workspace_root.as_std_path())
        .map_err(|_| UiError::forbidden("path escapes workspace"))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

fn workspace_display_name(workspace_root: &Utf8Path) -> String {
    workspace_root
        .file_name()
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace")
        .to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn generate_ui_token() -> String {
    format!("ui-{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
