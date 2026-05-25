use assert_cmd::Command;

#[test]
fn eval_context_dataset_outputs_schema_valid_results() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let dataset = repo_root.join("fixtures/context-recall-v1.yaml");
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../schemas/EvalResult.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(&repo_root)
        .args([
            "eval",
            "context",
            "--dataset",
            dataset.to_str().unwrap(),
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
    assert_eq!(results.len(), 15);
    for result in results {
        assert!(validator.is_valid(result), "invalid EvalResult: {result:#}");
        assert_eq!(result["metrics"]["cap_compliance"], true);
    }
}
