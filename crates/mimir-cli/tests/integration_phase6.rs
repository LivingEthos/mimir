//! CLI integration tests for packet/run behavior.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn context_packet_validator() -> jsonschema::Validator {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../schemas/ContextPacket.schema.json")).unwrap();
    let omitted: serde_json::Value = serde_json::from_str(include_str!(
        "../../../schemas/OmittedCandidate.schema.json"
    ))
    .unwrap();
    let candidate: serde_json::Value = serde_json::from_str(include_str!(
        "../../../schemas/ContextCandidate.schema.json"
    ))
    .unwrap();

    jsonschema::options()
        .with_resources(
            [
                (
                    "https://mimir.dev/schemas/OmittedCandidate.schema.json",
                    jsonschema::Resource::from_contents(omitted).unwrap(),
                ),
                (
                    "https://mimir.dev/schemas/ContextCandidate.schema.json",
                    jsonschema::Resource::from_contents(candidate).unwrap(),
                ),
            ]
            .into_iter(),
        )
        .build(&schema)
        .unwrap()
}

fn assert_valid_context_packet(packet: &serde_json::Value) {
    let validator = context_packet_validator();
    let errors = validator
        .iter_errors(packet)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(
        errors.is_empty(),
        "context packet schema errors: {errors:#?}"
    );
}

fn assert_valid_plan_artifact(plan: &serde_json::Value) {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../schemas/PlanArtifact.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors = validator
        .iter_errors(plan)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(
        errors.is_empty(),
        "plan artifact schema errors: {errors:#?}"
    );
}

fn rewrite_packet_capability_snapshot(packet_path: &std::path::Path, snapshot_ref: &str) {
    let mut packet: mimir_schemas::ContextPacket =
        serde_json::from_str(&std::fs::read_to_string(packet_path).unwrap()).unwrap();
    packet.capability_snapshot_ref = snapshot_ref.to_string();
    packet.packet_hash = mimir_context::hash_packet(&packet);
    std::fs::write(packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();
}

fn rewrite_packet_run_id(packet_path: &std::path::Path, run_id: &str) {
    let mut packet: mimir_schemas::ContextPacket =
        serde_json::from_str(&std::fs::read_to_string(packet_path).unwrap()).unwrap();
    packet.run_id = run_id.to_string();
    packet.packet_hash = mimir_context::hash_packet(&packet);
    std::fs::write(packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();
}

fn rewrite_packet_included_file(packet_path: &std::path::Path, path: &str, source_hash: &str) {
    let mut packet: mimir_schemas::ContextPacket =
        serde_json::from_str(&std::fs::read_to_string(packet_path).unwrap()).unwrap();
    packet.included = vec![mimir_schemas::IncludedItem {
        path: path.to_string(),
        ranges: Vec::new(),
        candidate_kind: "full_file".to_string(),
        reason_code: "direct_user_mention".to_string(),
        tokens: 4,
        source_hash: source_hash.to_string(),
        trust_level: "trusted".to_string(),
        editable: false,
        compression: None,
    }];
    packet.packet_hash = mimir_context::hash_packet(&packet);
    std::fs::write(packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();
}

fn rewrite_packet_goal(packet_path: &std::path::Path, goal: &str) {
    let mut packet: mimir_schemas::ContextPacket =
        serde_json::from_str(&std::fs::read_to_string(packet_path).unwrap()).unwrap();
    packet.task_card.goal = goal.to_string();
    packet.packet_hash = mimir_context::hash_packet(&packet);
    std::fs::write(packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[test]
fn memory_subcommand_help_lists_actions() {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.args(["memory", "--help"]).assert().success().stdout(
        contains("list")
            .and(contains("show"))
            .and(contains("search")),
    );
}

#[test]
fn memory_import_sessions_imports_codex_jsonl() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("session.jsonl"),
        "{\"role\":\"user\",\"content\":\"Write a parser regression test\"}\n\
         {\"role\":\"assistant\",\"content\":\"The parser test should cover malformed input\"}\n",
    )
    .unwrap();

    let mut import = Command::cargo_bin("mimir").unwrap();
    import
        .current_dir(dir.path())
        .args([
            "memory",
            "import-sessions",
            "--from",
            "codex",
            "session.jsonl",
        ])
        .assert()
        .success()
        .stdout(contains("Imported 2 total session entries from codex"));

    let mut search = Command::cargo_bin("mimir").unwrap();
    search
        .current_dir(dir.path())
        .args(["memory", "search", "parser"])
        .assert()
        .success()
        .stdout(contains("results for 'parser'").and(contains("codex")));
}

#[test]
fn memory_import_sessions_discovers_codex_defaults_dry_run() {
    let dir = TempDir::new().unwrap();
    let codex_home = dir.path().join("codex-home");
    let session_dir = codex_home.join("sessions/2026/05/20");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(
        session_dir.join("rollout-synthetic.jsonl"),
        "{\"role\":\"user\",\"content\":\"Synthetic only\"}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .env("CODEX_HOME", &codex_home)
        .args([
            "memory",
            "import-sessions",
            "--from",
            "codex",
            "--discover",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(contains("Discovered 1 session file(s) from codex"))
        .stdout(contains("rollout-synthetic.jsonl"));
}

#[test]
fn memory_import_sessions_requires_path_or_discovery() {
    let dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args(["memory", "import-sessions", "--from", "codex"])
        .assert()
        .failure()
        .stderr(contains(
            "mimir memory import-sessions requires PATH arguments or --discover",
        ));
}

#[test]
fn memory_import_sessions_rejects_unknown_source() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("session.log"), "synthetic session\n").unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args([
            "memory",
            "import-sessions",
            "--from",
            "unknown-tool",
            "session.log",
        ])
        .assert()
        .failure()
        .stderr(contains("unsupported session source 'unknown-tool'"));
}

#[test]
fn version_prints_binary_name() {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.arg("version")
        .assert()
        .success()
        .stdout(contains("mimir"));
}

#[test]
fn serve_help_lists_transports() {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.args(["serve", "--help"])
        .assert()
        .success()
        .stdout(contains("port").or(contains("stdio")));
}

#[test]
fn serve_rejects_mixed_transports() {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.args(["serve", "--port", "9999", "--rpc-stdio"])
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
}

#[test]
fn serve_rpc_stdio_routes_logs_to_stderr() {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .env("RUST_LOG", "info")
        .args(["serve", "--rpc-stdio"])
        .write_stdin("")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "stdout must stay JSON-RPC clean, got: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("Starting Mimir server on stdio"));
}

#[test]
fn tui_help_lists_live_server_refresh_flags() {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.args(["tui", "--help"])
        .assert()
        .success()
        .stdout(contains("--server"))
        .stdout(contains("--task"))
        .stdout(contains("--refresh-ms"));
}

#[test]
fn tui_server_requires_task() {
    let dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args(["tui", "--server", "127.0.0.1:7788"])
        .assert()
        .failure()
        .stderr(contains("--server requires --task"));
}

#[test]
fn tui_server_requires_provider_and_model_together() {
    let dir = TempDir::new().unwrap();
    let mut provider_only = Command::cargo_bin("mimir").unwrap();
    provider_only
        .current_dir(dir.path())
        .args([
            "tui",
            "--server",
            "127.0.0.1:7788",
            "--task",
            "inspect context",
            "--provider",
            "glm",
        ])
        .assert()
        .failure()
        .stderr(contains("--provider and --model to be set together"));

    let mut model_only = Command::cargo_bin("mimir").unwrap();
    model_only
        .current_dir(dir.path())
        .args([
            "tui",
            "--server",
            "127.0.0.1:7788",
            "--task",
            "inspect context",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .failure()
        .stderr(contains("--provider and --model to be set together"));
}

#[test]
fn doctor_reports_ok_for_initialized_project() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .arg("doctor")
        .assert()
        .success()
        .stdout(
            contains("Config: ok")
                .and(contains("Provider capabilities: ok"))
                .and(contains("Token counter: ok"))
                .and(contains("Context packet: ok"))
                .and(contains("Permissions: ok"))
                .and(contains("Provider credentials: optional"))
                .and(contains("Doctor status: ok")),
        );
}

#[test]
fn doctor_warns_when_project_is_not_initialized() {
    let dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .arg("doctor")
        .assert()
        .success()
        .stdout(contains("Config: missing").and(contains("Doctor status: warnings")));
}

#[test]
fn providers_doctor_validates_and_lists_registry_models() {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.args(["providers", "doctor"]).assert().success().stdout(
        contains("Provider capabilities: ok")
            .and(contains("anthropic"))
            .and(contains("claude-sonnet-4-20250514"))
            .and(contains("glm"))
            .and(contains("openai"))
            .and(contains("openai-compatible")),
    );
}

#[test]
fn context_build_writes_hashable_packet_with_matching_run_id() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success()
        .stdout(contains("Built packet"));

    let runs_root = dir.path().join(".mimir/runs");
    let run_dir = std::fs::read_dir(&runs_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    let packet: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(packet_path).unwrap()).unwrap();

    assert_eq!(packet["run_id"], run_id);
    assert_eq!(packet["provider"], "glm");
    assert_eq!(packet["model"], "glm-5.1");
    assert_eq!(packet["packet_hash"].as_str().unwrap().len(), 64);
    assert!(packet["estimated_input_tokens"].as_u64().unwrap() > 0);
    assert_valid_context_packet(&packet);
}

#[test]
fn context_call_rejects_tampered_packet_hash_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let runs_root = dir.path().join(".mimir/runs");
    let run_dir = std::fs::read_dir(&runs_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let packet_path = run_dir.join("context_packet.json");
    let mut packet: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&packet_path).unwrap()).unwrap();
    packet["task_card"]["goal"] = json!("tampered task");
    std::fs::write(&packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();

    let mut call = Command::cargo_bin("mimir").unwrap();
    call.current_dir(dir.path())
        .args(["context", "call", packet_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(contains("context packet hash mismatch"));
}

#[test]
fn context_call_rejects_changed_included_file_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let runs_root = dir.path().join(".mimir/runs");
    let run_dir = std::fs::read_dir(&runs_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let packet_path = run_dir.join("context_packet.json");
    let mut packet: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&packet_path).unwrap()).unwrap();
    packet["included"] = json!([{
        "path": "main.rs",
        "ranges": [],
        "candidate_kind": "full_file",
        "reason_code": "direct_task_match",
        "tokens": 4,
        "source_hash": "0".repeat(64),
        "trust_level": "trusted",
        "editable": false
    }]);
    let mut typed_packet: mimir_schemas::ContextPacket = serde_json::from_value(packet).unwrap();
    typed_packet.packet_hash = mimir_context::hash_packet(&typed_packet);
    std::fs::write(
        &packet_path,
        serde_json::to_vec_pretty(&typed_packet).unwrap(),
    )
    .unwrap();

    let mut call = Command::cargo_bin("mimir").unwrap();
    call.current_dir(dir.path())
        .args(["context", "call", packet_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(contains("source_hash mismatch"));
}

#[test]
fn context_call_rejects_stale_capability_snapshot_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_capability_snapshot(
        &packet_path,
        "generated:glm/glm-5.1@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );

    let mut call = Command::cargo_bin("mimir").unwrap();
    call.current_dir(dir.path())
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .args(["context", "call", packet_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(contains("capability snapshot mismatch"));
}

#[test]
fn packet_replay_rejects_stale_capability_snapshot() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_capability_snapshot(
        &packet_path,
        "generated:glm/glm-5.1@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );

    let mut replay = Command::cargo_bin("mimir").unwrap();
    replay
        .current_dir(dir.path())
        .args(["packet", "replay", &run_id])
        .assert()
        .failure()
        .stderr(contains("capability snapshot mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.replay",
        &["main.rs", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
    );
}

#[test]
fn packet_share_rejects_stale_capability_snapshot() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_capability_snapshot(
        &packet_path,
        "generated:glm/glm-5.1@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    );

    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args(["packet", "share", &run_id])
        .assert()
        .failure()
        .stderr(contains("capability snapshot mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.share",
        &["main.rs", "cccccccccccccccccccccccccccccccc"],
    );
}

#[test]
fn packet_replay_rejects_packet_run_id_mismatch() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_run_id(&packet_path, "20260101-000000-deadbeef");

    let mut replay = Command::cargo_bin("mimir").unwrap();
    replay
        .current_dir(dir.path())
        .args(["packet", "replay", &run_id])
        .assert()
        .failure()
        .stderr(contains("packet run_id mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.replay",
        &["main.rs", "20260101-000000-deadbeef"],
    );
}

#[test]
fn packet_share_rejects_packet_run_id_mismatch() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_run_id(&packet_path, "20260101-000000-feedface");

    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args(["packet", "share", &run_id])
        .assert()
        .failure()
        .stderr(contains("packet run_id mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.share",
        &["main.rs", "20260101-000000-feedface"],
    );
}

#[test]
fn packet_replay_rejects_tampered_packet_hash() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    let mut packet: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&packet_path).unwrap()).unwrap();
    packet["task_card"]["goal"] = json!("tampered replay task");
    std::fs::write(&packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();

    let mut replay = Command::cargo_bin("mimir").unwrap();
    replay
        .current_dir(dir.path())
        .args(["packet", "replay", &run_id])
        .assert()
        .failure()
        .stderr(contains("context packet hash mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.replay",
        &["main.rs", "tampered replay task"],
    );
}

#[test]
fn packet_share_rejects_tampered_packet_hash() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    let mut packet: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&packet_path).unwrap()).unwrap();
    packet["task_card"]["goal"] = json!("tampered share task");
    std::fs::write(&packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();

    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args(["packet", "share", &run_id])
        .assert()
        .failure()
        .stderr(contains("context packet hash mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.share",
        &["main.rs", "tampered share task"],
    );
}

#[test]
fn packet_replay_rejects_changed_included_file() {
    let dir = TempDir::new().unwrap();
    let original = "fn main() {}\n";
    std::fs::write(dir.path().join("main.rs"), original).unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_included_file(&packet_path, "main.rs", &sha256_hex(original.as_bytes()));
    std::fs::write(
        dir.path().join("main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();

    let mut replay = Command::cargo_bin("mimir").unwrap();
    replay
        .current_dir(dir.path())
        .args(["packet", "replay", &run_id])
        .assert()
        .failure()
        .stderr(contains("source_hash mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.replay",
        &["main.rs", "println!(\"changed\")"],
    );
}

#[test]
fn packet_share_rejects_changed_included_file() {
    let dir = TempDir::new().unwrap();
    let original = "fn main() {}\n";
    std::fs::write(dir.path().join("main.rs"), original).unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_included_file(&packet_path, "main.rs", &sha256_hex(original.as_bytes()));
    std::fs::write(
        dir.path().join("main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();

    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args(["packet", "share", &run_id])
        .assert()
        .failure()
        .stderr(contains("source_hash mismatch"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.share",
        &["main.rs", "println!(\"changed\")"],
    );
}

#[test]
fn packet_share_rejects_secret_like_packet_metadata_with_private_trace() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_goal(
        &packet_path,
        "share this synthetic secret sk-12345678901234567890",
    );

    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args(["packet", "share", &run_id])
        .assert()
        .failure()
        .stderr(contains("context packet contains secret-like text"))
        .stderr(
            predicates::str::is_match("sk-12345678901234567890")
                .unwrap()
                .not(),
        );
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.share",
        &["share this synthetic secret", "sk-12345678901234567890"],
    );
}

#[test]
fn packet_share_and_replay_reject_secret_like_saved_request_with_private_trace() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    std::fs::write(
        run_dir.join("provider_request.redacted.json"),
        serde_json::to_vec_pretty(&json!({
            "model": "glm-5.1",
            "messages": [
                {
                    "role": "user",
                    "content": "synthetic request secret sk-12345678901234567890"
                }
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args(["packet", "share", &run_id])
        .assert()
        .failure()
        .stderr(contains(
            "provider request artifact contains secret-like text",
        ))
        .stderr(
            predicates::str::is_match("sk-12345678901234567890")
                .unwrap()
                .not(),
        );
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.share",
        &[
            "provider_request.redacted.json",
            "synthetic request secret",
            "sk-12345678901234567890",
        ],
    );

    let mut replay = Command::cargo_bin("mimir").unwrap();
    replay
        .current_dir(dir.path())
        .args(["packet", "replay", &run_id, "--request-json"])
        .assert()
        .failure()
        .stderr(contains(
            "provider request artifact contains secret-like text",
        ))
        .stderr(
            predicates::str::is_match("sk-12345678901234567890")
                .unwrap()
                .not(),
        );
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.replay",
        &[
            "provider_request.redacted.json",
            "synthetic request secret",
            "sk-12345678901234567890",
        ],
    );
}

#[test]
fn packet_replay_rejects_oversized_saved_request_with_private_trace() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    std::fs::write(
        run_dir.join("provider_request.redacted.json"),
        vec![b'a'; 256 * 1024 + 1],
    )
    .unwrap();

    let mut replay = Command::cargo_bin("mimir").unwrap();
    replay
        .current_dir(dir.path())
        .args(["packet", "replay", &run_id, "--request-json"])
        .assert()
        .failure()
        .stderr(contains("exceeds size cap"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.packet.replay",
        &["provider_request.redacted.json", ".mimir/runs"],
    );
}

#[tokio::test]
async fn packet_share_bundle_replays_redacted_request_from_fresh_checkout() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    let server = mock_openai_response("shared replay response".to_string()).await;

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");

    let mut call = Command::cargo_bin("mimir").unwrap();
    let output = call
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "context",
            "call",
            packet_path.to_str().unwrap(),
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();
    assert_success(&output);

    let saved_request_path = run_dir.join("provider_request.redacted.json");
    let saved_request = std::fs::read(&saved_request_path).unwrap();

    let mut local_replay = Command::cargo_bin("mimir").unwrap();
    let local_output = local_replay
        .current_dir(dir.path())
        .args(["packet", "replay", &run_id, "--request-json"])
        .output()
        .unwrap();
    assert_success(&local_output);
    assert_eq!(local_output.stdout, saved_request);

    let bundle_path = dir.path().join("shared-bundle.json");
    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args([
            "packet",
            "share",
            &run_id,
            "--output",
            bundle_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(contains("Shared packet bundle written"));

    let bundle_text = std::fs::read_to_string(&bundle_path).unwrap();
    assert!(!bundle_text.contains("test-key"));
    assert!(!bundle_text.contains("OPENAI_API_KEY"));
    let bundle: serde_json::Value = serde_json::from_str(&bundle_text).unwrap();
    assert_eq!(bundle["kind"], json!("mimir.packet_share"));
    assert_eq!(bundle["schema_version"], json!(1));
    assert_eq!(bundle["run_id"], json!(run_id));
    let saved_request_json: serde_json::Value = serde_json::from_slice(&saved_request).unwrap();
    assert_eq!(
        bundle["replay"]["provider_request_redacted"],
        saved_request_json
    );

    let fresh = TempDir::new().unwrap();
    let fresh_bundle_path = fresh.path().join("shared-bundle.json");
    std::fs::copy(&bundle_path, &fresh_bundle_path).unwrap();

    let mut replay_request = Command::cargo_bin("mimir").unwrap();
    let replay_output = replay_request
        .current_dir(fresh.path())
        .args([
            "packet",
            "replay",
            fresh_bundle_path.to_str().unwrap(),
            "--request-json",
        ])
        .output()
        .unwrap();
    assert_success(&replay_output);
    assert_eq!(replay_output.stdout, saved_request);

    let mut replay_summary = Command::cargo_bin("mimir").unwrap();
    replay_summary
        .current_dir(fresh.path())
        .args(["packet", "replay", fresh_bundle_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(contains("Replaying shared packet").and(contains("Provider request sha256")));
}

#[tokio::test]
async fn packet_share_bundle_uses_saved_plan_provider_request() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let content = json!({
        "steps": ["Inspect app.txt"],
        "risks": [],
        "files_likely_affected": ["app.txt"],
        "tests_to_run": [],
        "assumptions": ["synthetic provider"]
    })
    .to_string();
    let server = mock_openai_response(content).await;

    let mut plan = Command::cargo_bin("mimir").unwrap();
    let output = plan
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "plan",
            "Plan a tiny text update",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_success(&output);
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = stdout["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    let saved_request = std::fs::read(run_dir.join("provider_request.redacted.json")).unwrap();

    let bundle_path = dir.path().join("plan-share.json");
    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .args([
            "packet",
            "share",
            run_id,
            "--output",
            bundle_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let fresh = TempDir::new().unwrap();
    let fresh_bundle_path = fresh.path().join("plan-share.json");
    std::fs::copy(&bundle_path, &fresh_bundle_path).unwrap();

    let mut replay = Command::cargo_bin("mimir").unwrap();
    let replay_output = replay
        .current_dir(fresh.path())
        .args([
            "packet",
            "replay",
            fresh_bundle_path.to_str().unwrap(),
            "--request-json",
        ])
        .output()
        .unwrap();
    assert_success(&replay_output);
    assert_eq!(replay_output.stdout, saved_request);
}

#[tokio::test]
async fn context_call_dispatches_saved_mode_specific_request() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let content = json!({
        "steps": ["Inspect app.txt"],
        "risks": [],
        "files_likely_affected": ["app.txt"],
        "tests_to_run": [],
        "assumptions": ["synthetic provider"]
    })
    .to_string();
    let plan_server = mock_openai_response(content).await;

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "plan",
            "Plan a tiny text update",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &plan_server.uri(),
            "--json",
        ])
        .output()
        .unwrap();
    assert_success(&output);
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = stdout["run_id"].as_str().unwrap();
    let packet_path = dir
        .path()
        .join(".mimir/runs")
        .join(run_id)
        .join("context_packet.json");

    let replay_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_string_contains("Return only JSON with this shape"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "mock-model",
            "choices": [{
                "message": {"content": "replayed saved plan request"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 4}
        })))
        .expect(1)
        .mount(&replay_server)
        .await;

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "context",
            "call",
            packet_path.to_str().unwrap(),
            "--base-url",
            &replay_server.uri(),
        ])
        .output()
        .unwrap();

    assert_success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("replayed saved plan request"));
    let trace_text =
        std::fs::read_to_string(packet_path.parent().unwrap().join("trace.spans.jsonl")).unwrap();
    let spans = trace_spans(&trace_text);
    assert!(spans
        .iter()
        .any(|span| span["name"] == "mimir.context.call"));
    assert!(
        spans
            .iter()
            .filter(|span| span["name"] == "mimir.provider.dispatch")
            .count()
            >= 2
    );
    assert!(!trace_text.contains("Plan a tiny text update"));
}

#[tokio::test]
async fn context_call_provider_error_records_sanitized_command_span() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let packet_path = run_dir.join("context_packet.json");
    let body = "synthetic context call error body OPENAI_API_KEY=sk-12345678901234567890";
    let server = mock_openai_error_response(400, body).await;

    let mut call = Command::cargo_bin("mimir").unwrap();
    let output = call
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "context",
            "call",
            packet_path.to_str().unwrap(),
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let trace_text = std::fs::read_to_string(run_dir.join("trace.spans.jsonl")).unwrap();
    let spans = trace_spans(&trace_text);
    assert!(spans.iter().any(|span| {
        span["name"] == "mimir.provider.dispatch" && span["attrs"]["status"] == "error"
    }));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.context.call",
        &[
            "Build a replayable context packet for the current repository",
            "app.txt",
            body,
            "synthetic context call error body",
        ],
    );
}

#[test]
fn context_call_rejects_run_path_mismatch_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_run_id(&packet_path, "20260101-000000-badc0ffe");

    let mut call = Command::cargo_bin("mimir").unwrap();
    call.current_dir(dir.path())
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .args(["context", "call", packet_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(contains("packet run_id mismatch"));
}

#[test]
fn context_call_rejects_detached_packet_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let detached = dir.path().join("detached_packet.json");
    std::fs::copy(run_dir.join("context_packet.json"), &detached).unwrap();

    let mut call = Command::cargo_bin("mimir").unwrap();
    call.current_dir(dir.path())
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .args(["context", "call", detached.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(contains(".mimir/runs/<run_id>/context_packet.json"));
}

#[test]
fn context_call_rejects_over_cap_packet_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "openai-compatible",
            "--model",
            "gpt-4.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let packet_path = run_dir.join("context_packet.json");
    let mut packet: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&packet_path).unwrap()).unwrap();
    packet["estimated_input_tokens"] = json!(999_999_999_u64);
    std::fs::write(&packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();

    let mut call = Command::cargo_bin("mimir").unwrap();
    call.current_dir(dir.path())
        .env_remove("OPENAI_API_KEY")
        .args(["context", "call", packet_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(contains("gateway_over_cap"))
        .stderr(predicates::str::contains("OPENAI_API_KEY not set").not());
}

#[test]
fn missing_packet_share_does_not_create_run_directory() {
    let dir = TempDir::new().unwrap();
    let run_id = "20260101-120000-deadbeef";

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args(["packet", "share", run_id])
        .assert()
        .failure()
        .stdout(contains("No packet found"));

    assert!(!dir.path().join(".mimir/runs").join(run_id).exists());
}

#[test]
fn missing_trace_export_does_not_create_run_directory() {
    let dir = TempDir::new().unwrap();
    let run_id = "20260101-120000-feedface";

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args(["trace", "export", run_id, "--redact"])
        .assert()
        .failure()
        .stdout(contains("No trace found"));

    assert!(!dir.path().join(".mimir/runs").join(run_id).exists());
}

#[test]
fn trace_export_prefers_first_class_trace_spans() {
    let dir = TempDir::new().unwrap();
    let run_id = "20260101-120000-facefeed";
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("trace.spans.jsonl"),
        "{\"schema_version\":1,\"span_id\":\"0123456789abcdef\",\"name\":\"mimir.context.build\",\"start_us\":1,\"end_us\":2,\"attrs\":{\"api_key\":\"sk-123456789012345678901234\"}}\n",
    )
    .unwrap();
    std::fs::write(
        run_dir.join("events.jsonl"),
        "{\"event\":\"legacy fallback should not win\"}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .args(["trace", "export", run_id, "--redact"])
        .output()
        .unwrap();

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"source\": \"trace.spans.jsonl\""));
    assert!(stdout.contains("mimir.context.build"));
    assert!(!stdout.contains("legacy fallback should not win"));
    assert!(!stdout.contains("sk-123456789012345678901234"));
}

#[test]
fn trace_export_malformed_trace_spans_falls_back_to_redacted_events() {
    let dir = TempDir::new().unwrap();
    let run_id = "20260101-120000-badc0de0";
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("trace.spans.jsonl"), "not-json-at-all\n").unwrap();
    std::fs::write(
        run_dir.join("events.jsonl"),
        "{\"event\":\"legacy fallback wins\",\"api_key\":\"sk-123456789012345678901234\"}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .args(["trace", "export", run_id, "--redact"])
        .output()
        .unwrap();

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"source\": \"events.jsonl\""));
    assert!(stdout.contains("legacy fallback wins"));
    assert!(!stdout.contains("not-json-at-all"));
    assert!(!stdout.contains("sk-123456789012345678901234"));
}

#[test]
fn trace_export_malformed_trace_and_events_reports_no_trace() {
    let dir = TempDir::new().unwrap();
    let run_id = "20260101-120000-bade0001";
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("trace.spans.jsonl"), "not-json-at-all\n").unwrap();
    std::fs::write(
        run_dir.join("events.jsonl"),
        "{\"api_key\":\"sk-123456789012345678901234\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args(["trace", "export", run_id, "--redact"])
        .assert()
        .failure()
        .stdout(contains("No trace found"))
        .stdout(contains("not-json-at-all").not())
        .stdout(contains("sk-123456789012345678901234").not());
}

#[test]
fn trace_export_output_write_failure_does_not_print_export_json() {
    let dir = TempDir::new().unwrap();
    let run_id = "20260101-120000-dead0002";
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(
        run_dir.join("trace.spans.jsonl"),
        "{\"schema_version\":1,\"span_id\":\"0123456789abcdef\",\"name\":\"mimir.context.build\",\"start_us\":1,\"end_us\":2}\n",
    )
    .unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args([
            "trace",
            "export",
            run_id,
            "--redact",
            "--output",
            "missing-parent/trace.json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("mimir.context.build"));
    assert!(!stdout.contains("\"events\""));
}

#[test]
fn trace_export_rejects_traversal_without_reading_outside_events() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir")).unwrap();
    std::fs::write(
        dir.path().join(".mimir/events.jsonl"),
        "{\"secret\":\"sk-123456789012345678901234\"}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .args(["trace", "export", "..", "--redact"])
        .assert()
        .failure()
        .stdout(contains("No trace found"))
        .stdout(contains("sk-123456789012345678901234").not());
}

#[test]
fn context_call_rejects_included_path_traversal_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();
    let outside_name = format!("mimir-outside-context-{}.txt", std::process::id());
    let outside_rel = format!("../{outside_name}");
    let outside = dir.path().parent().unwrap().join(&outside_name);
    std::fs::write(&outside, "outside context\n").unwrap();
    let outside_hash = sha256_hex(&std::fs::read(&outside).unwrap());

    let mut build = Command::cargo_bin("mimir").unwrap();
    build
        .current_dir(dir.path())
        .args([
            "context",
            "build",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
        ])
        .assert()
        .success();

    let run_dir = latest_run_dir(&dir);
    let packet_path = run_dir.join("context_packet.json");
    rewrite_packet_included_file(&packet_path, &outside_rel, &outside_hash);

    let mut call = Command::cargo_bin("mimir").unwrap();
    call.current_dir(dir.path())
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .args(["context", "call", packet_path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(contains("included context path escapes workspace"))
        .stderr(contains("GLM_API_KEY").not())
        .stderr(contains("ZAI_API_KEY").not());

    let _ = std::fs::remove_file(outside);
}

#[tokio::test]
async fn plan_writes_provider_backed_artifacts_with_redaction() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let leaked = "OPENAI_API_KEY=sk-12345678901234567890";
    let content = json!({
        "steps": ["Inspect app.txt", "Prepare a minimal edit"],
        "risks": [leaked],
        "files_likely_affected": ["app.txt"],
        "tests_to_run": ["cargo test"],
        "assumptions": ["mock provider"]
    })
    .to_string();
    let server = mock_openai_response(content).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "plan",
            "Update the greeting",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--json",
        ])
        .output()
        .unwrap();

    assert_success(&output);
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout_text.contains(leaked));
    let run_id = stdout["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    assert!(run_dir.join("context_packet.json").is_file());
    assert!(run_dir.join("provider_request.redacted.json").is_file());
    assert!(run_dir.join("response.json").is_file());
    assert!(run_dir.join("plan.md").is_file());
    assert!(run_dir.join("plan.json").is_file());

    let packet: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
    )
    .unwrap();
    assert_valid_context_packet(&packet);

    let plan = std::fs::read_to_string(run_dir.join("plan.json")).unwrap();
    let plan_json: serde_json::Value = serde_json::from_str(&plan).unwrap();
    assert_valid_plan_artifact(&plan_json);
    let plan_md = std::fs::read_to_string(run_dir.join("plan.md")).unwrap();
    let response = std::fs::read_to_string(run_dir.join("response.json")).unwrap();
    assert!(!plan.contains(leaked));
    assert!(!plan_md.contains(leaked));
    assert!(!response.contains(leaked));
    assert!(plan.contains("<REDACTED:"));
    assert!(plan_md.contains("<REDACTED:"));
}

#[tokio::test]
async fn ask_plain_stdout_redacts_provider_response() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let leaked = "OPENAI_API_KEY=sk-12345678901234567890";
    let server = mock_openai_response(format!("plain response leaked {leaked}")).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "ask",
            "Summarize app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(leaked));
    assert!(stdout.contains("<REDACTED:"));
    let run_dir = latest_run_dir(&dir);
    let trace_text = std::fs::read_to_string(run_dir.join("trace.spans.jsonl")).unwrap();
    let spans = trace_spans(&trace_text);
    assert!(spans.iter().any(|span| span["name"] == "mimir.ask"));
    assert!(spans
        .iter()
        .any(|span| span["name"] == "mimir.context.build"));
    assert!(spans
        .iter()
        .any(|span| span["name"] == "mimir.provider.dispatch"));
    assert!(!trace_text.contains("Summarize app.txt"));
}

#[tokio::test]
async fn ask_provider_error_records_sanitized_command_span() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let task = "Trace ask provider failure privately";
    let body = "synthetic ask error body OPENAI_API_KEY=sk-12345678901234567890";
    let server = mock_openai_error_response(400, body).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "ask",
            task,
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let run_dir = latest_run_dir(&dir);
    let trace_text = std::fs::read_to_string(run_dir.join("trace.spans.jsonl")).unwrap();
    let spans = trace_spans(&trace_text);
    assert!(spans.iter().any(|span| {
        span["name"] == "mimir.provider.dispatch" && span["attrs"]["status"] == "error"
    }));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.ask",
        &[task, "app.txt", body, "synthetic ask error body"],
    );
}

#[tokio::test]
async fn plan_omits_secret_like_included_file_from_provider_request() {
    let dir = TempDir::new().unwrap();
    let leaked = "OPENAI_API_KEY=sk-12345678901234567890";
    std::fs::write(dir.path().join("secrets.txt"), format!("{leaked}\n")).unwrap();
    let content = json!({
        "steps": ["Do not include secret file content"],
        "risks": [],
        "files_likely_affected": [],
        "tests_to_run": [],
        "assumptions": []
    })
    .to_string();
    let server = mock_openai_response(content).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "plan",
            "Review secrets.txt without exposing its contents",
            "--editable",
            "secrets.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--json",
        ])
        .output()
        .unwrap();

    assert_success(&output);
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = stdout["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    let packet_text = std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap();
    let request_text =
        std::fs::read_to_string(run_dir.join("provider_request.redacted.json")).unwrap();
    assert!(!packet_text.contains(leaked));
    assert!(!request_text.contains(leaked));
    assert!(packet_text.contains("secret_risk"));
    assert!(request_text.contains("secret_risk"));
}

#[test]
fn plan_rejects_secret_like_task_before_provider_auth() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let leaked = "OPENAI_API_KEY=sk-12345678901234567890";

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env_remove("OPENAI_API_KEY")
        .args([
            "plan",
            leaked,
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            "http://127.0.0.1:9",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("secret_risk"), "{stderr}");
    assert!(!stderr.contains("OPENAI_API_KEY not set"), "{stderr}");
}

#[tokio::test]
async fn glm_provider_honors_cli_model_for_gateway_validation() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let content = json!({
        "steps": ["Inspect app.txt"],
        "risks": [],
        "files_likely_affected": ["app.txt"],
        "tests_to_run": [],
        "assumptions": []
    })
    .to_string();
    let server = mock_openai_response(content).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("GLM_API_KEY", "test-key")
        .env("OPENAI_MODEL", "env-model-that-should-not-win")
        .args([
            "plan",
            "Plan with a custom GLM model",
            "--editable",
            "app.txt",
            "--provider",
            "glm",
            "--model",
            "custom-glm-model",
            "--base-url",
            &server.uri(),
            "--json",
        ])
        .output()
        .unwrap();

    assert_success(&output);
    let stdout: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = stdout["run_id"].as_str().unwrap();
    let plan: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(
            dir.path()
                .join(".mimir/runs")
                .join(run_id)
                .join("plan.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(plan["model"], "custom-glm-model");
}

#[tokio::test]
async fn glm_provider_does_not_use_openai_key_as_zai_credential() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let content = json!({
        "steps": ["Inspect app.txt"],
        "risks": [],
        "files_likely_affected": ["app.txt"],
        "tests_to_run": [],
        "assumptions": []
    })
    .to_string();
    let server = mock_openai_response(content).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "plan",
            "Do not leak OpenAI key to GLM",
            "--editable",
            "app.txt",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("GLM_API_KEY or ZAI_API_KEY not set"),
        "{stderr}"
    );
}

#[tokio::test]
async fn code_dry_run_writes_patch_artifacts_without_modifying_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-dry-run",
        "steps": [{
            "action": "unified_diff",
            "path": "app.txt",
            "diff": "--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-hello\n+hello dry\n"
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Update the greeting",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--dry-run",
            "--json",
        ])
        .output()
        .unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["applied"], false);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello\n"
    );

    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    assert!(run_dir.join("context_packet.json").is_file());
    assert!(run_dir.join("patch_plan.json").is_file());
    assert!(run_dir.join("patch_recipe.json").is_file());
    assert!(run_dir.join("patch.diff").is_file());
    assert!(run_dir.join("patch_report.json").is_file());

    let packet: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
    )
    .unwrap();
    assert_valid_context_packet(&packet);
    let recipe: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_recipe.json")).unwrap())
            .unwrap();
    assert_eq!(recipe["packet_id"], packet["packet_id"]);
}

#[tokio::test]
async fn code_rejects_empty_executable_recipe_before_patch_artifacts() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-empty",
        "steps": []
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Reject empty recipe",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("empty patch recipe"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello\n"
    );
    let run_dir = latest_run_dir(&dir);
    assert!(!run_dir.join("patch_recipe.json").exists());
    assert!(!run_dir.join("patch.diff").exists());
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["applied"], false);
    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("empty patch recipe"));
}

#[tokio::test]
async fn code_rejects_schema_invalid_executable_recipe() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-invalid",
        "unexpected": true,
        "steps": [{
            "action": "unified_diff",
            "path": "app.txt",
            "diff": "--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-hello\n+bad\n"
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Reject schema invalid recipe",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not a schema-valid executable patch recipe"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello\n"
    );
    let run_dir = latest_run_dir(&dir);
    assert!(!run_dir.join("patch_recipe.json").exists());
    assert!(!run_dir.join("patch.diff").exists());
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["applied"], false);
    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("schema-valid executable patch recipe"));
}

#[tokio::test]
async fn code_applies_safe_patch_inside_editable_set() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-apply",
        "steps": [{
            "action": "unified_diff",
            "path": "app.txt",
            "diff": "--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-hello\n+hello applied\n"
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Update the greeting",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--json",
        ])
        .output()
        .unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dry_run"], false);
    assert_eq!(report["applied"], true);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello applied"
    );
}

#[tokio::test]
async fn code_runs_detected_tests_and_does_not_execute_provider_commands() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"mimir-test-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn answer() -> i32 { 1 }\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn answer_is_two() {\n        assert_eq!(answer(), 2);\n    }\n}\n",
    )
    .unwrap();
    let patch_plan = json!({
        "patch_plan": {
            "schema_version": 1,
            "plan_id": "plan-test-run",
            "steps": [{
                "action": "unified_diff",
                "path": "src/lib.rs",
                "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn answer() -> i32 { 1 }\n+pub fn answer() -> i32 { 2 }\n"
            }]
        },
        "tests_to_run": ["touch should_not_exist"]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Make the fixture test pass",
            "--editable",
            "src/lib.rs",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--json",
        ])
        .output()
        .unwrap();

    assert_success(&output);
    assert!(!dir.path().join("should_not_exist").exists());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["test_policy"], "auto_detected");
    assert_eq!(report["test_passed"], true);
    assert_eq!(report["test_exit_code"], 0);
    let test_result_path = report["test_result_path"].as_str().unwrap();
    let test_result = std::fs::read_to_string(dir.path().join(test_result_path)).unwrap();
    assert!(test_result.contains("cargo test"));
    assert!(test_result.contains("touch should_not_exist"));
    assert!(test_result.contains("provider-suggested test commands are recorded"));
}

#[tokio::test]
async fn code_skips_auto_tests_when_cargo_build_hook_exists() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"mimir-hook-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("crates/member")).unwrap();
    std::fs::write(dir.path().join("crates/member/build.rs"), "fn main() {}\n").unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn answer() -> i32 { 1 }\n",
    )
    .unwrap();
    let patch_plan = json!({
        "patch_plan": {
            "schema_version": 1,
            "plan_id": "plan-skip-hook",
            "steps": [{
                "action": "unified_diff",
                "path": "src/lib.rs",
                "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn answer() -> i32 { 1 }\n+pub fn answer() -> i32 { 2 }\n"
            }]
        }
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Patch but skip hooky tests",
            "--editable",
            "src/lib.rs",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--json",
        ])
        .output()
        .unwrap();

    assert_success(&output);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["test_policy"], "skipped_suspicious_test_hooks");
    assert_eq!(report["test_passed"], serde_json::Value::Null);
    let test_result_path = report["test_result_path"].as_str().unwrap();
    let test_result = std::fs::read_to_string(dir.path().join(test_result_path)).unwrap();
    assert!(test_result.contains("crates/member/build.rs"));
}

#[tokio::test]
async fn code_fails_closed_when_detected_tests_fail() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"mimir-test-failure-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn answer() -> i32 { 1 }\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn answer_is_two() {\n        assert_eq!(answer(), 2);\n    }\n}\n",
    )
    .unwrap();
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-test-fail",
        "steps": [{
            "action": "unified_diff",
            "path": "src/lib.rs",
            "diff": "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn answer() -> i32 { 1 }\n+pub fn answer() -> i32 { 3 }\n"
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Make the fixture test fail",
            "--editable",
            "src/lib.rs",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--max-repair-turns",
            "0",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("tests_failed"), "{stderr}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["applied"], true);
    assert_eq!(report["test_policy"], "auto_detected");
    assert_eq!(report["test_passed"], false);
    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("tests_failed"));

    let run_id = report["run_id"].as_str().unwrap();
    let events = std::fs::read_to_string(
        dir.path()
            .join(".mimir/runs")
            .join(run_id)
            .join("events.jsonl"),
    )
    .unwrap();
    assert!(events.contains("patch_tests_failed"));
}

#[tokio::test]
async fn code_rejects_full_schema_packet_id_mismatch() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let patch_plan = json!({
        "patch_plan": {
            "schema_version": 1,
            "plan_id": "plan-wrong-packet",
            "packet_id": "pkt-not-this-run",
            "files_to_edit": [{"path": "app.txt", "edit_kind": "unified_diff"}],
            "editable_target_set": ["app.txt"],
            "reasoning_per_edit": [{"path": "app.txt", "rationale": "test mismatch"}],
            "tests_to_run": [],
            "risks": []
        },
        "patch_recipe": {
            "schema_version": 1,
            "plan_id": "plan-wrong-packet",
            "steps": [{
                "action": "unified_diff",
                "path": "app.txt",
                "diff": "--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-hello\n+bad\n"
            }]
        }
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Reject wrong packet",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("packet_id mismatch"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello\n"
    );
}

#[tokio::test]
async fn code_rejects_full_schema_without_files_to_edit() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let patch_plan = json!({
        "patch_plan": {
            "schema_version": 1,
            "plan_id": "plan-missing-files",
            "packet_id": "pkt-placeholder",
            "files_to_edit": [],
            "editable_target_set": ["app.txt"],
            "reasoning_per_edit": [{"path": "app.txt", "rationale": "missing audit file list"}],
            "tests_to_run": [],
            "risks": []
        },
        "patch_recipe": {
            "schema_version": 1,
            "plan_id": "plan-missing-files",
            "steps": [{
                "action": "unified_diff",
                "path": "app.txt",
                "diff": "--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-hello\n+bad\n"
            }]
        }
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Reject empty files_to_edit",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("files_to_edit"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello\n"
    );
}

#[tokio::test]
async fn code_rejects_unified_diff_header_path_mismatch() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-header-mismatch",
        "steps": [{
            "action": "unified_diff",
            "path": "app.txt",
            "diff": "--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-hello\n+bad\n"
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Reject mismatched diff",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not match step path"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello\n"
    );
}

#[tokio::test]
async fn code_rejects_unsafe_patch_paths() {
    let dir = TempDir::new().unwrap();
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-unsafe",
        "steps": [{
            "action": "create",
            "path": "../outside.txt",
            "content": "escaped"
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Write outside the repo",
            "--editable",
            "../outside.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("patch rejected by safety validation"),
        "{stderr}"
    );
    assert!(!dir.path().join("../outside.txt").exists());
    let run_dir = latest_run_dir(&dir);
    let report = std::fs::read_to_string(run_dir.join("patch_report.json")).unwrap();
    assert!(report.contains("FileNotEditable") || report.contains("file_not_editable"));
}

#[test]
fn code_missing_provider_config_errors_clearly() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env_remove("OPENAI_API_KEY")
        .args([
            "code",
            "Update the greeting",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            "http://127.0.0.1:9",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("OPENAI_API_KEY not set"), "{stderr}");
}

#[tokio::test]
async fn code_rejects_secret_like_patch_text_without_persisting_it() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let leaked = "OPENAI_API_KEY=sk-12345678901234567890";
    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-secret",
        "steps": [{
            "action": "unified_diff",
            "path": "app.txt",
            "diff": format!("--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-hello\n+{leaked}\n")
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Try to write a secret",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "hello\n"
    );
    let run_dir = latest_run_dir(&dir);
    for artifact in [
        "patch.diff",
        "patch_plan.json",
        "patch_report.json",
        "response.json",
        "events.jsonl",
    ] {
        let text = std::fs::read_to_string(run_dir.join(artifact)).unwrap();
        assert!(
            !text.contains(leaked),
            "{artifact} leaked provider patch content"
        );
    }
    let report = std::fs::read_to_string(run_dir.join("patch_report.json")).unwrap();
    assert!(report.contains("secret-like text"));
    let patch = std::fs::read_to_string(run_dir.join("patch.diff")).unwrap();
    assert!(patch.contains("<REDACTED:"));
    assert_sanitized_error_trace(
        &run_dir,
        "mimir.code",
        &["Try to write a secret", "app.txt", leaked],
    );
}

#[tokio::test]
async fn code_refuses_dirty_git_target_file() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["add", "app.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args([
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test User",
            "commit",
            "-m",
            "initial",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::fs::write(dir.path().join("app.txt"), "dirty\n").unwrap();

    let patch_plan = json!({
        "schema_version": 1,
        "plan_id": "plan-dirty",
        "steps": [{
            "action": "unified_diff",
            "path": "app.txt",
            "diff": "--- a/app.txt\n+++ b/app.txt\n@@ -1 +1 @@\n-dirty\n+clean patch\n"
        }]
    })
    .to_string();
    let server = mock_openai_response(patch_plan).await;

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "code",
            "Patch a dirty file",
            "--editable",
            "app.txt",
            "--provider",
            "openai-compatible",
            "--model",
            "mock-model",
            "--base-url",
            &server.uri(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("dirty_worktree"), "{stderr}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("app.txt")).unwrap(),
        "dirty\n"
    );
}

async fn mock_openai_response(content: String) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "mock-model",
            "choices": [{
                "message": {"content": content},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 4}
        })))
        .mount(&server)
        .await;
    server
}

async fn mock_openai_error_response(status: u16, body: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(&server)
        .await;
    server
}

fn latest_run_dir(dir: &TempDir) -> std::path::PathBuf {
    let mut dirs: Vec<_> = std::fs::read_dir(dir.path().join(".mimir/runs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    dirs.sort();
    dirs.pop().unwrap()
}

fn trace_spans(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect()
}

fn assert_sanitized_error_trace(run_dir: &std::path::Path, command_name: &str, forbidden: &[&str]) {
    let trace_text = std::fs::read_to_string(run_dir.join("trace.spans.jsonl")).unwrap();
    let spans = trace_spans(&trace_text);
    let command_span = spans
        .iter()
        .find(|span| span["name"] == command_name)
        .unwrap_or_else(|| panic!("missing {command_name} span in {trace_text}"));
    assert_eq!(command_span["attrs"]["status"], "error");
    assert_eq!(command_span["status"]["code"], "error");
    assert!(command_span["attrs"].get("run_id").is_some());
    assert!(command_span["attrs"].get("packet_id").is_some());
    assert!(command_span["attrs"].get("provider").is_some());
    assert!(command_span["attrs"].get("model").is_some());
    assert!(command_span["attrs"].get("error").is_none());
    assert!(command_span["attrs"].get("message").is_none());
    assert!(command_span["attrs"].get("path").is_none());

    for value in [
        "context_packet.json",
        "provider_request.redacted.json",
        "response.json",
        "patch_report.json",
        ".mimir/runs",
        "test-key",
        "OPENAI_API_KEY",
        "sk-12345678901234567890",
        "Generate a production implementation plan",
        "Propose a safe patch",
        "Repair the previously applied Mimir patch",
        "Return only JSON",
        "Current packet_id",
        "messages",
        "choices",
    ]
    .into_iter()
    .chain(forbidden.iter().copied())
    {
        assert!(
            !trace_text.contains(value),
            "trace leaked forbidden value {value:?}:\n{trace_text}"
        );
    }
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
