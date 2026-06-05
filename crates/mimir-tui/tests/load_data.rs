//! Integration tests for loading live data into the TUI App.

use mimir_schemas::{ContextPacket, ContextRange, IncludedItem, OmittedCandidate, TaskCard};
use mimir_tui::{
    panels::{IncludedPanel, OmittedPanel},
    server_client::{fetch_live_packet, LiveServerConfig},
    App,
};
use std::time::Duration;

/// Returns a minimal valid ContextPacket for testing serialization.
fn minimal_packet() -> ContextPacket {
    ContextPacket {
        schema_version: 1,
        packet_id: "pkt-test-001".to_string(),
        packet_hash: "0".repeat(64),
        run_id: "20260101-000000-00000001".to_string(),
        task_card: TaskCard {
            goal: "Test task card".to_string(),
            acceptance_criteria: Vec::new(),
            likely_files: Vec::new(),
            risk_level: None,
            expected_test_command: None,
            unknowns: Vec::new(),
            need_for_large_context: None,
            complexity: "tiny".to_string(),
        },
        mode: "ask".to_string(),
        cap_tokens: 64000,
        target_tokens: 32000,
        output_reserve_tokens: 4096,
        count_drift_reserve_tokens: 1024,
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        capability_snapshot_ref: "cap-001".to_string(),
        prompt_contract_version: 1,
        included: vec![],
        omitted_candidates: vec![],
        tool_schemas: vec![],
        evidence_cards: vec![],
        memory_entries: vec![],
        budget_ledger_ref: ".mimir/runs/20260101-000000-00000001/budget_ledger.json".to_string(),
        estimated_input_tokens: 1000,
        count_provenance: "local_estimate_only".to_string(),
        created_at: "2025-01-01T00:00:00Z".to_string(),
        authoritative_input_tokens: None,
        recall_guard_flags: vec![],
    }
}

fn lines_text(lines: Vec<ratatui::text::Line<'static>>) -> String {
    lines
        .into_iter()
        .map(|line| {
            line.spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_load_packet_from_file() {
    let packet = minimal_packet();
    let json = serde_json::to_string_pretty(&packet).unwrap();
    let path = "/tmp/mimir_test_packet.json";
    std::fs::write(path, &json).unwrap();

    let mut app = App::new();
    assert!(app.packet.is_none());
    app.load_from_file(path)
        .expect("load_from_file should succeed");
    assert!(app.packet.is_some());
    let loaded = app.packet.unwrap();
    assert_eq!(loaded.packet_id, "pkt-test-001");
    assert_eq!(loaded.run_id, "20260101-000000-00000001");
    assert!(app.budget.is_some());
    assert_eq!(app.provider_counts.len(), 1);
    assert_eq!(app.provider_counts[0].name, "anthropic");

    // cleanup
    let _ = std::fs::remove_file(path);
}

#[test]
fn test_packet_only_included_and_omitted_panels_use_packet_data() {
    let mut packet = minimal_packet();
    packet.included = vec![IncludedItem {
        path: "src/lib.rs".to_string(),
        ranges: vec![ContextRange { start: 3, end: 8 }],
        candidate_kind: "range".to_string(),
        reason_code: "direct_user_mention".to_string(),
        tokens: 42,
        source_hash: "a".repeat(64),
        trust_level: "trusted".to_string(),
        editable: true,
        compression: None,
    }];
    packet.omitted_candidates = vec![OmittedCandidate {
        schema_version: 1,
        path: "tests/lib_test.rs".to_string(),
        ranges: vec![ContextRange { start: 1, end: 20 }],
        candidate_kind: "test".to_string(),
        reason_code: "budget_limit".to_string(),
        score: 0.55,
        features: serde_json::json!({ "kind": "test" }),
        estimated_tokens: 80,
        discovered_by: vec!["test_map".to_string()],
        source_hash: None,
        reason_for_omission: "budget_limit".to_string(),
        risk: Some("medium".to_string()),
        what_would_trigger_inclusion: "more budget".to_string(),
    }];

    let mut app = App::new();
    app.load_packet(packet);

    let included = lines_text(IncludedPanel::lines(&app));
    assert!(included.contains("Items: 1 | Tokens: 42"));
    assert!(included.contains("src/lib.rs"));
    assert!(included.contains("3-8"));

    let omitted = lines_text(OmittedPanel::lines(&app));
    assert!(omitted.contains("Omitted: 1"));
    assert!(omitted.contains("tests/lib_test.rs"));
    assert!(omitted.contains("budget_limit"));
    assert!(omitted.contains("medium"));
}

#[test]
fn test_refresh_without_live_server_reports_missing_config() {
    let mut app = App::new();
    app.request_live_refresh();
    assert_eq!(app.status, "No live server configured");
}

#[tokio::test]
async fn test_live_server_client_fetches_context_packet() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("tui_live_marker.rs"),
        "pub fn tui_live_marker() -> &'static str { \"synthetic\" }\n",
    )
    .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_handle = tokio::spawn(async move {
        mimir_server::run_tcp_server(listener, mimir_server::SessionStore::new())
            .await
            .unwrap();
    });

    let mut config = LiveServerConfig::new(addr.to_string(), "Use tui_live_marker");
    config.provider = Some("anthropic".to_string());
    config.model = Some("claude-sonnet-4-20250514".to_string());
    config.workspace_root = Some(dir.path().to_path_buf());
    config.timeout = Duration::from_secs(2);

    let live = fetch_live_packet(&config).await.unwrap();
    assert!(!live.session_id.is_empty());
    assert!(live
        .packet
        .included
        .iter()
        .any(|item| item.path == "tui_live_marker.rs"));

    server_handle.abort();
}

#[tokio::test]
async fn test_live_server_client_recovers_from_stale_session() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("tui_live_stale_marker.rs"),
        "pub fn tui_live_stale_marker() -> &'static str { \"synthetic\" }\n",
    )
    .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_handle = tokio::spawn(async move {
        mimir_server::run_tcp_server(listener, mimir_server::SessionStore::new())
            .await
            .unwrap();
    });

    let mut config = LiveServerConfig::new(addr.to_string(), "Use tui_live_stale_marker");
    config.provider = Some("anthropic".to_string());
    config.model = Some("claude-sonnet-4-20250514".to_string());
    config.session_id = Some("stale-session".to_string());
    config.workspace_root = Some(dir.path().to_path_buf());
    config.timeout = Duration::from_secs(2);

    let live = fetch_live_packet(&config).await.unwrap();
    assert_ne!(live.session_id, "stale-session");
    assert!(live
        .packet
        .included
        .iter()
        .any(|item| item.path == "tui_live_stale_marker.rs"));
    assert_eq!(live.packet.provider, "anthropic");
    assert_eq!(live.packet.model, "claude-sonnet-4-20250514");

    server_handle.abort();
}

#[test]
fn test_interval_refresh_backs_off_after_failure() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let mut config = LiveServerConfig::new(addr.to_string(), "Refresh against closed port");
    config.refresh_interval = Some(Duration::from_secs(60));
    config.timeout = Duration::from_millis(20);

    let mut app = App::new();
    app.set_live_server(config);

    app.maybe_request_interval_refresh();
    assert_eq!(app.status, "Refreshing from live server...");

    for _ in 0..50 {
        app.poll_live_refresh();
        if app.status.starts_with("Live refresh failed:") {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        app.status.starts_with("Live refresh failed:"),
        "expected failed refresh status, got {}",
        app.status
    );
    let failure_status = app.status.clone();

    app.maybe_request_interval_refresh();
    assert_eq!(app.status, failure_status);
}

#[test]
fn test_load_pipeline_result_from_file() {
    let result = mimir_retrieval::PipelineResult {
        manifest: mimir_retrieval::PackedManifest {
            included: vec![],
            omitted: vec![],
            total_tokens: 0,
        },
        sufficiency_ok: true,
        sufficiency_warnings: vec!["warn1".to_string()],
        recall_guard_flags: vec![mimir_retrieval::RecallGuardFlag {
            category: "missing_test".to_string(),
            description: "No tests found".to_string(),
            paths: vec!["src/lib.rs".to_string()],
        }],
    };
    let json = serde_json::to_string_pretty(&result).unwrap();
    let path = "/tmp/mimir_test_pipeline.json";
    std::fs::write(path, &json).unwrap();

    let mut app = App::new();
    assert!(app.pipeline_result.is_none());
    app.load_pipeline_from_file(path)
        .expect("load_pipeline_from_file should succeed");
    assert!(app.pipeline_result.is_some());
    let loaded = app.pipeline_result.unwrap();
    assert!(loaded.sufficiency_ok);
    assert_eq!(loaded.sufficiency_warnings, vec!["warn1"]);
    assert_eq!(loaded.recall_guard_flags.len(), 1);

    let _ = std::fs::remove_file(path);
}

#[test]
fn test_load_packet_missing_file() {
    let mut app = App::new();
    let res = app.load_from_file("/tmp/nonexistent_mimir_packet.json");
    assert!(res.is_err());
}

#[test]
fn test_load_pipeline_missing_file() {
    let mut app = App::new();
    let res = app.load_pipeline_from_file("/tmp/nonexistent_mimir_pipeline.json");
    assert!(res.is_err());
}

#[test]
fn test_load_both_packet_and_pipeline() {
    let packet = minimal_packet();
    let packet_path = "/tmp/mimir_test_packet2.json";
    std::fs::write(packet_path, serde_json::to_string_pretty(&packet).unwrap()).unwrap();

    let pipeline = mimir_retrieval::PipelineResult {
        manifest: mimir_retrieval::PackedManifest {
            included: vec![mimir_retrieval::PackedItem {
                path: "src/main.rs".to_string(),
                ranges: vec![mimir_retrieval::Range { start: 1, end: 10 }],
                estimated_tokens: 100,
                score: 0.95,
                reason_code: "direct_match".to_string(),
            }],
            omitted: vec![],
            total_tokens: 100,
        },
        sufficiency_ok: true,
        sufficiency_warnings: vec![],
        recall_guard_flags: vec![],
    };
    let pipeline_path = "/tmp/mimir_test_pipeline2.json";
    std::fs::write(
        pipeline_path,
        serde_json::to_string_pretty(&pipeline).unwrap(),
    )
    .unwrap();

    let mut app = App::new();
    app.load_from_file(packet_path).unwrap();
    app.load_pipeline_from_file(pipeline_path).unwrap();

    assert!(app.packet.is_some());
    assert!(app.pipeline_result.is_some());
    assert_eq!(
        app.pipeline_result
            .as_ref()
            .unwrap()
            .manifest
            .included
            .len(),
        1
    );

    let _ = std::fs::remove_file(packet_path);
    let _ = std::fs::remove_file(pipeline_path);
}
