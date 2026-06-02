//! End-to-end journey tests for the `ask` and `code` commands.
//!
//! Both journeys run the real `mimir` binary against a localhost TCP mock
//! provider (the same pattern used by `provider_plan_code.rs`): the binary
//! resolves the `glm` provider from the `GLM_API_KEY` / `GLM_BASE_URL` /
//! `GLM_MODEL` environment variables set by [`mimir_cmd`], so every request is
//! served by a synthetic in-process server. No real provider or network call is
//! ever made, and only synthetic secrets are used.
//!
//! - The ASK journey asserts a viewable context packet artifact is produced.
//! - The CODE journey asserts a full round-trip: a provider patch is applied,
//!   the auto-detected test suite runs and PASSES, and the run reports no
//!   blockers. It also asserts that `code` still fails closed without
//!   `--editable`, so this test breaks if either invariant regresses.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use predicates::str::contains;
use serde_json::json;
use tempfile::TempDir;

struct MockProvider {
    url: String,
    requests: Receiver<String>,
}

/// Spawns a single-shot localhost provider that answers one `/chat/completions`
/// request with the supplied assistant `content`. The raw request is forwarded
/// over the channel so tests can assert on what the binary sent.
fn start_mock_provider(content: String) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let body = json!({
        "model": "glm-5.1",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": content
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

/// Builds a `mimir` command wired to the localhost mock through synthetic GLM
/// environment variables. Pre-existing provider env is cleared so the test is
/// hermetic regardless of the host environment.
fn mimir_cmd(dir: &TempDir, provider_url: &str) -> Command {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path())
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
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

/// Cargo fixture whose `answer()` returns `answer` while the test asserts it
/// equals `expected`. With `answer != expected` the suite fails until a patch
/// fixes the source; with `answer == expected` it passes immediately.
fn write_cargo_answer_fixture(dir: &TempDir, answer: i32, expected: i32) {
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"mimir-journey-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        answer_fixture_source(answer, expected),
    )
    .unwrap();
}

fn answer_fixture_source(answer: i32, expected: i32) -> String {
    format!(
        "pub fn answer() -> i32 {{ {answer} }}\n\n#[cfg(test)]\nmod tests {{\n    use super::*;\n\n    #[test]\n    fn answer_matches_expected() {{\n        assert_eq!(answer(), {expected});\n    }}\n}}\n"
    )
}

/// Patch recipe that rewrites the single `answer()` line from `from` to `to`.
fn answer_patch_recipe(plan_id: &str, from: i32, to: i32) -> serde_json::Value {
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

#[test]
fn ask_writes_viewable_context_packet_and_redacted_request() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("app.txt"), "hello world\n").unwrap();
    let provider = start_mock_provider("A concise synthetic answer.".to_string());

    let output = mimir_cmd(&dir, &provider.url)
        .args(["ask", "Summarize app.txt for me"])
        .assert()
        .success()
        .stdout(contains("Run ID: "))
        .stdout(contains("Packet:"))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let run_id = run_id_from_stdout(&stdout);

    // The binary actually dispatched to the mock provider.
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(request.starts_with("POST /chat/completions "));
    assert!(request
        .to_ascii_lowercase()
        .contains("authorization: bearer test-key"));

    let run_dir = dir.path().join(".mimir/runs").join(&run_id);

    // A viewable packet artifact exists, parses as a ContextPacket, and has a
    // positive token estimate — i.e. it is a real, inspectable packet.
    let packet_path = run_dir.join("context_packet.json");
    assert!(packet_path.exists(), "context_packet.json must be written");
    let packet_text = std::fs::read_to_string(&packet_path).unwrap();
    let packet: mimir_schemas::ContextPacket = serde_json::from_str(&packet_text).unwrap();
    assert_eq!(packet.run_id, run_id);
    assert!(
        packet.estimated_input_tokens > 0,
        "estimated_input_tokens must be positive, got {}",
        packet.estimated_input_tokens
    );

    // Cross-check the same field through an untyped read so the assertion does
    // not silently pass on a default-zero deserialization.
    let packet_json: serde_json::Value = serde_json::from_str(&packet_text).unwrap();
    assert!(packet_json["estimated_input_tokens"].as_u64().unwrap() > 0);

    // The redacted provider request is captured for replay/inspection.
    assert!(
        run_dir.join("provider_request.redacted.json").exists(),
        "provider_request.redacted.json must be written"
    );

    // Synthetic secret never lands in viewable artifacts.
    assert!(!packet_text.contains("test-key"));
    assert!(
        !std::fs::read_to_string(run_dir.join("provider_request.redacted.json"))
            .unwrap()
            .contains("test-key")
    );
}

#[test]
fn code_round_trip_applies_patch_runs_tests_and_reports_no_blockers() {
    let dir = TempDir::new().unwrap();
    // Source returns 1 but the test asserts 2 -> fails until the patch fixes it.
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_mock_provider(answer_patch_recipe("plan-journey", 1, 2).to_string());

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "src/lib.rs",
            "--json",
            "Fix answer() so the test passes",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);

    // The binary dispatched a code request against the mock provider.
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(30))
        .unwrap();
    assert!(request.starts_with("POST /chat/completions "));
    assert!(request.contains("Propose a safe patch"));

    // The edit was applied to the editable file.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs"))
            .unwrap()
            .trim_end(),
        answer_fixture_source(2, 2).trim_end()
    );

    // The patch report is the source of truth: applied, tests ran and PASSED,
    // and there are NO blockers (a rejection/blocker would populate `rejected`).
    let report_path = run_dir.join("patch_report.json");
    assert!(report_path.exists(), "patch_report.json must be written");
    let patch_report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).unwrap()).unwrap();
    assert_eq!(patch_report["plan_id"], "plan-journey");
    assert_eq!(patch_report["applied"], true);
    assert_eq!(patch_report["dry_run"], false);
    assert_eq!(
        patch_report["test_policy"], "auto_detected",
        "tests must actually run, not be skipped"
    );
    assert_eq!(
        patch_report["test_passed"], true,
        "a clean round-trip must pass its tests"
    );
    // No blockers on a clean run. If a clean run ever reports blockers, this
    // assertion (and the success exit asserted above) makes the test fail.
    assert!(
        patch_report["rejected"].is_null(),
        "a clean run must report no blockers, got rejected = {}",
        patch_report["rejected"]
    );

    // The applied event was recorded (not a rejection event).
    let events = std::fs::read_to_string(run_dir.join("events.jsonl")).unwrap();
    assert!(events.contains("patch_applied"));
    assert!(!events.contains("patch_tests_failed"));
    assert!(!events.contains("test-key"));
}

#[test]
fn code_fails_closed_without_editable() {
    // This guards the hard invariant: `mimir code` MUST require `--editable`.
    // If the requirement is ever removed, this test fails.
    let dir = TempDir::new().unwrap();
    write_cargo_answer_fixture(&dir, 1, 2);
    let provider = start_mock_provider(answer_patch_recipe("plan-journey", 1, 2).to_string());

    mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--no-test",
            "Edit without declaring an editable set",
        ])
        .assert()
        .failure()
        .stderr(contains("requires at least one --editable"));

    // Nothing was edited because the command refused to run.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        answer_fixture_source(1, 2)
    );
}
