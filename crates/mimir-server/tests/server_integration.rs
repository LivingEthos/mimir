//! Integration tests for the Mimir JSON-RPC server.

use futures::StreamExt as _;
use mimir_server::{
    run_tcp_server, run_ui_server, MimirLspBackend, MimirServer, ServerConfig, SessionStore,
    UiServerConfig,
};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{ORIGIN as WS_ORIGIN, SEC_WEBSOCKET_PROTOCOL};
use tower_lsp::lsp_types::{ExecuteCommandParams, InitializeParams, Url, WorkDoneProgressParams};
use tower_lsp::{LanguageServer, LspService};

const MIMIR_STUDIO_WS_PROTOCOL: &str = "mimir.studio.v1";

async fn write_lsp_message(stream: &mut TcpStream, message: serde_json::Value) {
    let body = message.to_string();
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    stream.write_all(frame.as_bytes()).await.unwrap();
}

async fn read_lsp_message(stream: &mut TcpStream) -> serde_json::Value {
    let mut header = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut byte))
            .await
            .unwrap()
            .unwrap();
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header = std::str::from_utf8(&header).unwrap();
    let len = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut body = vec![0u8; len];
    tokio::time::timeout(Duration::from_secs(2), stream.read_exact(&mut body))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn read_response_by_id(stream: &mut TcpStream, id: u64) -> serde_json::Value {
    loop {
        let message = read_lsp_message(stream).await;
        if message["id"].as_u64() == Some(id) {
            return message;
        }
    }
}

async fn spawn_ui_server(
    dir: &tempfile::TempDir,
) -> (String, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let workspace_root =
        camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 temp dir");
    let config = UiServerConfig::new(workspace_root, Some("test-token".to_string()));
    let handle = tokio::spawn(async move { run_ui_server(listener, config).await });
    (format!("http://{addr}"), handle)
}

async fn spawn_ui_server_with_assets(
    dir: &tempfile::TempDir,
    assets_root: camino::Utf8PathBuf,
) -> (String, tokio::task::JoinHandle<anyhow::Result<()>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let workspace_root =
        camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 temp dir");
    let config = UiServerConfig::new(workspace_root, Some("test-token".to_string()))
        .with_studio_assets_root(assets_root);
    let handle = tokio::spawn(async move { run_ui_server(listener, config).await });
    (format!("http://{addr}"), handle)
}

fn workspace_prefixes(dir: &tempfile::TempDir) -> Vec<String> {
    let mut prefixes = vec![dir.path().to_string_lossy().to_string()];
    if let Ok(canonical) = std::fs::canonicalize(dir.path()) {
        let canonical = canonical.to_string_lossy().to_string();
        if !prefixes.contains(&canonical) {
            prefixes.push(canonical);
        }
    }
    prefixes
}

fn assert_text_omits_workspace_prefix(text: &str, dir: &tempfile::TempDir) {
    for prefix in workspace_prefixes(dir) {
        assert!(
            !text.contains(&prefix),
            "UI payload leaked workspace prefix {prefix}: {text}"
        );
    }
}

fn assert_json_omits_workspace_prefix(value: &serde_json::Value, dir: &tempfile::TempDir) {
    assert_text_omits_workspace_prefix(&serde_json::to_string(value).unwrap(), dir);
}

fn assert_text_omits_needles(text: &str, needles: &[String]) {
    for (index, needle) in needles.iter().enumerate() {
        assert!(
            !text.contains(needle),
            "public response leaked private preview context #{index}: {text}"
        );
    }
}

fn websocket_event_request(
    base_url: &str,
    session_id: &str,
    after: u64,
    token: Option<&str>,
    origin: Option<&str>,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let ws_url = format!(
        "{}/v1/sessions/{session_id}/events?after={after}",
        base_url.replace("http://", "ws://")
    );
    let mut request = ws_url.into_client_request().unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            SEC_WEBSOCKET_PROTOCOL,
            format!(
                "{MIMIR_STUDIO_WS_PROTOCOL}, {}",
                websocket_token_protocol(token)
            )
            .parse()
            .unwrap(),
        );
    }
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert(WS_ORIGIN, origin.parse().unwrap());
    }
    request
}

fn websocket_token_protocol(token: &str) -> String {
    let encoded = token
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("mimir-token.{encoded}")
}

fn write_replayable_run(dir: &tempfile::TempDir, run_id: &str) -> serde_json::Value {
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/replay_target.rs"),
        "pub fn replay_target_unique() -> &'static str { \"stable\" }\n",
    )
    .unwrap();
    let packet = mimir_context::ContextBuilder::new()
        .run_id(mimir_runs::RunId(run_id.to_string()))
        .task_card("replay_target_unique")
        .provider("glm")
        .model("glm-5.1")
        .repo_root(dir.path())
        .build()
        .unwrap();
    assert!(
        packet
            .included
            .iter()
            .any(|item| item.path == "src/replay_target.rs"),
        "test packet should include replay_target.rs"
    );
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("context_packet.json"),
        serde_json::to_vec_pretty(&packet).unwrap(),
    )
    .unwrap();
    let request = serde_json::json!({
        "model": "glm-5.1",
        "messages": [
            {"role": "user", "content": "redacted replay request for replay_target_unique"}
        ],
        "stream": false
    });
    std::fs::write(
        run_dir.join("provider_request.redacted.json"),
        serde_json::to_vec_pretty(&request).unwrap(),
    )
    .unwrap();
    request
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

async fn tcp_client_provider_roundtrip(addr: std::net::SocketAddr, id_base: u64) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    write_lsp_message(
        &mut stream,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id_base,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": null,
                "capabilities": {}
            }
        }),
    )
    .await;
    let initialize = read_response_by_id(&mut stream, id_base).await;
    assert_eq!(initialize["result"]["serverInfo"]["name"], "mimir-server");

    write_lsp_message(
        &mut stream,
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }),
    )
    .await;
    write_lsp_message(
        &mut stream,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id_base + 1,
            "method": "workspace/executeCommand",
            "params": {
                "command": mimir_server::rpc::WORKSPACE_PROVIDERS,
                "arguments": []
            }
        }),
    )
    .await;
    let providers = read_response_by_id(&mut stream, id_base + 1).await;
    assert!(providers["result"]["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|provider| provider["provider"] == "anthropic"));

    write_lsp_message(
        &mut stream,
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id_base + 2,
            "method": "shutdown"
        }),
    )
    .await;
    let shutdown = read_response_by_id(&mut stream, id_base + 2).await;
    assert!(shutdown.get("error").is_none(), "{shutdown}");
}

/// Test a complete framed TCP lifecycle with a real LSP command.
#[tokio::test]
async fn tcp_lsp_framed_lifecycle_execute_command_shutdown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle =
        tokio::spawn(async move { run_tcp_server(listener, SessionStore::new()).await.unwrap() });

    tcp_client_provider_roundtrip(addr, 1).await;

    server_handle.abort();
}

#[tokio::test]
async fn tcp_lsp_accepts_sequential_and_concurrent_clients() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_handle =
        tokio::spawn(async move { run_tcp_server(listener, SessionStore::new()).await.unwrap() });

    tcp_client_provider_roundtrip(addr, 10).await;
    tcp_client_provider_roundtrip(addr, 20).await;

    tokio::join!(
        tcp_client_provider_roundtrip(addr, 30),
        tcp_client_provider_roundtrip(addr, 40)
    );

    server_handle.abort();
}

/// Test SessionStore operations.
#[test]
fn test_session_store_create_and_get() {
    let store = SessionStore::new();
    let id = store.create();
    let session = store.get(&id).unwrap();
    assert_eq!(session.id.0, id.0);
    assert!(session.provider.is_none());
    assert!(session.model.is_none());
}

#[test]
fn workspace_providers_lists_registry_and_dynamic_capabilities() {
    let server = MimirServer::new(SessionStore::new());
    let caps = server.handle_providers().unwrap();
    let anthropic = caps
        .providers
        .iter()
        .find(|provider| provider.provider == "anthropic")
        .unwrap();
    let model = anthropic.models.get("claude-sonnet-4-20250514").unwrap();
    assert_eq!(model.max_input_tokens, 935_488);
    assert_eq!(model.output_reserve_tokens, 64_000);
    assert_eq!(model.pricing.input_per_million, 3.0);
    assert!(caps
        .providers
        .iter()
        .any(|provider| provider.provider == "glm"));
    assert!(caps
        .providers
        .iter()
        .any(|provider| provider.provider == "openai"));
    assert!(caps
        .providers
        .iter()
        .any(|provider| provider.provider == "openai-compatible"));

    let wire = serde_json::to_value(&caps).unwrap();
    assert_eq!(wire["schema_version"], 1);
    assert!(wire["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|provider| { provider["provider"] == "glm" }));
    assert!(wire["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|provider| { provider["provider"] == "openai" }));
}

#[tokio::test]
async fn lsp_execute_command_workspace_providers_returns_plural_wire_shape() {
    let (service, _socket) =
        LspService::new(|client| MimirLspBackend::new(client, SessionStore::new()));
    let value = service
        .inner()
        .execute_command(ExecuteCommandParams {
            command: mimir_server::rpc::WORKSPACE_PROVIDERS.to_string(),
            arguments: Vec::new(),
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(value["schema_version"], 1);
    let providers = value["providers"].as_array().unwrap();
    assert!(providers
        .iter()
        .any(|provider| provider["provider"] == "anthropic"));
    assert!(providers
        .iter()
        .any(|provider| provider["provider"] == "glm"));
    assert!(providers
        .iter()
        .any(|provider| provider["provider"] == "openai"));
    assert!(providers
        .iter()
        .any(|provider| provider["provider"] == "openai-compatible"));
}

#[tokio::test]
async fn lsp_initialize_root_uri_feeds_context_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("server_marker.rs"),
        "pub fn server_marker() -> &'static str { \"synthetic\" }\n",
    )
    .unwrap();

    let (service, _socket) =
        LspService::new(|client| MimirLspBackend::new(client, SessionStore::new()));
    let params = InitializeParams {
        root_uri: Some(Url::from_directory_path(dir.path()).unwrap()),
        ..InitializeParams::default()
    };
    service.inner().initialize(params).await.unwrap();

    let value = service
        .inner()
        .execute_command(ExecuteCommandParams {
            command: mimir_server::rpc::WORKSPACE_CONTEXT_GOVERNOR.to_string(),
            arguments: vec![serde_json::json!({
                "task": "Use server_marker",
                "provider": "anthropic",
                "model": "claude-sonnet-4-20250514"
            })],
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .unwrap();

    let included = value["packet"]["included"].as_array().unwrap();
    assert!(
        included
            .iter()
            .any(|item| item["path"] == "server_marker.rs"),
        "{value}"
    );

    let run_id = value["packet"]["run_id"].as_str().unwrap();
    assert!(mimir_runs::RunId::parse(run_id.to_string()).is_ok());
    let packet_path = dir
        .path()
        .join(".mimir/runs")
        .join(run_id)
        .join("context_packet.json");
    assert!(packet_path.is_file(), "missing {}", packet_path.display());
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&packet_path).unwrap()).unwrap();
    assert_eq!(persisted, value["packet"]);

    let workspace_root = camino::Utf8Path::from_path(dir.path()).unwrap();
    let replay =
        mimir_session::packet::replay_request_preview_for_run(workspace_root, run_id).unwrap();
    assert_eq!(replay.run_id, run_id);
    assert_eq!(replay.packet_hash, value["packet"]["packet_hash"]);
}

#[tokio::test]
async fn lsp_session_build_context_persists_replayable_packet() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("session_marker.rs"),
        "pub fn session_marker() -> &'static str { \"synthetic\" }\n",
    )
    .unwrap();

    let (service, _socket) =
        LspService::new(|client| MimirLspBackend::new(client, SessionStore::new()));
    service
        .inner()
        .initialize(InitializeParams {
            root_uri: Some(Url::from_directory_path(dir.path()).unwrap()),
            ..InitializeParams::default()
        })
        .await
        .unwrap();

    let created = service
        .inner()
        .execute_command(ExecuteCommandParams {
            command: mimir_server::rpc::SESSION_CREATE.to_string(),
            arguments: Vec::new(),
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .unwrap();
    let session_id = created["session_id"].as_str().unwrap();

    let set_provider = service
        .inner()
        .execute_command(ExecuteCommandParams {
            command: mimir_server::rpc::SESSION_SET_PROVIDER.to_string(),
            arguments: vec![serde_json::json!({
                "session_id": session_id,
                "provider": "glm",
                "model": "glm-5.1"
            })],
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(set_provider["ok"], true);

    let value = service
        .inner()
        .execute_command(ExecuteCommandParams {
            command: mimir_server::rpc::SESSION_BUILD_CONTEXT.to_string(),
            arguments: vec![serde_json::json!({
                "session_id": session_id,
                "task": "Use session_marker"
            })],
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .unwrap();

    assert_eq!(value["session_id"], session_id);
    assert_eq!(value["packet"]["provider"], "glm");
    assert_eq!(value["packet"]["model"], "glm-5.1");
    assert!(value["packet"]["included"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["path"] == "session_marker.rs"));

    let run_id = value["packet"]["run_id"].as_str().unwrap();
    assert!(mimir_runs::RunId::parse(run_id.to_string()).is_ok());
    let packet_path = dir
        .path()
        .join(".mimir/runs")
        .join(run_id)
        .join("context_packet.json");
    assert!(packet_path.is_file(), "missing {}", packet_path.display());
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&packet_path).unwrap()).unwrap();
    assert_eq!(persisted, value["packet"]);

    let workspace_root = camino::Utf8Path::from_path(dir.path()).unwrap();
    let share =
        mimir_session::packet::share_bundle_preview_for_run(workspace_root, run_id).unwrap();
    assert_eq!(share.run_id, run_id);
    assert_eq!(share.packet_hash, value["packet"]["packet_hash"]);
}

/// Test SessionStore update and retrieve.
#[test]
fn test_session_store_update() {
    let store = SessionStore::new();
    let id = store.create();
    let mut session = store.get(&id).unwrap();
    session.provider = Some("anthropic".to_string());
    session.model = Some("claude-sonnet-4".to_string());
    store.update(session);

    let updated = store.get(&id).unwrap();
    assert_eq!(updated.provider, Some("anthropic".to_string()));
    assert_eq!(updated.model, Some("claude-sonnet-4".to_string()));
}

/// Test SessionStore purge idle.
#[test]
fn test_session_store_purge_idle() {
    let store = SessionStore::new();
    let id = store.create();
    assert_eq!(store.len(), 1);

    // Purge with zero duration should remove immediately
    let removed = store.purge_idle(Duration::from_secs(0));
    assert_eq!(removed, 1);
    assert!(store.get(&id).is_none());
    assert_eq!(store.len(), 0);
}

/// Test SessionStore list_ids.
#[test]
fn test_session_store_list_ids() {
    let store = SessionStore::new();
    let id1 = store.create();
    let id2 = store.create();
    let ids = store.list_ids();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&id1));
    assert!(ids.contains(&id2));
}

/// Test that ServerConfig defaults are sensible.
#[test]
fn test_server_config_default() {
    let config = ServerConfig::default();
    assert!(config.tcp_bind.is_none());
    assert!(config.stdio);
    assert!(config.ui_bind.is_none());
    assert!(config.ui_token.is_none());
    assert!(config.workspace_root.is_none());
}

#[tokio::test]
async fn ui_api_requires_token_and_runs_status_turn_with_event_replay() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn phase2_marker() {}\n").unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let unauthorized = client
        .get(format!("{base_url}/v1/workspace/status"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let created: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"title": "Phase 2"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = created["metadata"]["session_id"].as_str().unwrap();
    assert!(session_id.starts_with("sess-"));
    assert_eq!(created["events"].as_array().unwrap().len(), 1);

    let message: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"message": "/status"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(message["command"], "status");
    assert_eq!(message["events"].as_array().unwrap().len(), 3);
    assert_eq!(message["result"]["mimir"]["sessions_count"], 1);

    let ws_url = format!(
        "{}/v1/sessions/{}/events?token=test-token&after=0",
        base_url.replace("http://", "ws://"),
        session_id
    );
    assert!(connect_async(ws_url).await.is_err());

    let (mut ws, response) = connect_async(websocket_event_request(
        &base_url,
        session_id,
        0,
        Some("test-token"),
        None,
    ))
    .await
    .unwrap();
    assert_eq!(
        response
            .headers()
            .get(SEC_WEBSOCKET_PROTOCOL)
            .and_then(|value| value.to_str().ok()),
        Some(MIMIR_STUDIO_WS_PROTOCOL)
    );
    let mut event_types = Vec::new();
    while event_types.len() < 3 {
        let message = tokio::time::timeout(Duration::from_secs(2), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            let event: serde_json::Value = serde_json::from_str(&text).unwrap();
            event_types.push(event["type"].as_str().unwrap().to_string());
        }
    }
    assert!(event_types.contains(&"workspace.status.ready".to_string()));
    ws.close(None).await.unwrap();
    handle.abort();
}

#[tokio::test]
async fn ui_api_omits_workspace_prefix_from_session_status_turns_and_ws() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/checks")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join(".mimir/checks/no-secrets.md"),
        "# No Secrets\n\n- severity: error\n- forbidden: sk-[A-Za-z0-9]{24,}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/privacy.rs"),
        "pub fn privacy_marker_unique() -> &'static str { \"privacy\" }\n",
    )
    .unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"title": "Privacy"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_json_omits_workspace_prefix(&created, &dir);
    assert!(created["metadata"].get("workspace_root").is_none());
    assert!(created["metadata"]["workspace_name"].is_string());
    assert!(created["events"][0]["payload"]
        .get("workspace_root")
        .is_none());
    assert!(created["events"][0]["payload"]["workspace_name"].is_string());
    let session_id = created["metadata"]["session_id"].as_str().unwrap();

    let listed: serde_json::Value = client
        .get(format!("{base_url}/v1/sessions"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_json_omits_workspace_prefix(&listed, &dir);
    assert!(listed[0].get("workspace_root").is_none());
    assert!(listed[0]["workspace_name"].is_string());

    let loaded: serde_json::Value = client
        .get(format!("{base_url}/v1/sessions/{session_id}"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_json_omits_workspace_prefix(&loaded, &dir);
    assert!(loaded["metadata"].get("workspace_root").is_none());
    assert!(loaded["metadata"]["workspace_name"].is_string());

    let status: serde_json::Value = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_json_omits_workspace_prefix(&status, &dir);
    assert!(status.get("workspace_root").is_none());
    assert!(status["workspace_name"].is_string());

    for message in [
        "/status",
        "/init",
        "/doctor",
        "/context explain privacy_marker_unique",
        "/explore where is privacy_marker_unique handled?",
        "/runs",
    ] {
        let response: serde_json::Value = client
            .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
            .bearer_auth("test-token")
            .json(&serde_json::json!({ "message": message }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_json_omits_workspace_prefix(&response, &dir);
    }

    let loaded_after_turns: serde_json::Value = client
        .get(format!("{base_url}/v1/sessions/{session_id}"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_json_omits_workspace_prefix(&loaded_after_turns, &dir);
    let event_log = std::fs::read_to_string(
        dir.path()
            .join(".mimir/sessions")
            .join(session_id)
            .join("events.jsonl"),
    )
    .unwrap();
    assert_text_omits_workspace_prefix(&event_log, &dir);

    let (mut socket, _) = connect_async(websocket_event_request(
        &base_url,
        session_id,
        0,
        Some("test-token"),
        None,
    ))
    .await
    .unwrap();
    for _ in 0..6 {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
            assert_text_omits_workspace_prefix(&text, &dir);
        }
    }

    handle.abort();
}

#[tokio::test]
async fn ui_api_exposes_command_registry_and_rejects_unavailable_slash_commands() {
    let dir = tempfile::TempDir::new().unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let commands: serde_json::Value = client
        .get(format!("{base_url}/v1/workspace/commands"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let commands = commands.as_array().unwrap();
    let find_command = |name: &str| {
        commands
            .iter()
            .find(|command| command["name"] == name)
            .unwrap_or_else(|| panic!("missing command metadata for {name}"))
    };
    assert_eq!(find_command("/status")["support"], "backend");
    assert_eq!(find_command("/settings")["support"], "local");
    assert_eq!(find_command("/plan")["support"], "planned");
    assert_eq!(find_command("/share")["support"], "local");
    assert_eq!(find_command("/share")["enabled"], true);

    let status: serde_json::Value = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(status["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|command| command["name"] == "/share" && command["support"] == "local"));

    let created: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"title": "Commands"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = created["metadata"]["session_id"].as_str().unwrap();

    for (message, expected) in [
        (
            "/share run-demo",
            "/share is handled locally by Mimir Studio and is not a backend command",
        ),
        (
            "/not-real",
            "/not-real is not a recognized Mimir Studio command",
        ),
        ("/settings", "/settings is handled locally by Mimir Studio"),
    ] {
        let response = client
            .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
            .bearer_auth("test-token")
            .json(&serde_json::json!({ "message": message }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.text().await.unwrap();
        assert!(body.contains(expected), "{body}");
        assert!(!body.contains("test-token"));
    }

    handle.abort();
}

#[tokio::test]
async fn ui_api_pins_origins_to_served_ui_and_vite_dev() {
    let dir = tempfile::TempDir::new().unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let served_origin = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth("test-token")
        .header("origin", &base_url)
        .send()
        .await
        .unwrap();
    assert!(served_origin.status().is_success());

    let vite_origin = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth("test-token")
        .header("origin", "http://127.0.0.1:5173")
        .send()
        .await
        .unwrap();
    assert!(vite_origin.status().is_success());

    let unrelated_loopback = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth("test-token")
        .header("origin", "http://127.0.0.1:6173")
        .send()
        .await
        .unwrap();
    assert_eq!(unrelated_loopback.status(), StatusCode::FORBIDDEN);

    let non_local = client
        .get(format!("{base_url}/v1/workspace/status"))
        .bearer_auth("test-token")
        .header("origin", "https://evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(non_local.status(), StatusCode::FORBIDDEN);
    handle.abort();
}

#[tokio::test]
async fn ui_api_serves_studio_assets_with_token_gated_index() {
    let dir = tempfile::TempDir::new().unwrap();
    let assets_root = camino::Utf8PathBuf::from_path_buf(dir.path().join("studio-dist")).unwrap();
    std::fs::create_dir_all(assets_root.join("assets")).unwrap();
    std::fs::write(
        assets_root.join("index.html"),
        r#"<!doctype html><script type="module" src="/assets/app.js"></script>"#,
    )
    .unwrap();
    std::fs::write(
        assets_root.join("assets/app.js"),
        "console.log('studio');\n",
    )
    .unwrap();

    let (base_url, handle) = spawn_ui_server_with_assets(&dir, assets_root).await;
    let client = reqwest::Client::new();

    let unauthorized = client.get(format!("{base_url}/")).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    let index = client
        .get(format!("{base_url}/?token=test-token"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(index.contains("/assets/app.js"));
    assert!(!index.contains("ANTHROPIC_API_KEY"));

    let asset = client
        .get(format!("{base_url}/assets/app.js"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(asset, "console.log('studio');\n");

    let traversal = client
        .get(format!("{base_url}/assets/../index.html"))
        .send()
        .await
        .unwrap();
    assert!(!traversal.status().is_success());
    handle.abort();
}

#[tokio::test]
async fn ui_api_runs_init_and_lists_local_runs() {
    let dir = tempfile::TempDir::new().unwrap();
    let run_id = "20260101-120000-abcdef20";
    std::fs::create_dir_all(dir.path().join(".mimir/runs").join(run_id)).unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("context_packet.json"),
        r#"{"ok":true}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ.json"),
        r#"{"ok":true}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("oversized.log"),
        vec![b'x'; 512 * 1024 + 1],
    )
    .unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"title": "Init"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = created["metadata"]["session_id"].as_str().unwrap();

    let init: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"message": "/init"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(init["command"], "init");
    assert!(init["result"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item == ".mimir/config.yaml"));
    assert!(dir.path().join(".mimir/project-rules.md").is_file());

    let second_init: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"message": "/init"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(second_init["result"]["created"]
        .as_array()
        .unwrap()
        .is_empty());

    let runs: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"message": "/runs"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(runs["command"], "runs");
    let run = runs["result"]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|run| run["run_id"] == run_id)
        .unwrap();
    assert_eq!(run["has_context_packet"], true);
    assert_eq!(run["artifact_count"], 1);
    assert_eq!(run["trace_status"]["state"], "absent");
    assert_eq!(run["trace_status"]["redacted"], false);
    handle.abort();
}

#[tokio::test]
async fn ui_api_context_why_uses_latest_packet_without_provider_call() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/why_target.rs"),
        "pub fn why_target_marker() -> &'static str { \"why_target_unique\" }\n",
    )
    .unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let created: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"title": "Why"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = created["metadata"]["session_id"].as_str().unwrap();

    let context: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"message": "/context explain why_target_unique"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(context["command"], "context");
    let ready = context["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["type"] == "context.packet.ready")
        .unwrap();
    assert_eq!(ready["payload"]["packet_hash"].as_str().unwrap().len(), 64);
    let why_path = "src/why_target.rs";

    let why: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"message": format!("/why {why_path}")}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(why["command"], "why");
    assert_eq!(why["result"]["path"], why_path);
    assert_eq!(why["result"]["status"], "not_found");
    assert_eq!(why["result"]["packet_hash"].as_str().unwrap().len(), 64);
    assert!(why["result"]["packet_path"]
        .as_str()
        .unwrap()
        .ends_with("context_packet.json"));
    let why_text = serde_json::to_string(&why).unwrap();
    assert!(!why_text.contains("sk-123456789012345678901234"));
    assert_text_omits_workspace_prefix(&why_text, &dir);
    let event_log = std::fs::read_to_string(
        dir.path()
            .join(".mimir/sessions")
            .join(session_id)
            .join("events.jsonl"),
    )
    .unwrap();
    assert_text_omits_workspace_prefix(&event_log, &dir);
    assert!(event_log.contains(".mimir/runs/"));
    handle.abort();
}

#[tokio::test]
async fn ui_api_serves_files_and_redacted_artifacts_fail_closed() {
    let dir = tempfile::TempDir::new().unwrap();
    let run_id = "20260101-120000-abcdef21";
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/runs").join(run_id)).unwrap();
    std::fs::write(
        dir.path().join("src/phase2.rs"),
        "pub fn phase2_marker() {}\n",
    )
    .unwrap();
    let raw_packet_bytes = br#"{"token":"sk-123456789012345678901234","ok":true}"#.to_vec();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("context_packet.json"),
        &raw_packet_bytes,
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ.json"),
        b"{\"ok\":true}",
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("oversized.log"),
        vec![b'x'; 512 * 1024 + 1],
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("trace.spans.jsonl"),
        b"{\"schema_version\":1,\"span_id\":\"0123456789abcdef\",\"name\":\"mimir.context.build\",\"start_us\":1,\"end_us\":2,\"attrs\":{\"api_key\":\"sk-123456789012345678901234\"}}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("events.jsonl"),
        vec![b'{'; 512 * 1024 + 1],
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        dir.path().join("src/phase2.rs"),
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("linked-source.rs"),
    )
    .unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let files: serde_json::Value = client
        .get(format!(
            "{base_url}/v1/workspace/files?token=test-token&q=phase2"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let results = files["results"].as_array().unwrap();
    assert!(results.iter().any(|item| item["path"] == "src/phase2.rs"));
    assert!(results.iter().any(|item| item["symbol"] == "phase2_marker"));

    let artifacts: serde_json::Value = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts?token=test-token"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(artifacts["trace_status"]["state"], "recorded");
    assert_eq!(artifacts["trace_status"]["redacted"], true);
    assert!(artifacts.get("path").is_none());
    assert_eq!(artifacts["artifacts"].as_array().unwrap().len(), 2);
    assert!(artifacts["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|artifact| artifact["name"] == "context_packet.json"));
    assert!(artifacts["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|artifact| artifact["name"] == "trace.spans.jsonl"));
    let listed = artifacts["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["name"] == "context_packet.json")
        .unwrap();
    assert_eq!(listed["checksum_basis"], "redacted_preview");
    assert_ne!(listed["sha256"], sha256_hex(&raw_packet_bytes));

    let artifact: serde_json::Value = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/artifacts/context_packet.json?token=test-token"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let artifact_text = serde_json::to_string(&artifact).unwrap();
    assert!(!artifact_text.contains("sk-123456789012345678901234"));
    assert_eq!(artifact["content"]["ok"], true);
    assert_eq!(artifact["checksum_basis"], "redacted_preview");
    assert_eq!(artifact["sha256"], listed["sha256"]);

    let traversal = client
        .get(format!(
            "{base_url}/v1/runs/%2e%2e/artifacts?token=test-token"
        ))
        .send()
        .await
        .unwrap();
    assert!(!traversal.status().is_success());
    handle.abort();
}

#[tokio::test]
async fn ui_api_redacts_session_events_before_http_ws_and_disk_exposure() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/checks")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join(".mimir/checks/no-secrets.md"),
        "# No Secrets\n\n- severity: error\n- forbidden: sk-[A-Za-z0-9]{24,}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/secret.txt"),
        "api_key=sk-123456789012345678901234\n",
    )
    .unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();
    let secret = "sk-123456789012345678901234";

    let created: serde_json::Value = client
        .post(format!("{base_url}/v1/sessions"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"title": format!("Redaction {secret}")}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let created_text = serde_json::to_string(&created).unwrap();
    assert!(!created_text.contains(secret));
    assert!(created_text.contains("REDACTED"));
    let session_id = created["metadata"]["session_id"].as_str().unwrap();

    let workspace_root =
        camino::Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).expect("utf8 temp dir");
    let mut session = mimir_session::SessionStore::for_workspace(workspace_root.clone())
        .open(&mimir_session::SessionId::parse(session_id.to_string()).unwrap())
        .unwrap();
    session
        .append_event(mimir_session::SessionEventKind::TurnStarted {
            turn_id: "turn-secret".to_string(),
            command: "context".to_string(),
            task: format!("inspect api_key={secret}"),
        })
        .unwrap();

    let event_log = std::fs::read_to_string(
        dir.path()
            .join(".mimir/sessions")
            .join(session_id)
            .join("events.jsonl"),
    )
    .unwrap();
    assert!(!event_log.contains(secret));

    let loaded_text = client
        .get(format!("{base_url}/v1/sessions/{session_id}"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!loaded_text.contains(secret));
    assert!(loaded_text.contains("REDACTED"));

    let check_text = client
        .post(format!("{base_url}/v1/sessions/{session_id}/messages"))
        .bearer_auth("test-token")
        .json(&serde_json::json!({"message": "/check"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!check_text.contains(secret));
    assert!(check_text.contains("REDACTED"));

    let (mut socket, _) = connect_async(websocket_event_request(
        &base_url,
        session_id,
        0,
        Some("test-token"),
        None,
    ))
    .await
    .unwrap();
    let message = socket.next().await.unwrap().unwrap();
    let message_text = message.into_text().unwrap();
    assert!(!message_text.contains(secret));

    handle.abort();
}

#[tokio::test]
async fn ui_api_serves_redacted_packet_replay_and_share_with_integrity_checks() {
    let dir = tempfile::TempDir::new().unwrap();
    let run_id = "20260101-120000-abcdef01";
    let request = write_replayable_run(&dir, run_id);
    let request_bytes = serde_json::to_vec_pretty(&request).unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let replay: serde_json::Value = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/replay?token=test-token"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(replay["run_id"], run_id);
    assert_eq!(replay["source"], "saved_artifact");
    assert_eq!(replay["redacted"], true);
    assert_eq!(
        replay["provider_request_sha256"],
        sha256_hex(&request_bytes)
    );
    assert_eq!(
        replay["request"]["messages"][0]["content"],
        "redacted replay request for replay_target_unique"
    );

    let share: serde_json::Value = client
        .post(format!("{base_url}/v1/runs/{run_id}/share"))
        .bearer_auth("test-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(share["run_id"], run_id);
    assert_eq!(share["redacted"], true);
    assert_eq!(share["bundle"]["kind"], "mimir.packet_share");
    assert_eq!(
        share["bundle"]["replay"]["provider_request_sha256"],
        sha256_hex(&request_bytes)
    );
    assert_eq!(share["bundle"]["packet"]["run_id"], run_id);
    assert_eq!(share["packet_hash"], share["bundle"]["packet_hash"]);

    let share_text = serde_json::to_string(&share).unwrap();
    assert!(!share_text.contains("sk-123456789012345678901234"));
    assert!(!share_text.contains("test-token"));

    handle.abort();
}

#[tokio::test]
async fn ui_api_packet_replay_fails_closed_for_traversal_tamper_and_secret_artifacts() {
    let dir = tempfile::TempDir::new().unwrap();
    let run_id = "20260101-120000-abcdef02";
    write_replayable_run(&dir, run_id);
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    let traversal = client
        .get(format!("{base_url}/v1/runs/%2e%2e/replay?token=test-token"))
        .send()
        .await
        .unwrap();
    assert!(!traversal.status().is_success());

    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("provider_request.redacted.json"),
        r#"{"api_key":"sk-123456789012345678901234","messages":[]}"#,
    )
    .unwrap();
    let secret_response = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/replay?token=test-token"
        ))
        .send()
        .await
        .unwrap();
    assert!(!secret_response.status().is_success());
    let secret_text = secret_response.text().await.unwrap();
    assert!(!secret_text.contains("sk-123456789012345678901234"));
    assert_text_omits_workspace_prefix(&secret_text, &dir);

    std::fs::write(
        dir.path().join("src/replay_target.rs"),
        "pub fn replay_target_unique() -> &'static str { \"tampered\" }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("provider_request.redacted.json"),
        r#"{"messages":[]}"#,
    )
    .unwrap();
    let tampered = client
        .get(format!(
            "{base_url}/v1/runs/{run_id}/share?token=test-token"
        ))
        .send()
        .await
        .unwrap();
    assert!(!tampered.status().is_success());
    let tampered_text = tampered.text().await.unwrap();
    assert!(tampered_text.contains("packet replay/share preview is unavailable"));
    assert!(!tampered_text.contains("test-token"));
    assert_text_omits_workspace_prefix(&tampered_text, &dir);

    handle.abort();
}

#[tokio::test]
async fn ui_api_packet_preview_errors_are_public_and_redacted() {
    let dir = tempfile::TempDir::new().unwrap();
    let bad_packet_run_id = "20260101-120000-abcdef03";
    let bad_request_run_id = "20260101-120000-abcdef04";
    let missing_packet_run_id = "20260101-120000-abcdef05";
    write_replayable_run(&dir, bad_packet_run_id);
    write_replayable_run(&dir, bad_request_run_id);
    std::fs::create_dir_all(dir.path().join(".mimir/runs").join(missing_packet_run_id)).unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(bad_packet_run_id)
            .join("context_packet.json"),
        "{not valid json",
    )
    .unwrap();
    std::fs::write(
        dir.path()
            .join(".mimir/runs")
            .join(bad_request_run_id)
            .join("provider_request.redacted.json"),
        r#"{"api_key":"sk-123456789012345678901234","messages":["unterminated]"#,
    )
    .unwrap();
    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    for path in [
        format!("/v1/runs/{bad_packet_run_id}/replay?token=test-token"),
        format!("/v1/runs/{bad_packet_run_id}/share?token=test-token"),
        format!("/v1/runs/{bad_request_run_id}/replay?token=test-token"),
        format!("/v1/runs/{bad_request_run_id}/share?token=test-token"),
        format!("/v1/runs/{missing_packet_run_id}/replay?token=test-token"),
        "/v1/runs/20260101-120000-abcdef06/replay?token=test-token".to_string(),
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap();
        assert!(!response.status().is_success());
        let text = response.text().await.unwrap();
        assert!(
            text.contains("packet replay/share preview is unavailable")
                || text.contains("run not found")
        );
        assert!(!text.contains("sk-123456789012345678901234"));
        assert!(!text.contains("test-token"));
        assert_text_omits_workspace_prefix(&text, &dir);
    }

    handle.abort();
}

#[tokio::test]
async fn ui_api_blocked_preview_errors_omit_private_paths_prompts_and_provider_bodies() {
    let dir = tempfile::TempDir::new().unwrap();
    let packet_run_id = "20260101-120000-abcdef31";
    let artifact_run_id = "20260101-120000-abcdef32";
    let secret = "sk-123456789012345678901234";
    let raw_prompt = "RAW_PROMPT_SHOULD_NOT_LEAK_trace_artifact_packet";
    let provider_request_body = "PROVIDER_REQUEST_BODY_SHOULD_NOT_LEAK";
    let provider_response_body = "PROVIDER_RESPONSE_BODY_SHOULD_NOT_LEAK";
    let secret_artifact_name = "ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ.json";

    write_replayable_run(&dir, packet_run_id);
    let packet_run_dir = dir.path().join(".mimir/runs").join(packet_run_id);
    let packet_path = packet_run_dir.join("context_packet.json");
    let packet_path_text = packet_path.to_string_lossy().to_string();
    std::fs::write(
        &packet_path,
        format!(
            r#"{{"packet_path":"{packet_path_text}","raw_prompt":"{raw_prompt}","provider_request":"{provider_request_body}","provider_response":"{provider_response_body}","api_key":"{secret}""#
        ),
    )
    .unwrap();
    std::fs::write(
        packet_run_dir.join("provider_request.redacted.json"),
        format!(
            r#"{{"messages":[{{"role":"user","content":"{raw_prompt}"}}],"provider_request_body":"{provider_request_body}","provider_response_body":"{provider_response_body}","api_key":"{secret}""#
        ),
    )
    .unwrap();

    let artifact_run_dir = dir.path().join(".mimir/runs").join(artifact_run_id);
    std::fs::create_dir_all(&artifact_run_dir).unwrap();
    let trace_path = artifact_run_dir.join("trace.spans.jsonl");
    let oversized_path = artifact_run_dir.join("blocked_preview.log");
    let binary_path = artifact_run_dir.join("blocked_preview.bin");
    std::fs::create_dir_all(&trace_path).unwrap();
    let blocked_artifact_payload = format!(
        "artifact_path={}\nraw_prompt={raw_prompt}\nprovider_request={provider_request_body}\nprovider_response={provider_response_body}\napi_key={secret}\n",
        oversized_path.to_string_lossy()
    );
    let mut oversized = blocked_artifact_payload.as_bytes().to_vec();
    oversized.resize(512 * 1024 + 1, b'x');
    std::fs::write(&oversized_path, oversized).unwrap();
    std::fs::write(
        artifact_run_dir.join(secret_artifact_name),
        &blocked_artifact_payload,
    )
    .unwrap();
    std::fs::write(&binary_path, b"\xff\x00\xfe").unwrap();

    let private_needles = vec![
        secret.to_string(),
        "test-token".to_string(),
        raw_prompt.to_string(),
        provider_request_body.to_string(),
        provider_response_body.to_string(),
        packet_path_text,
        oversized_path.to_string_lossy().to_string(),
        trace_path.to_string_lossy().to_string(),
        binary_path.to_string_lossy().to_string(),
        format!(".mimir/runs/{packet_run_id}/context_packet.json"),
        format!(".mimir/runs/{artifact_run_id}/blocked_preview.log"),
        format!(".mimir/runs/{artifact_run_id}/trace.spans.jsonl"),
        format!(".mimir/runs/{artifact_run_id}/blocked_preview.bin"),
        secret_artifact_name.to_string(),
    ];

    let (base_url, handle) = spawn_ui_server(&dir).await;
    let client = reqwest::Client::new();

    for path in [
        format!("/v1/runs/{packet_run_id}/replay?token=test-token"),
        format!("/v1/runs/{packet_run_id}/share?token=test-token"),
    ] {
        let response = client
            .get(format!("{base_url}{path}"))
            .send()
            .await
            .unwrap();
        assert!(!response.status().is_success());
        let text = response.text().await.unwrap();
        assert!(text.contains("packet replay/share preview is unavailable"));
        assert_text_omits_workspace_prefix(&text, &dir);
        assert_text_omits_needles(&text, &private_needles);
    }

    let artifacts_response = client
        .get(format!(
            "{base_url}/v1/runs/{artifact_run_id}/artifacts?token=test-token"
        ))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let artifacts_text = artifacts_response.text().await.unwrap();
    assert_text_omits_workspace_prefix(&artifacts_text, &dir);
    assert_text_omits_needles(&artifacts_text, &private_needles);
    let artifacts: serde_json::Value = serde_json::from_str(&artifacts_text).unwrap();
    assert_eq!(artifacts["trace_status"]["state"], "unavailable");
    assert_eq!(artifacts["trace_status"]["redacted"], false);
    assert!(artifacts["artifacts"].as_array().unwrap().is_empty());

    for artifact_name in [
        "blocked_preview.log",
        "trace.spans.jsonl",
        "blocked_preview.bin",
        secret_artifact_name,
    ] {
        let response = client
            .get(format!(
                "{base_url}/v1/runs/{artifact_run_id}/artifacts/{artifact_name}?token=test-token"
            ))
            .send()
            .await
            .unwrap();
        assert!(!response.status().is_success());
        let text = response.text().await.unwrap();
        assert_text_omits_workspace_prefix(&text, &dir);
        assert_text_omits_needles(&text, &private_needles);
    }

    handle.abort();
}
