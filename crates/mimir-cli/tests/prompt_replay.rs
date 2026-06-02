//! Exit gate: "All prompts replayable from local artifacts".
//!
//! Every provider-bound CLI path (`ask`, `plan`, `code`, `context call`) persists
//! the request it dispatched to `.mimir/runs/<id>/provider_request.redacted.json`
//! via `write_provider_request_artifact`. `mimir packet replay <run-id>
//! --request-json` resolves to `mimir_session::packet::replay_request_bytes_for_run`,
//! which returns the SAVED redacted request artifact byte-for-byte whenever that
//! artifact exists (only falling back to reconstruction when it is absent).
//!
//! These tests drive each of the four commands against a localhost mock (the
//! sanctioned pattern cloned from `outbound_redaction.rs`), confirm the redacted
//! request artifact was persisted, then run the replay command and assert its
//! stdout is byte-IDENTICAL to the saved artifact. The gate fails if any command
//! stops persisting a replayable request, or if replay diverges from what was
//! saved.
//!
//! No real provider call is made — the model is a `TcpListener` bound to
//! `127.0.0.1:0` returning an OpenAI-compatible JSON body.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;

struct MockProvider {
    url: String,
    requests: Receiver<String>,
}

/// Start a one-shot localhost mock returning `content` as the assistant message.
fn start_mock_provider(content: &str) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    let body = json!({
        "model": "glm-5.1",
        "choices": [{
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": 17, "completion_tokens": 5 }
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
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn content_length(headers: &str) -> usize {
    headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
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

/// Absolute path to the persisted redacted request artifact for `run_id`.
fn request_artifact_path(dir: &TempDir, run_id: &str) -> std::path::PathBuf {
    dir.path()
        .join(".mimir/runs")
        .join(run_id)
        .join("provider_request.redacted.json")
}

/// Read the persisted redacted request artifact bytes, asserting it exists.
fn read_request_artifact_bytes(dir: &TempDir, run_id: &str) -> Vec<u8> {
    let path = request_artifact_path(dir, run_id);
    std::fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "missing redacted request artifact {}: {err}",
            path.display()
        )
    })
}

/// Run `mimir packet replay <run_id> --request-json` in the workspace and return
/// its stdout bytes. The replay command resolves its workspace root from the
/// process cwd (`Utf8Path::new(".")`), so it MUST run with `current_dir` set to
/// the workspace — `mimir_cmd` already does this.
fn replay_request_json_bytes(dir: &TempDir, run_id: &str) -> Vec<u8> {
    // The provider URL is irrelevant for replay (no network is touched); any
    // unreachable address keeps env setup identical to the producing command.
    let assert = mimir_cmd(dir, "http://127.0.0.1:1")
        .args(["packet", "replay", run_id, "--request-json"])
        .assert()
        .success();
    assert.get_output().stdout.clone()
}

/// Assert that replaying `run_id` reproduces the persisted artifact byte-for-byte.
fn assert_replay_matches_artifact(dir: &TempDir, run_id: &str) {
    let saved = read_request_artifact_bytes(dir, run_id);
    let replayed = replay_request_json_bytes(dir, run_id);
    assert_eq!(
        replayed,
        saved,
        "replay --request-json for run {run_id} diverged from the saved \
         provider_request.redacted.json artifact ({} replayed bytes vs {} saved bytes)",
        replayed.len(),
        saved.len()
    );
}

/// Plant a benign file defining `symbol` so retrieval includes it when the task
/// mentions `symbol`, guaranteeing a non-empty, prompt-bearing provider request.
fn plant_retrievable_symbol_file(dir: &TempDir, symbol: &str) {
    let body = format!("def {symbol}():\n    return {{\"widget\": True}}\n").repeat(8);
    std::fs::write(dir.path().join("widget.py"), body).unwrap();
}

#[test]
fn ask_request_replays_from_saved_artifact() {
    let dir = TempDir::new().unwrap();
    let symbol = "parse_widget_settings";
    plant_retrievable_symbol_file(&dir, symbol);

    let provider = start_mock_provider("answer");
    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "ask",
            &format!("How is {symbol} used across the repository?"),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run_id = run_id_from_stdout(&String::from_utf8(output).unwrap());
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(request.starts_with("POST /chat/completions "));

    assert_replay_matches_artifact(&dir, &run_id);
}

#[test]
fn plan_request_replays_from_saved_artifact() {
    let dir = TempDir::new().unwrap();
    let symbol = "parse_widget_settings";
    plant_retrievable_symbol_file(&dir, symbol);
    std::fs::write(dir.path().join("editme.py"), "def use():\n    return 1\n").unwrap();

    let plan = json!({
        "steps": ["Audit the widget settings parser."],
        "risks": [],
        "files_likely_affected": ["editme.py"],
        "tests_to_run": [],
        "assumptions": []
    });
    let provider = start_mock_provider(&plan.to_string());

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "plan",
            "--editable",
            "editme.py",
            &format!("Refactor how {symbol} is consumed"),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run_id = run_id_from_stdout(&String::from_utf8(output).unwrap());
    let _ = provider.requests.recv_timeout(Duration::from_secs(5));

    assert_replay_matches_artifact(&dir, &run_id);
}

#[test]
fn code_request_replays_from_saved_artifact() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();

    let patch = json!({
        "schema_version": 1,
        "plan_id": "plan-replay",
        "steps": [{
            "action": "whole_file",
            "path": "hello.txt",
            "content": "new greeting\n"
        }]
    });
    let provider = start_mock_provider(&patch.to_string());

    // `--dry-run` validates the patch without applying it, so the editable file
    // is left byte-for-byte unchanged. Replay re-verifies included-source hashes
    // against the working tree, so an applied patch would (correctly) make the
    // run no longer replayable; the persisted request artifact itself is written
    // before any apply step, so the gate it asserts still holds.
    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--no-test",
            "--dry-run",
            "Update the greeting in hello.txt",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let run_id = run_id_from_stdout(&String::from_utf8(output).unwrap());
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(request.contains("Propose a safe patch"));

    assert_replay_matches_artifact(&dir, &run_id);
}

#[test]
fn context_call_request_replays_from_saved_artifact() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    // Build a benign replayable packet (no provider call yet).
    mimir_cmd(&dir, "http://127.0.0.1:1")
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
    let run_id = run_dir.file_name().unwrap().to_string_lossy().to_string();
    let packet_path = run_dir.join("context_packet.json");

    // Dispatch the call against the localhost mock; this persists the redacted
    // provider request artifact for the same run.
    let provider = start_mock_provider("answer");
    mimir_cmd(&dir, &provider.url)
        .args(["context", "call", packet_path.to_str().unwrap()])
        .assert()
        .success();
    let request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(request.starts_with("POST /chat/completions "));

    assert_replay_matches_artifact(&dir, &run_id);
}
