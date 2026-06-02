//! Journey tests for the provider-free `init` and `doctor` commands.
//!
//! These exercise the full CLI binary end to end in a fresh tempdir:
//! `mimir init` seeds the workflow scaffold non-interactively, and
//! `mimir doctor` reports green for every probe it actually runs against an
//! initialized project. No provider or network calls are involved — both
//! commands are provider-free by construction.

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;
use tempfile::TempDir;

/// Files that `mimir init` is contractually required to seed.
const SEEDED_FILES: &[&str] = &[
    ".mimir/config.yaml",
    ".mimir/project-rules.md",
    ".mimir/checks/no-provider-secrets.md",
    ".mimir/commands/fast-check.md",
    ".mimir/commands/release-check.md",
];

fn read_non_empty(dir: &TempDir, rel: &str) -> String {
    let path = dir.path().join(rel);
    assert!(path.is_file(), "expected seeded file to exist: {rel}");
    let body = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("seeded file {rel} should be readable utf-8: {error}");
    });
    assert!(
        !body.trim().is_empty(),
        "seeded file {rel} must be non-empty"
    );
    body
}

/// INIT gate: `mimir init` runs non-interactively with closed stdin, exits 0
/// without prompting, and seeds every workflow file with non-empty content.
/// The seeded `config.yaml` must parse as YAML.
#[test]
fn init_runs_non_interactively_and_seeds_non_empty_files() {
    let dir = TempDir::new().unwrap();

    let output = Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        // Closed stdin: any interactive prompt would hang or fail here.
        .write_stdin("")
        .arg("init")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "init must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Created workflow files"),
        "init should report created files, got: {stdout}"
    );
    // No interactive prompt text should ever be emitted.
    let combined = format!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
    let prompt_markers = ["? ", "(y/n)", "[Y/n]", "[y/N]", "Press enter", "Continue?"];
    for marker in prompt_markers {
        assert!(
            !combined.contains(marker),
            "init must not prompt; found marker {marker:?} in output: {combined}"
        );
    }

    for rel in SEEDED_FILES {
        let _ = read_non_empty(&dir, rel);
    }

    // config.yaml must be valid YAML.
    let config_body = read_non_empty(&dir, ".mimir/config.yaml");
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&config_body).expect("seeded .mimir/config.yaml must parse as YAML");
    assert!(
        parsed.is_mapping(),
        "config.yaml should deserialize into a YAML mapping, got: {parsed:?}"
    );
}

/// INIT gate: a second `mimir init` is idempotent and must not overwrite an
/// existing file the user has edited.
#[test]
fn second_init_does_not_overwrite_edited_file() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .write_stdin("")
        .arg("init")
        .assert()
        .success();

    // The user hand-edits a seeded file.
    let edited_rel = ".mimir/project-rules.md";
    let edited_path = dir.path().join(edited_rel);
    let sentinel = "# EDITED BY USER\nDo not clobber this content.\n";
    std::fs::write(&edited_path, sentinel).unwrap();

    // Second init in the already-initialized directory.
    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .write_stdin("")
        .arg("init")
        .assert()
        .success();

    let after = std::fs::read_to_string(&edited_path).unwrap();
    assert_eq!(
        after, sentinel,
        "second init must preserve the user-edited file verbatim"
    );

    // The other seeded files must still exist and stay non-empty.
    for rel in SEEDED_FILES {
        let _ = read_non_empty(&dir, rel);
    }
}

/// DOCTOR gate: after `mimir init`, `mimir doctor` exits 0 and reports OK/green
/// for every probe it actually runs (config, provider capabilities, token
/// counter, context packet, permissions) plus the optional provider-credentials
/// probe and an overall ok status. There is no `--json` flag for doctor, so we
/// assert on the real human-readable probe labels.
#[test]
fn doctor_reports_green_for_initialized_project() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .write_stdin("")
        .arg("init")
        .assert()
        .success();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        // Keep the credentials probe deterministically "optional".
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .arg("doctor")
        .assert()
        .success()
        .stdout(
            contains("Config: ok")
                .and(contains("Provider capabilities: ok"))
                .and(contains("Token counter: ok"))
                .and(contains("Context packet: ok"))
                .and(contains("Permissions: ok"))
                .and(contains("Provider credentials: optional"))
                .and(contains("Doctor status: ok"))
                // No probe should report a failure on a clean init.
                .and(contains(": fail").not()),
        );
}

/// DOCTOR gate: when a provider credential is present, the credentials probe
/// flips from optional to ok while every other probe stays green. The value is
/// synthetic — doctor never makes a provider call, so no secret leaves the box.
#[test]
fn doctor_detects_provider_credentials_without_network() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .write_stdin("")
        .arg("init")
        .assert()
        .success();

    Command::cargo_bin("mimir")
        .unwrap()
        .current_dir(dir.path())
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ZAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        // Synthetic, never transmitted: doctor only reads the env var name.
        .env("GLM_API_KEY", "synthetic-doctor-probe-key")
        .arg("doctor")
        .assert()
        .success()
        .stdout(
            contains("Config: ok")
                .and(contains("Provider credentials: ok"))
                .and(contains("glm"))
                .and(contains("Doctor status: ok")),
        );
}
