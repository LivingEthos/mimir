//! `mimir-tools` — Tool runner with safety classification and result cards.

#![warn(missing_docs)]

use mimir_security::{classify_command, SafetyClass};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::process::Command;

/// Result of running a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultCard {
    /// Schema version.
    pub schema_version: u32,
    /// Tool name.
    pub tool_name: String,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Exit code.
    pub exit_code: i32,
}

/// Run a shell command with safety classification.
///
/// # Errors
///
/// Returns an error if the command fails to spawn or if the safety class
/// is `Dangerous` and `allow_dangerous` is `false`.
pub async fn run_command(
    name: &str,
    cmd: &str,
    allow_dangerous: bool,
) -> Result<ToolResultCard, String> {
    let safety = classify_command(cmd);
    if safety == SafetyClass::Dangerous && !allow_dangerous {
        return Err("policy_denied: dangerous command requires explicit allow".to_string());
    }

    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("io_error: {}", e))?;

    Ok(ToolResultCard {
        schema_version: 1,
        tool_name: name.to_string(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_echo() {
        let result = run_command("echo", "echo hello", false).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn run_dangerous_denied() {
        let result = run_command("rm", "rm -rf /tmp/test", false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("policy_denied"));
    }

    #[tokio::test]
    async fn run_dangerous_allowed() {
        let result = run_command("echo", "echo hello", true).await.unwrap();
        assert_eq!(result.exit_code, 0);
    }
}
