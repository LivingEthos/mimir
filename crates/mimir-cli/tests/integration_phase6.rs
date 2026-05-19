//! Integration tests for Phase 6: Memory, Server, TUI.

use std::process::Command;

#[test]
fn test_cli_memory_subcommand_help() {
    let output = Command::new("cargo")
        .args(["run", "--bin", "mimir", "--", "memory", "--help"])
        .current_dir(".")
        .output()
        .expect("failed to execute");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("list") || combined.contains("show") || combined.contains("search"),
        "memory subcommands should be listed: {}",
        combined
    );
}

#[test]
fn test_cli_version() {
    let output = Command::new("cargo")
        .args(["run", "--bin", "mimir", "--", "version"])
        .current_dir(".")
        .output()
        .expect("failed to execute");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("mimir"), "version should print mimir: {}", stdout);
}

#[test]
fn test_cli_serve_help() {
    let output = Command::new("cargo")
        .args(["run", "--bin", "mimir", "--", "serve", "--help"])
        .current_dir(".")
        .output()
        .expect("failed to execute");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("port") || combined.contains("stdio"),
        "serve options should be listed: {}",
        combined
    );
}
