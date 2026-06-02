//! Override request/grant audit coverage (DOD: "override request logs grants").
//!
//! Exercises the `mimir override request` command end-to-end: every request
//! writes a request artifact plus a redacted `override_requested` audit event,
//! and a satisfied auto-grant threshold additionally writes an `OverrideGrant`
//! artifact plus a redacted `override_granted` audit event. No provider calls.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

/// AWS example access-key id — matched by the built-in `AKIA…` redactor pattern.
const SECRET_LIKE: &str = "AKIAIOSFODNN7EXAMPLE";

fn mimir(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path());
    cmd
}

fn run_id_from_stdout(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("Run ID: "))
        .expect("Run ID line in stdout")
        .trim()
        .to_string()
}

fn run_dir(dir: &TempDir, run_id: &str) -> std::path::PathBuf {
    dir.path().join(".mimir/runs").join(run_id)
}

#[test]
fn request_writes_request_artifact_and_audit_event_when_pending() {
    let dir = TempDir::new().unwrap();
    let output = mimir(&dir)
        .args([
            "override",
            "request",
            "--cap",
            "128000",
            "--reason",
            "Need more room to review the migration scripts",
        ])
        .assert()
        .success()
        .stdout(contains("Status: pending approval"))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let run = run_dir(&dir, &run_id_from_stdout(&stdout));

    // The request artifact is written.
    assert!(run.join("override_request.json").exists());
    // A structured request audit event is appended.
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    assert!(events.contains("override_requested"));
    assert!(events.contains("\"requested_cap\":128000"));
    assert!(events.contains("\"requested_by\":\"cli\""));
    // No grant is recorded while the request is pending.
    assert!(!run.join("override_grant.json").exists());
    assert!(!events.contains("override_granted"));
}

#[test]
fn request_auto_grants_immediately_with_zero_threshold() {
    let dir = TempDir::new().unwrap();
    let output = mimir(&dir)
        .args([
            "override",
            "request",
            "--cap",
            "256000",
            "--reason",
            "Burst review across the whole module",
            "--auto-grant-after",
            "0",
        ])
        .assert()
        .success()
        .stdout(contains("Status: granted (auto_after_failures)"))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let run = run_dir(&dir, &run_id_from_stdout(&stdout));

    // Both the request and the grant artifacts exist.
    assert!(run.join("override_request.json").exists());
    let grant: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run.join("override_grant.json")).unwrap())
            .unwrap();
    assert_eq!(grant["schema_version"], 1);
    assert_eq!(grant["granted_cap"], 256000);
    assert_eq!(grant["granted_by"], "auto_after_failures");
    assert_eq!(grant["prior_failures"], 0);
    assert_eq!(grant["auto_grant_after"], 0);

    // The grant is validated against its JSON schema contract.
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../schemas/OverrideGrant.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(
        validator.is_valid(&grant),
        "override_grant.json must validate against OverrideGrant.schema.json"
    );

    // Both audit events are present.
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    assert!(events.contains("override_requested"));
    assert!(events.contains("override_granted"));
}

#[test]
fn request_auto_grants_after_counted_failures() {
    let dir = TempDir::new().unwrap();
    // Seed an existing run with three real failure events plus a non-failure event.
    let run_id = "20260601-120000-aaaaaaaa";
    let run = run_dir(&dir, run_id);
    std::fs::create_dir_all(&run).unwrap();
    let events = [
        r#"{"event_type":"provider_response","timestamp":"2026-06-01T12:00:00Z"}"#,
        r#"{"event_type":"patch_rejected","timestamp":"2026-06-01T12:00:01Z"}"#,
        r#"{"event_type":"patch_tests_failed","timestamp":"2026-06-01T12:00:02Z"}"#,
        r#"{"event_type":"cost_cap_aborted","timestamp":"2026-06-01T12:00:03Z"}"#,
    ]
    .join("\n");
    std::fs::write(run.join("events.jsonl"), format!("{events}\n")).unwrap();

    mimir(&dir)
        .args([
            "override",
            "request",
            "--cap",
            "128000",
            "--reason",
            "Three attempts already failed under the default cap",
            "--auto-grant-after",
            "3",
            "--run-id",
            run_id,
        ])
        .assert()
        .success()
        .stdout(contains("Prior failed attempts: 3"))
        .stdout(contains("Status: granted (auto_after_failures)"));

    let grant: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run.join("override_grant.json")).unwrap())
            .unwrap();
    assert_eq!(grant["prior_failures"], 3);
    assert_eq!(grant["auto_grant_after"], 3);

    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    assert!(events.contains("override_granted"));
}

#[test]
fn request_stays_pending_when_failures_below_threshold() {
    let dir = TempDir::new().unwrap();
    // Two failures plus a non-failure event — below the threshold of three.
    let run_id = "20260601-120500-bbbbbbbb";
    let run = run_dir(&dir, run_id);
    std::fs::create_dir_all(&run).unwrap();
    let events = [
        r#"{"event_type":"patch_rejected","timestamp":"2026-06-01T12:05:01Z"}"#,
        r#"{"event_type":"provider_response","timestamp":"2026-06-01T12:05:02Z"}"#,
        r#"{"event_type":"cost_cap_aborted","timestamp":"2026-06-01T12:05:03Z"}"#,
    ]
    .join("\n");
    std::fs::write(run.join("events.jsonl"), format!("{events}\n")).unwrap();

    mimir(&dir)
        .args([
            "override",
            "request",
            "--cap",
            "128000",
            "--reason",
            "Only two attempts have failed so far",
            "--auto-grant-after",
            "3",
            "--run-id",
            run_id,
        ])
        .assert()
        .success()
        .stdout(contains("Prior failed attempts: 2"))
        .stdout(contains("Status: pending approval"));

    assert!(!run.join("override_grant.json").exists());
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();
    assert!(!events.contains("override_granted"));
}

#[test]
fn request_redacts_secret_like_reason_in_all_artifacts_and_events() {
    let dir = TempDir::new().unwrap();
    let reason = format!("debugging with leaked key {SECRET_LIKE} attached");
    let output = mimir(&dir)
        .args([
            "override",
            "request",
            "--cap",
            "128000",
            "--reason",
            &reason,
            "--auto-grant-after",
            "0",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    let run = run_dir(&dir, &run_id_from_stdout(&stdout));

    let request = std::fs::read_to_string(run.join("override_request.json")).unwrap();
    let grant = std::fs::read_to_string(run.join("override_grant.json")).unwrap();
    let events = std::fs::read_to_string(run.join("events.jsonl")).unwrap();

    for (label, body) in [
        ("override_request.json", &request),
        ("override_grant.json", &grant),
        ("events.jsonl", &events),
    ] {
        assert!(
            !body.contains(SECRET_LIKE),
            "secret leaked into {label}: {body}"
        );
        assert!(
            body.contains("<REDACTED:"),
            "{label} should carry a redaction marker"
        );
    }
}
