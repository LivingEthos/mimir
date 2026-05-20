//! Integration tests for the Mimir JSON-RPC server.

use mimir_server::{run_server, MimirLspBackend, MimirServer, ServerConfig, SessionStore};
use std::time::Duration;
use tower_lsp::lsp_types::{ExecuteCommandParams, WorkDoneProgressParams};
use tower_lsp::{LanguageServer, LspService};

/// Test that the server responds to a TCP connection with a valid JSON-RPC message.
#[tokio::test]
async fn test_server_tcp_accepts_connection() {
    let config = ServerConfig {
        tcp_bind: Some("127.0.0.1:0".to_string()),
        stdio: false,
    };

    // Start server in background
    let server_handle = tokio::spawn(async move {
        run_server(config).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The server binds to port 0 (random), but we can't easily know the port.
    // For this test, we just verify the server task is running.
    assert!(!server_handle.is_finished());
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
