use assert_cmd::Command;

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
