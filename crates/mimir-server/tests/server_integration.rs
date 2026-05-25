//! Integration tests for the Mimir JSON-RPC server.

use mimir_server::{run_tcp_server, MimirLspBackend, MimirServer, ServerConfig, SessionStore};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower_lsp::lsp_types::{ExecuteCommandParams, InitializeParams, Url, WorkDoneProgressParams};
use tower_lsp::{LanguageServer, LspService};

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
}
