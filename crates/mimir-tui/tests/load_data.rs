//! Integration tests for loading live data into the TUI App.

use mimir_schemas::ContextPacket;
use mimir_tui::App;

/// Returns a minimal valid ContextPacket for testing serialization.
fn minimal_packet() -> ContextPacket {
    ContextPacket {
        schema_version: 1,
        packet_id: "pkt-test-001".to_string(),
        packet_hash: "abc123".to_string(),
        run_id: "run-test-001".to_string(),
        task_card: "Test task card".to_string(),
        mode: "standard".to_string(),
        cap_tokens: 64000,
        target_tokens: 32000,
        output_reserve_tokens: 4096,
        count_drift_reserve_tokens: 1024,
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        capability_snapshot_ref: "cap-001".to_string(),
        prompt_contract_version: "1.0".to_string(),
        included: vec![],
        omitted_candidates: vec![],
        tool_schemas: vec![],
        evidence_cards: vec![],
        memory_entries: vec![],
        budget_ledger_ref: None,
        estimated_input_tokens: 1000,
        count_provenance: "tiktoken-rs".to_string(),
        created_at: "2025-01-01T00:00:00Z".to_string(),
        authoritative_input_tokens: None,
        recall_guard_flags: vec![],
    }
}

#[test]
fn test_load_packet_from_file() {
    let packet = minimal_packet();
    let json = serde_json::to_string_pretty(&packet).unwrap();
    let path = "/tmp/mimir_test_packet.json";
    std::fs::write(path, &json).unwrap();

    let mut app = App::new();
    assert!(app.packet.is_none());
    app.load_from_file(path).expect("load_from_file should succeed");
    assert!(app.packet.is_some());
    let loaded = app.packet.unwrap();
    assert_eq!(loaded.packet_id, "pkt-test-001");
    assert_eq!(loaded.run_id, "run-test-001");
    assert!(app.budget.is_some());
    assert_eq!(app.provider_counts.len(), 1);
    assert_eq!(app.provider_counts[0].name, "anthropic");

    // cleanup
    let _ = std::fs::remove_file(path);
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
    assert_eq!(app.pipeline_result.as_ref().unwrap().manifest.included.len(), 1);

    let _ = std::fs::remove_file(packet_path);
    let _ = std::fs::remove_file(pipeline_path);
}
