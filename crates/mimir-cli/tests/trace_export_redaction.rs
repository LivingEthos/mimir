//! Trace-export hardening (Codex slice 3b).
//!
//! `trace export --redact` must scrub local filesystem paths (not just secret
//! values), and `--output` must refuse to write through a symlink.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

fn mimir(dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("mimir").unwrap();
    cmd.current_dir(dir.path());
    cmd
}

fn seed_run(dir: &TempDir, run_id: &str, events: &str) {
    let run = dir.path().join(".mimir/runs").join(run_id);
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("events.jsonl"), events).unwrap();
}

#[test]
fn redact_scrubs_absolute_workspace_paths_from_export() {
    let dir = TempDir::new().unwrap();
    // The CLI computes its workspace root via canonicalize("."); match it here.
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let absolute = canonical.join("private_notes").join("inner.txt");
    let absolute = absolute.to_string_lossy().to_string();

    let run_id = "20260601-130000-cccccccc";
    let event = format!(
        r#"{{"event_type":"patch_applied","timestamp":"2026-06-01T13:00:00Z","path":"{}"}}"#,
        absolute.replace('\\', "\\\\")
    );
    seed_run(&dir, run_id, &format!("{event}\n"));

    let output = mimir(&dir)
        .args(["trace", "export", run_id, "--redact"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();

    // The absolute workspace prefix must be gone; the relative tail must remain.
    assert!(
        !stdout.contains(canonical.to_string_lossy().as_ref()),
        "absolute workspace path leaked into export: {stdout}"
    );
    assert!(
        stdout.contains("private_notes/inner.txt"),
        "expected workspace-relative path in export: {stdout}"
    );
}

#[cfg(unix)]
#[test]
fn export_refuses_symlinked_output_target() {
    let dir = TempDir::new().unwrap();
    let run_id = "20260601-130500-dddddddd";
    seed_run(
        &dir,
        run_id,
        "{\"event_type\":\"patch_applied\",\"timestamp\":\"2026-06-01T13:05:00Z\"}\n",
    );

    // A sentinel file the symlinked --output target points at.
    let sentinel = dir.path().join("sentinel.txt");
    std::fs::write(&sentinel, "ORIGINAL").unwrap();
    std::os::unix::fs::symlink(&sentinel, dir.path().join("out.json")).unwrap();

    mimir(&dir)
        .args(["trace", "export", run_id, "--output", "out.json"])
        .assert()
        .failure()
        .stderr(contains("symlink"));

    // The symlink target must be untouched — nothing written through the link.
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "ORIGINAL");
}
