//! Secret redaction on ALL outbound provider calls (DOD: secrets never leave the
//! machine in the clear).
//!
//! Every provider-bound CLI path persists a redacted snapshot of the request it
//! dispatched to `.mimir/runs/<id>/provider_request.redacted.json`. These tests
//! plant a *synthetic* secret into each path's input/context, drive the command
//! against a localhost mock (the sanctioned pattern from
//! `provider_plan_code.rs`), then read the persisted redacted request artifact
//! and assert two things hold simultaneously:
//!   1. the planted secret *value* never appears in the artifact, and
//!   2. a `<REDACTED:...>` marker is present where the secret used to be.
//!
//! No real provider call is made — the model is a `TcpListener` bound to
//! `127.0.0.1:0` returning an OpenAI-compatible JSON body, and the only secret
//! material is synthetic (fake `sk-ant-...` / `AKIA...` shaped strings).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use serde_json::json;
use tempfile::TempDir;

/// Synthetic Anthropic-shaped key planted into retrieval-included file *paths*.
/// Matches the `ANTHROPIC_KEY` redactor pattern (`sk-ant-[a-zA-Z0-9-]+`).
const ANTHROPIC_SECRET: &str = "sk-ant-api03-OUTBOUNDPLANTEDsecretABCDEF1234567890";
/// Synthetic AWS-shaped key planted into a tampered context packet task goal.
/// Matches the `AWS_KEY` redactor pattern (`AKIA[0-9A-Z]{16}`) in full.
const AWS_SECRET: &str = "AKIAIOSFODNN7EXAMPLE";

struct MockProvider {
    url: String,
    requests: Receiver<String>,
}

/// Start a one-shot localhost mock returning `content` as the assistant message.
/// Captures the raw HTTP request bytes it received so callers can assert no
/// secret reached the wire in the clear if they wish.
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

fn read_request_artifact(dir: &TempDir, run_id: &str) -> String {
    let path = dir
        .path()
        .join(".mimir/runs")
        .join(run_id)
        .join("provider_request.redacted.json");
    std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "missing redacted request artifact {}: {err}",
            path.display()
        )
    })
}

/// Assert the persisted outbound request artifact (a) never contains the planted
/// secret value and (b) carries the expected `<REDACTED:...>` marker in its place.
fn assert_redacted_request(artifact: &str, planted_secret: &str, marker: &str) {
    assert!(
        !artifact.contains(planted_secret),
        "outbound request artifact leaked planted secret {planted_secret:?}:\n{artifact}"
    );
    assert!(
        artifact.contains(marker),
        "outbound request artifact is missing redaction marker {marker:?}:\n{artifact}"
    );
    // The bearer credential the CLI was pointed at must never be persisted either.
    assert!(
        !artifact.contains("test-key"),
        "outbound request artifact leaked provider api key:\n{artifact}"
    );
}

/// Plant a secret-*named* file whose benign content defines `symbol`, so the
/// retrieval pipeline includes it when the task mentions `symbol` — forcing the
/// secret-bearing path into the outbound provider request, where it must be
/// redacted. Returns the planted relative path.
fn plant_retrievable_secret_named_file(dir: &TempDir, symbol: &str) -> String {
    let secret_named = format!("config_{ANTHROPIC_SECRET}.py");
    let body = format!("def {symbol}():\n    return {{\"widget\": True}}\n").repeat(8);
    std::fs::write(dir.path().join(&secret_named), body).unwrap();
    secret_named
}

#[test]
fn ask_redacts_secret_named_context_path_in_outbound_request() {
    let dir = TempDir::new().unwrap();
    let symbol = "parse_widget_settings";
    let secret_named = plant_retrievable_secret_named_file(&dir, symbol);
    // The secret value lives only in the file *name*; the planted key must not
    // round-trip into the request artifact in the clear.
    assert!(secret_named.contains(ANTHROPIC_SECRET));

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
    // The mock genuinely received an outbound request for this run.
    assert!(request.starts_with("POST /chat/completions "));

    let artifact = read_request_artifact(&dir, &run_id);
    assert_redacted_request(&artifact, ANTHROPIC_SECRET, "<REDACTED:ANTHROPIC_KEY>");
}

#[test]
fn plan_redacts_secret_named_context_path_in_outbound_request() {
    let dir = TempDir::new().unwrap();
    let symbol = "parse_widget_settings";
    plant_retrievable_secret_named_file(&dir, symbol);
    // A benign, safe editable target so `plan` is fully reachable via the mock.
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

    let artifact = read_request_artifact(&dir, &run_id);
    assert_redacted_request(&artifact, ANTHROPIC_SECRET, "<REDACTED:ANTHROPIC_KEY>");
}

#[test]
fn code_redacts_secret_named_editable_path_in_outbound_request() {
    let dir = TempDir::new().unwrap();
    // A secret-*named* file is added to the editable target set; the `code`
    // prompt echoes the editable target set verbatim, so the planted secret
    // flows outbound and must be redacted in the persisted request artifact.
    let secret_named = format!("legacy_{ANTHROPIC_SECRET}.txt");
    std::fs::write(dir.path().join(&secret_named), "old greeting\n").unwrap();
    std::fs::write(dir.path().join("hello.txt"), "old greeting\n").unwrap();

    // The model proposes a safe patch that only touches the benign file.
    let patch = json!({
        "schema_version": 1,
        "plan_id": "plan-redaction",
        "steps": [{
            "action": "whole_file",
            "path": "hello.txt",
            "content": "new greeting\n"
        }]
    });
    let provider = start_mock_provider(&patch.to_string());

    let output = mimir_cmd(&dir, &provider.url)
        .args([
            "code",
            "--editable",
            "hello.txt",
            "--editable",
            &secret_named,
            "--no-test",
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

    let artifact = read_request_artifact(&dir, &run_id);
    assert_redacted_request(&artifact, ANTHROPIC_SECRET, "<REDACTED:ANTHROPIC_KEY>");
}

#[test]
fn context_call_redacts_secret_in_outbound_request() {
    let dir = TempDir::new().unwrap();
    std::fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    // 1) Build a benign replayable packet (no secret yet — the build-time guard
    //    would otherwise reject secret-like task text).
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

    // 2) Plant a synthetic secret into the saved packet's task goal and re-seal
    //    the integrity hash so `context call` reconstructs an outbound prompt
    //    carrying the secret. This exercises the outbound-redaction last line of
    //    defense for the context-driven path.
    let mut packet: mimir_schemas::ContextPacket =
        serde_json::from_str(&std::fs::read_to_string(&packet_path).unwrap()).unwrap();
    packet.task_card.goal = format!("Investigate credential {AWS_SECRET} in main.rs");
    packet.packet_hash = mimir_context::hash_packet(&packet);
    std::fs::write(&packet_path, serde_json::to_vec_pretty(&packet).unwrap()).unwrap();

    // 3) Dispatch the call against the localhost mock.
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

    let artifact = read_request_artifact(&dir, &run_id);
    assert_redacted_request(&artifact, AWS_SECRET, "<REDACTED:AWS_KEY>");
}
