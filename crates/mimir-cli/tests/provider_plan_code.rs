use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;
use serde_json::json;
use tempfile::TempDir;

struct MockProvider {
    url: String,
    requests: Receiver<String>,
}

fn start_mock_provider(plan: serde_json::Value) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let body = json!({
        "model": "glm-5.1",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": plan.to_string()
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 17,
            "completion_tokens": 5
        }
    })
    .to_string();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        tx.send(request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    MockProvider { url, requests: rx }
}

fn start_mock_provider_sequence(plans: Vec<serde_json::Value>) -> MockProvider {
    start_mock_provider_content_sequence(
        plans
            .into_iter()
            .map(|plan| plan.to_string())
            .collect::<Vec<_>>(),
    )
}

fn start_repair_provider_that_mutates_owned_file(
    mutation_path: std::path::PathBuf,
    mutation_content: String,
) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let bodies = [
        answer_patch_plan("plan-initial-bad", 1, 3).to_string(),
        answer_patch_plan("plan-repair-good", 3, 2).to_string(),
    ]
    .into_iter()
    .map(|content| {
        json!({
            "model": "glm-5.1",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 500
            }
        })
        .to_string()
    })
    .collect::<Vec<_>>();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        tx.send(request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            bodies[0].len(),
            bodies[0]
        )
        .unwrap();

        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        tx.send(request).unwrap();
        std::fs::write(mutation_path, mutation_content).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            bodies[1].len(),
            bodies[1]
        )
        .unwrap();
    });

    MockProvider { url, requests: rx }
}

fn start_mock_provider_content_sequence(contents: Vec<String>) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let bodies = contents
        .into_iter()
        .map(|content| {
            json!({
                "model": "glm-5.1",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": content
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 1000,
                    "completion_tokens": 500
                }
            })
            .to_string()
        })
        .collect::<Vec<_>>();

    thread::spawn(move || {
        for body in bodies {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            tx.send(request).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });

    MockProvider { url, requests: rx }
}

fn start_packet_bound_patch_provider() -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let packet_id = extract_packet_id(&request).unwrap_or_else(|| "pkt-missing".to_string());
        let plan = json!({
            "patch_plan": {
                "schema_version": 1,
                "plan_id": "plan-full-schema",
                "packet_id": packet_id,
                "files_to_edit": [{
                    "path": "hello.txt",
                    "edit_kind": "line_range_replace",
                    "ranges": [{"start": 1, "end": 1}],
                    "expected_new_content_hash": null
                }],
                "editable_target_set": ["hello.txt"],
                "reasoning_per_edit": [{
                    "path": "hello.txt",
                    "rationale": "Update the requested greeting."
                }],
                "tests_to_run": [],
                "risks": []
            },
            "patch_recipe": {
                "schema_version": 1,
                "plan_id": "plan-full-schema",
                "packet_id": packet_id,
                "steps": [{
                    "action": "line_range",
                    "path": "hello.txt",
                    "start_line": 1,
                    "end_line": 1,
                    "content": "schema greeting\n"
                }]
            }
        });
        let body = json!({
            "model": "glm-5.1",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": plan.to_string()
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 17,
                "completion_tokens": 5
            }
        })
        .to_string();
        tx.send(request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    MockProvider { url, requests: rx }
}

fn start_packet_bound_plan_id_mismatch_provider() -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let packet_id = extract_packet_id(&request).unwrap_or_else(|| "pkt-missing".to_string());
        let plan = json!({
            "patch_plan": {
                "schema_version": 1,
                "plan_id": "plan-metadata",
                "packet_id": packet_id,
                "files_to_edit": [{
                    "path": "hello.txt",
                    "edit_kind": "line_range_replace",
                    "ranges": [{"start": 1, "end": 1}]
                }],
                "editable_target_set": ["hello.txt"],
                "reasoning_per_edit": [{
                    "path": "hello.txt",
                    "rationale": "Exercise plan/recipe binding."
                }],
                "tests_to_run": [],
                "risks": []
            },
            "patch_recipe": {
                "schema_version": 1,
                "plan_id": "plan-recipe",
                "packet_id": packet_id,
                "steps": [{
                    "action": "line_range",
                    "path": "hello.txt",
                    "start_line": 1,
                    "end_line": 1,
                    "content": "bad\n"
                }]
            }
        });
        let body = json!({
            "model": "glm-5.1",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": plan.to_string()
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 17,
                "completion_tokens": 5
            }
        })
        .to_string();
        tx.send(request).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    MockProvider { url, requests: rx }
}

fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];

    loop {
        let read = stream.read(&mut chunk).unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);

        let Some(header_end) = find_bytes(&buffer, b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end + 4]);
        let content_len = content_length(&headers);
        if buffer.len() >= header_end + 4 + content_len {
            break;
        }
    }

    String::from_utf8_lossy(&buffer).to_string()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn content_length(headers: &str) -> usize {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

fn extract_packet_id(request: &str) -> Option<String> {
    let (_, tail) = request.split_once("Current packet_id: ")?;
    let packet_id: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect();
    (!packet_id.is_empty()).then_some(packet_id)
}

fn mimir_cmd(dir: &TempDir, provider_url: &str) -> Command {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("MIMIR_PROVIDER")
        .env_remove("MIMIR_MODEL")
        .env_remove("MIMIR_BASE_URL")
        .env("GLM_API_KEY", "test-key")
        .env("GLM_BASE_URL", provider_url)
        .env("GLM_MODEL", "glm-5.1");
    cmd
}

fn run_id_from_stdout(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("Run ID: "))
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn plan_uses_mock_provider_and_writes_replayable_artifacts() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_mock_provider(json!({
        "steps": ["Replace the first line in hello.txt."],
        "risks": [],
        "files_likely_affected": ["hello.txt"],
        "tests_to_run": [],
        "assumptions": []
    }));

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "plan",
            "--editable",
            "hello.txt",
            "Update the greeting in hello.txt",
        ])
        .assert()
        .success()
        .stdout(contains("Steps: 1"))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let run_id = run_id_from_stdout(&stdout);
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    assert!(request.starts_with("POST /chat/completions "));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-key"));
    assert!(request.contains("Generate a production implementation plan"));
    assert!(request.contains("hello.txt"));

    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    let packet: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
    )
    .unwrap();
    let plan: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("plan.json")).unwrap()).unwrap();

    assert_eq!(packet["mode"], "plan");
    assert_eq!(packet["provider"], "glm");
    assert_eq!(packet["model"], "glm-5.1");
    assert_eq!(packet["packet_hash"].as_str().unwrap().len(), 64);
    assert_eq!(plan["steps"][0], "Replace the first line in hello.txt.");
    assert_eq!(plan["files_likely_affected"][0], "hello.txt");
    assert!(run_dir.join("plan.md").exists());
    assert!(run_dir.join("provider_request.redacted.json").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old greeting\n"
    );

    let artifacts = format!(
        "{}{}{}{}{}",
        std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("provider_request.redacted.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("response.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("plan.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("events.jsonl")).unwrap()
    );
    assert!(!artifacts.contains("test-key"));
}

#[test]
fn plan_uses_local_provider_capabilities_yaml_end_to_end() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello\n").unwrap();
    let capabilities_path = dir.path().join("local-provider.yaml");
    std::fs::write(
        &capabilities_path,
        local_provider_capabilities_yaml("local-openai", "local-model"),
    )
    .unwrap();
    let provider = start_mock_provider(json!({
        "steps": ["Use the local provider capability snapshot."],
        "risks": [],
        "files_likely_affected": ["app.txt"],
        "tests_to_run": [],
        "assumptions": ["local ProviderCapabilities YAML"]
    }));

    let mut doctor = Command::cargo_bin("mimir").unwrap();
    doctor
        .current_dir(dir.path())
        .env("MIMIR_PROVIDER_CAPABILITIES_PATH", &capabilities_path)
        .args(["providers", "doctor"])
        .assert()
        .success()
        .stdout(contains("local-openai").and(contains("local-model")));

    let mut cmd = Command::cargo_bin("mimir").unwrap();
    let output = cmd
        .current_dir(dir.path())
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env("MIMIR_PROVIDER_CAPABILITIES_PATH", &capabilities_path)
        .env("OPENAI_API_KEY", "test-key")
        .args([
            "plan",
            "--editable",
            "app.txt",
            "--provider",
            "local-openai",
            "--model",
            "local-model",
            "--base-url",
            &provider.url,
            "--json",
            "Plan with a local provider capability file",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let run_id = stdout["run_id"].as_str().unwrap();
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    assert!(request.starts_with("POST /chat/completions "));
    assert!(request.contains("\"model\":\"local-model\""));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-key"));

    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    let packet: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
    )
    .unwrap();
    let plan: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("plan.json")).unwrap()).unwrap();
    assert_eq!(packet["provider"], "local-openai");
    assert_eq!(packet["model"], "local-model");
    assert!(packet["capability_snapshot_ref"]
        .as_str()
        .unwrap()
        .contains("local-provider.yaml@sha256:"));
    assert_eq!(plan["provider"], "local-openai");
    assert_eq!(plan["model"], "local-model");
    assert!(run_dir.join("provider_request.redacted.json").exists());
    assert!(run_dir.join("response.json").exists());

    let mut replay = Command::cargo_bin("mimir").unwrap();
    replay
        .current_dir(dir.path())
        .env("MIMIR_PROVIDER_CAPABILITIES_PATH", &capabilities_path)
        .args(["packet", "replay", run_id])
        .assert()
        .success()
        .stdout(contains("Replaying packet"));

    let mut share = Command::cargo_bin("mimir").unwrap();
    share
        .current_dir(dir.path())
        .env("MIMIR_PROVIDER_CAPABILITIES_PATH", &capabilities_path)
        .args(["packet", "share", run_id])
        .assert()
        .success()
        .stdout(contains("local-openai").and(contains("local-model")));

    let artifacts = format!(
        "{}{}{}{}{}",
        std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("provider_request.redacted.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("response.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("plan.json")).unwrap(),
        std::fs::read_to_string(run_dir.join("events.jsonl")).unwrap()
    );
    assert!(!artifacts.contains("test-key"));
    assert!(!artifacts.contains("OPENAI_API_KEY"));
    assert!(!std::fs::read_to_string(&capabilities_path)
        .unwrap()
        .contains("test-key"));
}

#[test]
fn code_applies_provider_patch_with_safe_editable_set() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_mock_provider(json!({
        "schema_version": 1,
        "plan_id": "plan-apply",
        "steps": [{
            "action": "line_range",
            "path": "hello.txt",
            "start_line": 1,
            "end_line": 1,
            "content": "new greeting\n"
        }]
    }));

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--no-test",
            "Update the greeting in hello.txt",
        ])
        .assert()
        .success()
        .stdout(contains("Status: applied"))
        .stdout(contains("Patch report:"))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let run_id = run_id_from_stdout(&stdout);
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    assert!(request.starts_with("POST /chat/completions "));
    assert!(request.contains("hello.txt"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "new greeting\n"
    );

    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    let packet: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
    )
    .unwrap();
    let patch_recipe: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_recipe.json")).unwrap())
            .unwrap();
    let patch_report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_report.json")).unwrap())
            .unwrap();
    let events = std::fs::read_to_string(run_dir.join("events.jsonl")).unwrap();

    assert_eq!(patch_report["plan_id"], "plan-apply");
    assert_eq!(patch_report["applied"], true);
    assert_eq!(patch_report["test_policy"], "skipped_by_flag");
    assert_eq!(patch_recipe["packet_id"], packet["packet_id"]);
    assert!(run_dir.join("patch_plan.json").exists());
    assert!(run_dir.join("patch.diff").exists());
    assert!(!run_dir.join("backups/initial").exists());
    assert!(events.contains("provider_response"));
    assert!(events.contains("patch_applied"));
    assert!(!events.contains("test-key"));
}

#[test]
fn code_accepts_full_schema_patch_plan_bound_to_current_packet() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_packet_bound_patch_provider();

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--no-test",
            "Update the greeting in hello.txt",
        ])
        .assert()
        .success()
        .stdout(contains("Status: applied"))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let run_id = run_id_from_stdout(&stdout);
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(request.contains("Current packet_id: pkt-"));

    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "schema greeting\n"
    );

    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    let packet: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("context_packet.json")).unwrap(),
    )
    .unwrap();
    let patch_plan: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_plan.json")).unwrap())
            .unwrap();
    let patch_recipe: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_recipe.json")).unwrap())
            .unwrap();

    assert_eq!(patch_plan["packet_id"], packet["packet_id"]);
    assert_eq!(patch_plan["editable_target_set"][0], "hello.txt");
    assert_eq!(patch_plan["files_to_edit"][0]["path"], "hello.txt");
    assert!(patch_plan.get("steps").is_none());
    assert_eq!(patch_recipe["packet_id"], packet["packet_id"]);
    assert_eq!(patch_recipe["steps"][0]["path"], "hello.txt");
}

#[test]
fn code_rejects_full_schema_plan_id_mismatch() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_packet_bound_plan_id_mismatch_provider();

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--no-test",
            "Reject mismatched plan ids",
        ])
        .assert()
        .failure()
        .stderr(contains("plan_id mismatch"));

    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old greeting\n"
    );
}

#[test]
fn code_rejects_provider_patch_outside_editable_set() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_mock_provider(json!({
        "schema_version": 1,
        "plan_id": "plan-outside",
        "steps": [{
            "action": "create",
            "path": "outside.txt",
            "content": "not allowed\n"
        }]
    }));

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--no-test",
            "Try to edit a different file",
        ])
        .assert()
        .failure()
        .stderr(contains("patch rejected by safety validation"));
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    assert!(request.starts_with("POST /chat/completions "));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old greeting\n"
    );
    assert!(!dir.path().join("outside.txt").exists());

    let runs_root = dir.path().join(".mimir/runs");
    let run_dir = std::fs::read_dir(runs_root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(run_dir.join("context_packet.json").exists());
    assert!(run_dir.join("response.json").exists());
    assert!(run_dir.join("patch_plan.json").exists());
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("patch_report.json")).unwrap())
            .unwrap();
    assert_eq!(report["applied"], false);
    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("outside editable_target_set"));
}

#[test]
fn code_rejects_unified_diff_with_mismatched_hunk_counts() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_mock_provider(json!({
        "schema_version": 1,
        "plan_id": "plan-bad-hunk-count",
        "steps": [{
            "action": "unified_diff",
            "path": "hello.txt",
            "diff": "--- a/hello.txt\n+++ b/hello.txt\n@@ -1,1 +1,1 @@\n-old greeting\n+new greeting\n+extra greeting\n"
        }]
    }));

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--no-test",
            "Reject bad unified diff counts",
        ])
        .assert()
        .failure()
        .stderr(
            contains("mismatched line counts")
                .or(contains("exceeds declared line counts"))
                .or(contains("extra line after declared range")),
        );

    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old greeting\n"
    );
}

#[test]
fn code_rejects_unified_diff_with_unprefixed_hunk_line() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_mock_provider(json!({
        "schema_version": 1,
        "plan_id": "plan-bad-hunk-line",
        "steps": [{
            "action": "unified_diff",
            "path": "hello.txt",
            "diff": "--- a/hello.txt\n+++ b/hello.txt\n@@ -1,1 +1,1 @@\n-old greeting\nnew greeting\n"
        }]
    }));

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--no-test",
            "Reject bad unified diff hunk line",
        ])
        .assert()
        .failure()
        .stderr(contains("must start with space"));

    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old greeting\n"
    );
}

#[test]
fn code_repairs_failing_tests_with_bounded_provider_turn() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_mock_provider_sequence(vec![
        answer_patch_plan("plan-initial-bad", 1, 3),
        answer_patch_plan_with_tests("plan-repair-good", 3, 2, &["cargo test --lib"]),
    ]);

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "2",
            "--json",
            "Repair the answer implementation",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);

    assert_eq!(report["test_passed"], true);
    assert_eq!(report["repair"]["converged"], true);
    assert_eq!(report["repair"]["turns_executed"], 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        answer_fixture_source(2, 2).trim_end()
    );
    assert!(run_dir.join("repair_request.redacted.turn-1.json").exists());
    assert!(run_dir.join("repair_response.turn-1.json").exists());
    assert!(run_dir.join("repair_patch_plan.turn-1.json").exists());
    assert!(run_dir.join("repair_patch_recipe.turn-1.json").exists());
    assert!(run_dir.join("repair_patch.turn-1.diff").exists());
    assert!(run_dir.join("repair_summary.json").exists());
    assert!(!run_dir.join("backups/initial").exists());
    assert!(!run_dir.join("backups/repair-1").exists());
    assert!(run_dir.join("test_result.turn-1.json").exists());
    let repair_test: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(run_dir.join("test_result.turn-1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        repair_test["repair_suggested_commands"][0],
        "cargo test --lib"
    );
    assert!(
        repair_test["skipped_repair_suggested_commands"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("repair-suggested")
    );

    let first_request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let repair_request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(first_request.contains("Propose a safe patch"));
    assert!(repair_request.contains("Repair the previously applied Mimir patch"));
    assert!(repair_request.contains("Tests failed"));
}

#[test]
fn code_aborts_repair_before_provider_call_when_tests_mutate_run_owned_file() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture_with_mutating_test(&dir, 1, 2);
    let provider = start_mock_provider(answer_patch_plan("plan-initial-bad", 1, 3));

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "1",
            "--json",
            "Abort repair when tests dirty a Mimir-owned file",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout_text.contains("test-key"));
    assert!(!stderr_text.contains("test-key"));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);

    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("dirty_worktree"));
    assert!(report["repair"]["stop_reason"]
        .as_str()
        .unwrap()
        .contains("repair_run_owned_files_changed"));
    assert!(!run_dir.join("repair_request.redacted.turn-1.json").exists());
    assert!(!run_dir.join("repair_response.turn-1.json").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        "pub fn answer() -> i32 { 42 }"
    );

    let first_request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(first_request.contains("Propose a safe patch"));
    assert!(provider
        .requests
        .recv_timeout(Duration::from_millis(200))
        .is_err());

    for entry in std::fs::read_dir(&run_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains("test-key"),
                "{} leaked provider key",
                path.display()
            );
        }
    }
}

#[test]
fn code_aborts_repair_before_apply_when_external_process_mutates_run_owned_file() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_repair_provider_that_mutates_owned_file(
        dir.path().join("src/lib.rs"),
        "pub fn answer() -> i32 { 77 }\n".to_string(),
    );

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "1",
            "--json",
            "Abort repair apply when an external process dirties the file",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout_text.contains("test-key"));
    assert!(!stderr_text.contains("test-key"));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);

    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("dirty_worktree"));
    assert!(report["repair"]["stop_reason"]
        .as_str()
        .unwrap()
        .contains("repair_run_owned_files_changed_before_apply"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        "pub fn answer() -> i32 { 77 }"
    );
    assert!(run_dir.join("repair_request.redacted.turn-1.json").exists());
    assert!(run_dir.join("repair_response.turn-1.json").exists());
    assert!(run_dir.join("repair_patch_plan.turn-1.json").exists());
    assert!(run_dir.join("repair_patch_recipe.turn-1.json").exists());
    assert!(run_dir.join("repair_patch.turn-1.diff").exists());
    assert!(!run_dir.join("test_result.turn-1.json").exists());
    assert!(!run_dir.join("backups/repair-1").exists());

    let first_request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let repair_request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(first_request.contains("Propose a safe patch"));
    assert!(repair_request.contains("Repair the previously applied Mimir patch"));

    for entry in std::fs::read_dir(&run_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains("test-key"),
                "{} leaked provider key",
                path.display()
            );
            assert!(
                !text.contains("pub fn answer() -> i32 { 77 }"),
                "{} persisted externally mutated content",
                path.display()
            );
        }
    }
}

#[test]
fn code_fails_closed_when_max_repair_turns_reached() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_mock_provider_sequence(vec![
        answer_patch_plan("plan-initial-bad", 1, 3),
        answer_patch_plan("plan-repair-still-bad", 3, 4),
    ]);

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "1",
            "--json",
            "Try one repair turn",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["test_passed"], false);
    assert_eq!(report["repair"]["converged"], false);
    assert_eq!(report["repair"]["turns_executed"], 1);
    assert_eq!(report["repair"]["stop_reason"], "max_repair_turns_reached");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        answer_fixture_source(4, 2).trim_end()
    );
}

#[test]
fn code_stops_repair_before_apply_when_cost_cap_is_hit() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_mock_provider_sequence(vec![
        answer_patch_plan("plan-initial-bad", 1, 3),
        answer_patch_plan("plan-repair-good", 3, 2),
    ]);

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "2",
            "--cost-cap",
            "0.0026",
            "--json",
            "Repair should be stopped by cost",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["repair"]["converged"], false);
    assert!(report["repair"]["stop_reason"]
        .as_str()
        .unwrap()
        .contains("cost_cap"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        answer_fixture_source(3, 2).trim_end()
    );
}

#[test]
fn code_stops_before_initial_provider_call_when_cost_cap_is_hit() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();
    let provider = start_mock_provider(answer_patch_plan("plan-never-called", 1, 2));

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--cost-cap",
            "0",
            "--json",
            "Do not call provider when preflight cost exceeds cap",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(provider
        .requests
        .recv_timeout(Duration::from_millis(200))
        .is_err());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["applied"], false);
    assert_eq!(report["test_policy"], "not_run_cost_cap");
    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("initial_cost_cap_preflight_exceeded"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old greeting\n"
    );
}

#[test]
fn code_rejects_repair_patch_outside_editable_set() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_mock_provider_sequence(vec![
        answer_patch_plan("plan-initial-bad", 1, 3),
        json!({
            "schema_version": 1,
            "plan_id": "plan-repair-outside",
            "steps": [{
                "action": "create",
                "path": "outside.txt",
                "content": "not allowed\n"
            }]
        }),
    ]);

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "1",
            "--json",
            "Reject unsafe repair",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["repair"]["converged"], false);
    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("repair_patch_rejected"));
    assert!(!dir.path().join("outside.txt").exists());
}

#[test]
fn code_fails_closed_on_malformed_repair_json_with_artifacts() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_mock_provider_content_sequence(vec![
        answer_patch_plan("plan-initial-bad", 1, 3).to_string(),
        "{not-json".to_string(),
    ]);

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "1",
            "--json",
            "Reject malformed repair",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);

    assert_eq!(report["repair"]["converged"], false);
    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("repair_patch_malformed"));
    assert!(run_dir.join("repair_summary.json").exists());
    assert!(run_dir.join("patch_report.json").exists());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        answer_fixture_source(3, 2).trim_end()
    );
}

#[test]
fn code_rejects_secret_like_repair_patch_without_leaking() {
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let leaked = "OPENAI_API_KEY=sk-12345678901234567890";
    let repair = json!({
        "schema_version": 1,
        "plan_id": "plan-repair-secret",
        "steps": [{
            "action": "unified_diff",
            "path": "src/lib.rs",
            "diff": format!(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn answer() -> i32 {{ 3 }}\n+pub fn answer() -> &'static str {{ \"{leaked}\" }}\n"
            )
        }]
    });
    let provider =
        start_mock_provider_sequence(vec![answer_patch_plan("plan-initial-bad", 1, 3), repair]);

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--max-repair-turns",
            "1",
            "--json",
            "Reject secret repair",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);

    assert!(report["rejected"]
        .as_str()
        .unwrap()
        .contains("secret-like text"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        answer_fixture_source(3, 2).trim_end()
    );

    for entry in std::fs::read_dir(&run_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(
                !text.contains(leaked),
                "{} leaked repair secret",
                path.display()
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn code_rejects_symlink_backed_editable_before_backup() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "external secret\n").unwrap();
    symlink(outside.path(), dir.path().join("link")).unwrap();

    let provider = start_mock_provider(json!({
        "schema_version": 1,
        "plan_id": "plan-symlink",
        "steps": [{
            "action": "whole_file",
            "path": "link/secret.txt",
            "content": "replacement\n"
        }]
    }));

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "link/secret.txt",
            "--no-test",
            "Reject symlink-backed editable",
        ])
        .assert()
        .failure()
        .stderr(contains("symlink-backed editable path"));

    let run_dir = std::fs::read_dir(dir.path().join(".mimir/runs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(!run_dir.join("backups").exists());
    assert_eq!(
        std::fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
        "external secret\n"
    );
}

#[test]
fn code_refuses_dirty_git_target_with_spaces() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/foo bar.txt"), "hello\n").unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["add", "src/foo bar.txt"])
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
    std::fs::write(dir.path().join("src/foo bar.txt"), "dirty\n").unwrap();

    let provider = start_mock_provider(json!({
        "schema_version": 1,
        "plan_id": "plan-dirty-space",
        "steps": [{
            "action": "whole_file",
            "path": "src/foo bar.txt",
            "content": "patched\n"
        }]
    }));

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/foo bar.txt",
            "--no-test",
            "Patch a dirty file with spaces",
        ])
        .assert()
        .failure()
        .stderr(contains("dirty_worktree"));

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/foo bar.txt")).unwrap(),
        "dirty\n"
    );
}

#[test]
fn code_refuses_dirty_untracked_nested_target_inside_untracked_dir() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src/generated")).unwrap();
    std::fs::write(dir.path().join("src/generated/new.txt"), "dirty\n").unwrap();
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let provider = start_mock_provider(json!({
        "schema_version": 1,
        "plan_id": "plan-dirty-untracked-nested",
        "steps": [{
            "action": "whole_file",
            "path": "src/generated/new.txt",
            "content": "patched\n"
        }]
    }));

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/generated/new.txt",
            "--no-test",
            "Patch an untracked nested file",
        ])
        .assert()
        .failure()
        .stderr(contains("dirty_worktree"));

    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/generated/new.txt")).unwrap(),
        "dirty\n"
    );
}

fn write_cargo_answer_fixture(dir: &TempDir, answer: i32, expected: i32) {
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"mimir-repair-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        answer_fixture_source(answer, expected),
    )
    .unwrap();
}

fn write_cargo_answer_fixture_with_mutating_test(dir: &TempDir, answer: i32, expected: i32) {
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"mimir-repair-dirty-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        format!(
            "pub fn answer() -> i32 {{ {answer} }}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n\n    #[test]\n    fn mutates_source_then_fails() {{\n        std::fs::write(\"src/lib.rs\", \"pub fn answer() -> i32 {{ 42 }}\\n\").unwrap();\n        assert_eq!(answer(), {expected});\n    }}\n}}\n"
        ),
    )
    .unwrap();
}

fn answer_fixture_source(answer: i32, expected: i32) -> String {
    format!(
        "pub fn answer() -> i32 {{ {answer} }}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n\n    #[test]\n    fn answer_matches_expected() {{\n        assert_eq!(answer(), {expected});\n    }}\n}}\n"
    )
}

fn local_provider_capabilities_yaml(provider: &str, model: &str) -> String {
    format!(
        r#"schema_version: 1
provider: {provider}
models:
  {model}:
    max_context_tokens: 32768
    max_input_tokens: 28000
    max_output_tokens: 4096
    output_reserve_tokens: 1024
    counts_system_tokens: true
    counts_tool_schemas: true
    counts_tool_results: true
    counts_reasoning_tokens: true
    supports_server_token_count: false
    supports_prompt_cache: false
    overflow_behavior: validation_error
    pricing:
      input_per_million: 0.1
      output_per_million: 0.2
"#
    )
}

fn answer_patch_plan(plan_id: &str, from: i32, to: i32) -> serde_json::Value {
    json!({
        "schema_version": 1,
        "plan_id": plan_id,
        "steps": [{
            "action": "unified_diff",
            "path": "src/lib.rs",
            "diff": format!(
                "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-pub fn answer() -> i32 {{ {from} }}\n+pub fn answer() -> i32 {{ {to} }}\n"
            )
        }]
    })
}

fn answer_patch_plan_with_tests(
    plan_id: &str,
    from: i32,
    to: i32,
    tests_to_run: &[&str],
) -> serde_json::Value {
    json!({
        "patch_recipe": answer_patch_plan(plan_id, from, to),
        "tests_to_run": tests_to_run,
    })
}
