//! Integration tests for `mimir context expand`.

use std::fs;
use std::process::Command;

fn mimir_bin(mimir_root: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mimir"));
    cmd.current_dir(mimir_root);
    cmd
}

#[test]
fn expand_compressed_item_round_trips_and_verifies_hash() {
    let run_id = "20260101-120000-abcdef20";
    let mimir_root = format!("/tmp/mimir-expand-test-{}", run_id);
    let _ = fs::remove_dir_all(&mimir_root);

    // Seed a packet with a compressed item.
    let original_content = "pub fn hello() {\n    println!(\"world\");\n}\n";
    let original_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(original_content.as_bytes());
        format!("{:x}", hasher.finalize())
    };

    let packet = serde_json::json!({
        "schema_version": 1,
        "packet_id": "pkt-test",
        "packet_hash": "0".repeat(64),
        "run_id": run_id,
        "task_card": {
            "goal": "test",
            "acceptance_criteria": [],
            "likely_files": [],
            "complexity": "tiny"
        },
        "mode": "ask",
        "cap_tokens": 64000,
        "target_tokens": 32000,
        "output_reserve_tokens": 4096,
        "count_drift_reserve_tokens": 512,
        "provider": "anthropic",
        "model": "claude-sonnet-4-20250514",
        "capability_snapshot_ref": "test",
        "prompt_contract_version": 1,
        "included": [
            {
                "path": "src/lib.rs",
                "ranges": [{"start": 1, "end": 3}],
                "candidate_kind": "full_file",
                "reason_code": "symbol_definition",
                "tokens": 10,
                "source_hash": original_hash,
                "trust_level": "trusted",
                "editable": false,
                "compression": {
                    "algorithm": "code_skeleton",
                    "original_tokens": 50,
                    "compressed_tokens": 10,
                    "original_hash": original_hash,
                    "original_artifact_path": format!(".mimir/runs/{}/artifacts/{}.orig", run_id, original_hash)
                }
            }
        ],
        "omitted_candidates": [],
        "tool_schemas": [],
        "evidence_cards": [],
        "memory_entries": [],
        "budget_ledger_ref": format!(".mimir/runs/{}/budget_ledger.json", run_id),
        "estimated_input_tokens": 10,
        "count_provenance": "local_estimate_only",
        "created_at": "2026-01-01T00:00:00Z"
    });

    let dot_mimir = format!("{}/.mimir", mimir_root);
    let run_dir = format!("{}/runs/{}", dot_mimir, run_id);
    let artifacts_dir = format!("{}/artifacts", run_dir);
    fs::create_dir_all(&artifacts_dir).unwrap();
    fs::write(
        format!("{}/context_packet.json", run_dir),
        serde_json::to_vec_pretty(&packet).unwrap(),
    )
    .unwrap();
    fs::write(
        format!("{}/artifacts/{}.orig", run_dir, original_hash),
        original_content,
    )
    .unwrap();

    let output = mimir_bin(&mimir_root)
        .args(["context", "expand", run_id, "src/lib.rs"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("pub fn hello()"),
        "expected expanded content, got: {} (stderr: {})",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success());
}

#[test]
fn expand_hash_mismatch_fails_closed() {
    let run_id = "20260101-120000-abcdef21";
    let mimir_root = format!("/tmp/mimir-expand-test-{}", run_id);
    let _ = fs::remove_dir_all(&mimir_root);

    let packet = serde_json::json!({
        "schema_version": 1,
        "packet_id": "pkt-test",
        "packet_hash": "0".repeat(64),
        "run_id": run_id,
        "task_card": {
            "goal": "test",
            "acceptance_criteria": [],
            "likely_files": [],
            "complexity": "tiny"
        },
        "mode": "ask",
        "cap_tokens": 64000,
        "target_tokens": 32000,
        "output_reserve_tokens": 4096,
        "count_drift_reserve_tokens": 512,
        "provider": "anthropic",
        "model": "claude-sonnet-4-20250514",
        "capability_snapshot_ref": "test",
        "prompt_contract_version": 1,
        "included": [
            {
                "path": "src/lib.rs",
                "ranges": [{"start": 1, "end": 3}],
                "candidate_kind": "full_file",
                "reason_code": "symbol_definition",
                "tokens": 10,
                "source_hash": "a".repeat(64),
                "trust_level": "trusted",
                "editable": false,
                "compression": {
                    "algorithm": "code_skeleton",
                    "original_tokens": 50,
                    "compressed_tokens": 10,
                    "original_hash": "a".repeat(64),
                    "original_artifact_path": format!(".mimir/runs/{}/artifacts/bad.orig", run_id)
                }
            }
        ],
        "omitted_candidates": [],
        "tool_schemas": [],
        "evidence_cards": [],
        "memory_entries": [],
        "budget_ledger_ref": format!(".mimir/runs/{}/budget_ledger.json", run_id),
        "estimated_input_tokens": 10,
        "count_provenance": "local_estimate_only",
        "created_at": "2026-01-01T00:00:00Z"
    });

    let dot_mimir = format!("{}/.mimir", mimir_root);
    let run_dir = format!("{}/runs/{}", dot_mimir, run_id);
    let artifacts_dir = format!("{}/artifacts", run_dir);
    fs::create_dir_all(&artifacts_dir).unwrap();
    fs::write(
        format!("{}/context_packet.json", run_dir),
        serde_json::to_vec_pretty(&packet).unwrap(),
    )
    .unwrap();
    // Write wrong content so hash mismatches.
    fs::write(format!("{}/artifacts/bad.orig", run_dir), "tampered").unwrap();

    let output = mimir_bin(&mimir_root)
        .args(["context", "expand", run_id, "src/lib.rs"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("hash mismatch"),
        "expected hash-mismatch error, got: {}",
        stderr
    );
}

#[test]
fn expand_unknown_target_errors() {
    let run_id = "20260101-120000-abcdef22";
    let mimir_root = format!("/tmp/mimir-expand-test-{}", run_id);
    let _ = fs::remove_dir_all(&mimir_root);

    let packet = serde_json::json!({
        "schema_version": 1,
        "packet_id": "pkt-test",
        "packet_hash": "0".repeat(64),
        "run_id": run_id,
        "task_card": {
            "goal": "test",
            "acceptance_criteria": [],
            "likely_files": [],
            "complexity": "tiny"
        },
        "mode": "ask",
        "cap_tokens": 64000,
        "target_tokens": 32000,
        "output_reserve_tokens": 4096,
        "count_drift_reserve_tokens": 512,
        "provider": "anthropic",
        "model": "claude-sonnet-4-20250514",
        "capability_snapshot_ref": "test",
        "prompt_contract_version": 1,
        "included": [],
        "omitted_candidates": [],
        "tool_schemas": [],
        "evidence_cards": [],
        "memory_entries": [],
        "budget_ledger_ref": format!(".mimir/runs/{}/budget_ledger.json", run_id),
        "estimated_input_tokens": 10,
        "count_provenance": "local_estimate_only",
        "created_at": "2026-01-01T00:00:00Z"
    });

    let dot_mimir = format!("{}/.mimir", mimir_root);
    let run_dir = format!("{}/runs/{}", dot_mimir, run_id);
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        format!("{}/context_packet.json", run_dir),
        serde_json::to_vec_pretty(&packet).unwrap(),
    )
    .unwrap();

    let output = mimir_bin(&mimir_root)
        .args(["context", "expand", run_id, "nonexistent.rs"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found"),
        "expected not-found error, got: {}",
        stderr
    );
}
