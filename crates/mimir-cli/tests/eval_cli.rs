use assert_cmd::Command;
use serde_json::json;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

struct MockProvider {
    url: String,
    requests: Receiver<String>,
}

fn start_mock_provider(content: &str, request_count: usize) -> MockProvider {
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
            "prompt_tokens": 41,
            "completion_tokens": 3
        }
    })
    .to_string();

    thread::spawn(move || {
        for _ in 0..request_count {
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

fn write_answer_eval_fixture(root: &Path, gold_answer: &str) -> (PathBuf, PathBuf) {
    let repo = root.join("answer-repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("reference.md"),
        format!("The fixture answer is `{gold_answer}`.\n"),
    )
    .unwrap();
    let dataset = root.join("answer-eval.yaml");
    std::fs::write(
        &dataset,
        format!(
            r#"schema_version: 1
id: cli-answer-smoke
description: Small answer-quality CLI smoke dataset.
cases:
  - id: answer-smoke-case
    repo_path: "{}"
    base_commit: synthetic
    task: "Answer with the fixture answer exactly."
    gold_answer: "{}"
    grading: exact_match
"#,
            repo.display(),
            gold_answer
        ),
    )
    .unwrap();
    (repo, dataset)
}

fn answer_eval_cmd(current_dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(current_dir)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("MIMIR_PROVIDER")
        .env_remove("MIMIR_MODEL")
        .env_remove("MIMIR_BASE_URL");
    cmd
}

#[test]
fn eval_context_dataset_outputs_schema_valid_results() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let temp = tempfile::TempDir::new().unwrap();
    let fixture_repo = temp.path().join("repo");
    std::fs::create_dir_all(&fixture_repo).unwrap();
    std::fs::write(
        fixture_repo.join("ContextBuilder.rs"),
        "pub struct ContextBuilder;\n",
    )
    .unwrap();
    let dataset = temp.path().join("eval.yaml");
    std::fs::write(
        &dataset,
        format!(
            r#"schema_version: 1
id: cli-smoke
description: Small CLI smoke dataset that avoids full-repository eval work.
cases:
  - schema_version: 1
    id: cli-smoke-case
    repo_path: "{}"
    base_commit: synthetic
    task: Explain ContextBuilder
    gold:
      files: [ContextBuilder.rs]
      ranges:
        - {{ path: ContextBuilder.rs, start: 1, end: 1 }}
    allowed_mode: ask
    allowed_caps_to_test: [64000]
  - schema_version: 1
    id: cli-smoke-case-repeat
    repo_path: "{}"
    base_commit: synthetic
    task: Explain ContextBuilder again
    gold:
      files: [ContextBuilder.rs]
      ranges:
        - {{ path: ContextBuilder.rs, start: 1, end: 1 }}
    allowed_mode: ask
    allowed_caps_to_test: [64000]
"#,
            fixture_repo.display(),
            fixture_repo.display()
        ),
    )
    .unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../schemas/EvalResult.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let output_path = temp.path().join("eval_results.json");

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(&repo_root)
        .args([
            "eval",
            "context",
            "--dataset",
            dataset.to_str().unwrap(),
            "--output",
            output_path.to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = results.as_array().expect("eval stdout should be an array");
    assert_eq!(results.len(), 2);
    for result in results {
        assert!(validator.is_valid(result), "invalid EvalResult: {result:#}");
        assert_eq!(result["metrics"]["cap_compliance"], true);
        assert!(result["metrics"]["repo_map_refresh_latency_ms"].is_number());
    }
    assert_eq!(results[0]["metrics"]["index_cache_hit_rate"], 0.0);
    assert_eq!(results[1]["metrics"]["index_cache_hit_rate"], 0.5);
}

#[test]
fn eval_answer_without_key_reports_offline_savings_and_skips_grading() {
    let temp = tempfile::TempDir::new().unwrap();
    let (_repo, dataset) = write_answer_eval_fixture(temp.path(), "fixture-answer");

    let output = answer_eval_cmd(temp.path())
        .args([
            "eval",
            "answer",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
            "--dataset",
            dataset.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dataset_id"], "cli-answer-smoke");
    assert_eq!(report["answer_grading"], "not_run");
    assert!(report["totals"]["tokens_saved"].is_number());
    assert!(String::from_utf8_lossy(&output.stderr).contains("answer grading skipped"));
}

#[test]
fn eval_answer_with_mock_provider_dispatches_and_grades_both_arms() {
    let temp = tempfile::TempDir::new().unwrap();
    let (_repo, dataset) = write_answer_eval_fixture(temp.path(), "fixture-answer");
    let provider = start_mock_provider("fixture-answer", 2);

    let output = answer_eval_cmd(temp.path())
        .env("GLM_API_KEY", "test-key")
        .env("GLM_BASE_URL", &provider.url)
        .env("GLM_MODEL", "glm-5.1")
        .args([
            "eval",
            "answer",
            "--provider",
            "glm",
            "--model",
            "glm-5.1",
            "--dataset",
            dataset.to_str().unwrap(),
            "--compare",
            "verbatim,compressed",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let first_request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    let second_request = provider
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert!(first_request.starts_with("POST /chat/completions "));
    assert!(second_request.starts_with("POST /chat/completions "));

    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let grading = &report["answer_grading"];
    assert_eq!(grading["dataset_id"], "cli-answer-smoke");
    assert_eq!(grading["arms"]["verbatim"]["cases"], 1);
    assert_eq!(grading["arms"]["compressed"]["cases"], 1);
    assert_eq!(grading["arms"]["verbatim"]["accuracy"], 1.0);
    assert_eq!(grading["arms"]["compressed"]["accuracy"], 1.0);
    assert_eq!(grading["arms"]["verbatim"]["mean_tokens_in"], 41.0);
    assert_eq!(grading["arms"]["compressed"]["mean_tokens_in"], 41.0);
    assert_eq!(grading["deltas"]["accuracy_delta"], 0.0);
    assert_eq!(grading["deltas"]["mean_tokens_in_saved"], 0.0);
    assert!(String::from_utf8_lossy(&output.stderr).contains("answer-quality live summary"));
}
