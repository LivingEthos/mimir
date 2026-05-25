use assert_cmd::Command;
use predicates::{prelude::PredicateBooleanExt, str::contains};
use serde_json::{json, Value};
use tempfile::TempDir;

#[test]
fn init_seeds_project_workflow_files() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(contains("Created workflow files"));

    assert!(dir.path().join(".mimir/config.yaml").is_file());
    assert!(dir.path().join(".mimir/project-rules.md").is_file());
    assert!(dir
        .path()
        .join(".mimir/checks/no-provider-secrets.md")
        .is_file());
    assert!(dir.path().join(".mimir/commands/fast-check.md").is_file());
    assert!(dir
        .path()
        .join(".mimir/commands/release-check.md")
        .is_file());
}

#[test]
fn check_command_reports_source_controlled_findings_as_json() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/checks")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join(".mimir/checks/no-todos.md"),
        "# No TODOs\n\n- severity: error\n- forbidden: TODO\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "// TODO: fix\n").unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["check", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["checks_loaded"], 1);
    assert_eq!(value["passed"], false);
    assert_eq!(value["blocking_findings"], 1);
    assert_eq!(value["findings"].as_array().unwrap().len(), 1);
    let finding = &value["findings"][0];
    assert_eq!(finding["category"], "check:no-todos");
    assert_eq!(finding["severity"], "error");
    assert_eq!(finding["line_numbers"], serde_json::json!([1]));
    assert!(finding["paths"][0]
        .as_str()
        .unwrap()
        .ends_with("src/lib.rs"));
    assert!(finding["description"]
        .as_str()
        .unwrap()
        .contains("Forbidden pattern"));
    assert!(!dir.path().join(".mimir/runs").exists());

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["check", "--ci"])
        .assert()
        .failure();
}

#[test]
fn check_ci_warning_only_json_exits_success() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/checks")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join(".mimir/checks/soft-todos.md"),
        "# Soft TODOs\n\n- severity: warn\n- forbidden: TODO\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "// TODO: follow up\n").unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["check", "--ci", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["checks_loaded"], 1);
    assert_eq!(value["passed"], true);
    assert_eq!(value["blocking_findings"], 0);
    assert_eq!(value["findings"].as_array().unwrap().len(), 1);
    assert_eq!(value["findings"][0]["severity"], "warn");
}

#[test]
fn context_suggest_writes_packet_and_includes_guidance() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/checks")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join(".mimir/checks/no-todos.md"),
        "# No TODOs\n\n- severity: error\n- forbidden: TODO\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "# Agent guide\nRead me first.\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn feature() {}\n").unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["context", "suggest", "explain the feature", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["checks_loaded"], 1);
    let packet_path = value["packet_path"].as_str().unwrap();
    assert!(dir.path().join(packet_path).is_file());
    let guidance = value["guidance_files"].as_array().unwrap();
    assert!(guidance.iter().any(|path| path == "AGENTS.md"));
    let likely_files = value["likely_files"].as_array().unwrap();
    assert!(likely_files.iter().any(|path| path == "src/lib.rs"));
    assert!(!likely_files.iter().any(|path| path == "AGENTS.md"));

    let packet: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(packet_path)).unwrap()).unwrap();
    assert_eq!(packet["run_id"], value["run_id"]);
    assert_eq!(packet["packet_id"], value["packet_id"]);
    let included = packet["included"].as_array().unwrap();
    let source_item = included
        .iter()
        .find(|item| item["path"] == "src/lib.rs")
        .unwrap();
    assert!(source_item["tokens"].as_u64().unwrap() > 0);
    assert_eq!(source_item["source_hash"].as_str().unwrap().len(), 64);
    let guidance_item = included
        .iter()
        .find(|item| item["path"] == "AGENTS.md")
        .unwrap();
    assert_eq!(guidance_item["reason_code"], "manifest_reference");
    assert!(!dir
        .path()
        .join(".mimir/runs")
        .join(value["run_id"].as_str().unwrap())
        .join("provider_request.redacted.json")
        .exists());
}

#[test]
fn explore_persists_read_only_evidence() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/context.rs"),
        "pub fn build_context_packet() {}\n",
    )
    .unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["explore", "where is context built", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let evidence_path = value["evidence_path"].as_str().unwrap();
    assert!(dir.path().join(evidence_path).is_file());
    assert_eq!(value["evidence"]["subagent"], "search");
    assert_eq!(value["evidence"]["parent_run_id"], value["run_id"]);
    assert!(value["evidence"]["run_id"]
        .as_str()
        .unwrap()
        .starts_with("search-"));
    assert_eq!(value["evidence"]["cost_usd"], 0.0);
    assert!(value["evidence"]["tokens_consumed"].as_u64().unwrap() > 0);
    let relevant_paths = value["evidence"]["relevant_paths"].as_array().unwrap();
    assert!(relevant_paths.iter().any(|path| path == "src/context.rs"));
    let findings = value["evidence"]["findings"].as_array().unwrap();
    assert!(findings
        .iter()
        .any(|finding| finding.as_str().unwrap().contains("src/context.rs")));
    assert!(!findings
        .iter()
        .any(|finding| finding.as_str().unwrap().contains("Stub result")));
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join(evidence_path)).unwrap()).unwrap();
    assert_eq!(persisted, value["evidence"]);
}

#[test]
fn code_recipe_expands_parameters_and_persists_execution_artifact() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/commands")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn feature() {}\n").unwrap();
    std::fs::write(
        dir.path().join(".mimir/commands/focused.md"),
        r#"---
name: focused
description: Focus code mode on one changed file.
mode: code
retrieval:
  seeds:
    - "{{target}}"
  expand:
    - imports
allowed_tools:
  - run_command
output_budget_tokens: 1024
parameters:
  - name: target
    description: File to center retrieval on.
    required: true
---

Keep the patch centered on {{target}}.
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args([
            "code",
            "update the feature",
            "--editable",
            "src/lib.rs",
            "--recipe",
            "focused",
            "--param",
            "target=src/lib.rs",
            "--cost-cap=-1.0",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = report["run_id"].as_str().unwrap();
    let run_dir = dir.path().join(".mimir/runs").join(run_id);
    let recipe: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("command_recipe.json")).unwrap())
            .unwrap();
    assert_eq!(recipe["name"], "focused");
    assert_eq!(recipe["parameters"], json!({"target": "src/lib.rs"}));
    assert_eq!(recipe["retrieval"]["seeds"], json!(["src/lib.rs"]));
    assert!(recipe["expanded_body"]
        .as_str()
        .unwrap()
        .contains("src/lib.rs"));

    let packet: Value =
        serde_json::from_slice(&std::fs::read(run_dir.join("context_packet.json")).unwrap())
            .unwrap();
    let task = packet["task_card"]["goal"].as_str().unwrap();
    assert!(task.contains("Mimir command recipe: focused"));
    assert!(task.contains("Retrieval seeds:\n- src/lib.rs"));
    assert!(task.contains("Keep the patch centered on src/lib.rs."));
}

#[test]
fn code_recipe_requires_declared_parameters() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".mimir/commands")).unwrap();
    std::fs::write(
        dir.path().join(".mimir/commands/focused.md"),
        r#"---
name: focused
description: Focus code mode on one changed file.
mode: code
retrieval:
  seeds:
    - "{{target}}"
allowed_tools:
  - run_command
output_budget_tokens: 1024
parameters:
  - name: target
    description: File to center retrieval on.
    required: true
---

Use {{target}}.
"#,
    )
    .unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args([
            "code",
            "update",
            "--editable",
            "src/lib.rs",
            "--recipe",
            "focused",
        ])
        .assert()
        .failure()
        .stderr(contains(
            "schema_recipe_invalid: missing required recipe parameter `target`",
        ));
}

#[test]
fn agent_search_returns_real_local_evidence() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/search.rs"),
        "pub fn search_index() {}\n",
    )
    .unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["agent", "search", "where is search index"])
        .assert()
        .success()
        .stdout(contains("src/search.rs"))
        .stdout(predicates::str::is_match("Stub result").unwrap().not());
}

#[test]
fn agent_file_analyst_returns_pattern_evidence() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn feature() { thing.unwrap(); }\n",
    )
    .unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["agent", "file-analyst", "find panics"])
        .assert()
        .success()
        .stdout(contains("Subagent: file-analyst"))
        .stdout(contains("src/lib.rs"))
        .stdout(contains("Unwrap may panic"))
        .stdout(contains("Cost: $0.0000"))
        .stdout(predicates::str::is_match("Stub result").unwrap().not());
}

#[test]
fn agent_unknown_subagent_exits_nonzero() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .args(["agent", "unknown", "query"])
        .assert()
        .failure()
        .stderr(contains("unknown_subagent"));
}
