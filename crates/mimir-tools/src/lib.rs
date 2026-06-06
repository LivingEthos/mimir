//! `mimir-tools` — Tool runner with safety classification and result cards.

#![warn(missing_docs)]

use mimir_runs::RunDir;
use mimir_security::{classify_command, SafetyClass};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// Re-export the schema type so consumers use a single source of truth.
pub use mimir_schemas::ToolResultCard;

/// Configuration for running a command and producing a [`ToolResultCard`].
pub struct RunCommandConfig<'a> {
    /// Stable card identifier.
    pub card_id: &'a str,
    /// Human-readable tool name.
    pub name: &'a str,
    /// Shell command to execute.
    pub cmd: &'a str,
    /// Working directory for the command.
    pub cwd: &'a str,
    /// Timeout in milliseconds.
    pub timeout_ms: u32,
    /// Whether to allow `Dangerous` safety-class commands.
    pub allow_dangerous: bool,
    /// Optional run directory for spilling full stdout/stderr artifacts.
    pub run_dir: Option<&'a RunDir>,
}

/// Preview cap for stdout/stderr (characters).
const PREVIEW_CAP: usize = 2_000;
/// Threshold above which output is considered "large" and gets
/// `summary_only` inclusion policy.
const LARGE_OUTPUT_TOKENS: u32 = 8_192;

/// Run a shell command with safety classification and produce a
/// schema-compliant [`ToolResultCard`].
///
/// # Errors
///
/// Returns an error if the command fails to spawn, if the safety class
/// is `Dangerous` and `allow_dangerous` is `false`, or if the command
/// times out.
pub async fn run_command(config: &RunCommandConfig<'_>) -> Result<ToolResultCard, String> {
    let safety = classify_command(config.cmd);
    if safety == SafetyClass::Dangerous && !config.allow_dangerous {
        return Err("policy_denied: dangerous command requires explicit allow".to_string());
    }

    let start = Instant::now();
    let child = Command::new("sh")
        .arg("-c")
        .arg(config.cmd)
        .current_dir(config.cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    let output = timeout(Duration::from_millis(u64::from(config.timeout_ms)), child)
        .await
        .map_err(|_| "timeout: command exceeded timeout".to_string())?
        .map_err(|e| format!("io_error: {}", e))?;

    let duration_ms = saturating_millis(start.elapsed().as_millis());

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let stdout_preview = cap_preview(&stdout, PREVIEW_CAP);
    let stderr_preview = cap_preview(&stderr, PREVIEW_CAP);

    let stdout_original_size_bytes = u32::try_from(output.stdout.len()).unwrap_or(u32::MAX);
    let stderr_original_size_bytes = u32::try_from(output.stderr.len()).unwrap_or(u32::MAX);

    let (stdout_artifact_path, stderr_artifact_path) = if let Some(run_dir) = config.run_dir {
        let s_path = write_artifact(run_dir, &format!("{}-stdout.log", config.card_id), &stdout)
            .map_err(|e| format!("artifact_error: {}", e))?;
        let e_path = write_artifact(run_dir, &format!("{}-stderr.log", config.card_id), &stderr)
            .map_err(|e| format!("artifact_error: {}", e))?;
        (Some(s_path), Some(e_path))
    } else {
        (None, None)
    };

    let estimated_tokens = estimate_tokens(&stdout) + estimate_tokens(&stderr);
    let inclusion_policy = if estimated_tokens > LARGE_OUTPUT_TOKENS {
        "summary_only"
    } else {
        "preview_only"
    };

    let detected_file_refs = extract_file_refs(&stdout);
    let detected_test_refs = extract_test_refs(&stdout);

    Ok(ToolResultCard {
        schema_version: 1,
        card_id: config.card_id.to_string(),
        command: config.cmd.to_string(),
        cwd: config.cwd.to_string(),
        safety_class: safety_class_string(safety),
        timeout_ms: config.timeout_ms,
        exit_code: output.status.code().unwrap_or(-1),
        duration_ms,
        stdout_preview,
        stderr_preview,
        stdout_artifact_path,
        stderr_artifact_path,
        stdout_original_size_bytes: Some(stdout_original_size_bytes),
        stderr_original_size_bytes: Some(stderr_original_size_bytes),
        detected_file_refs,
        detected_test_refs,
        estimated_tokens,
        inclusion_policy: inclusion_policy.to_string(),
        filters_applied: Vec::new(),
    })
}

fn saturating_millis(val: u128) -> u32 {
    u32::try_from(val).unwrap_or(u32::MAX)
}

fn cap_preview(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        text.to_string()
    } else {
        // Truncate on a UTF-8 char boundary: a raw `&text[..cap]` slice panics
        // when `cap` splits a multi-byte character (common in tool output).
        let end = floor_char_boundary(text, cap);
        format!("{}…(+{} chars)", &text[..end], text.len() - end)
    }
}

/// Largest byte index `<= max_bytes` that lands on a UTF-8 char boundary.
fn floor_char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn write_artifact(run_dir: &RunDir, name: &str, contents: &str) -> std::io::Result<String> {
    let path = run_dir.artifact_path(name)?;
    mimir_runs::atomic_write(&path, contents.as_bytes())?;
    Ok(path.as_str().to_string())
}

fn estimate_tokens(text: &str) -> u32 {
    // Conservative heuristic: ~4 chars per token.
    (text.len() / 4).try_into().unwrap_or(u32::MAX)
}

fn safety_class_string(safety: SafetyClass) -> String {
    match safety {
        SafetyClass::Read => "READ".to_string(),
        SafetyClass::LocalVerify => "LOCAL_VERIFY".to_string(),
        SafetyClass::Mutate => "MUTATE".to_string(),
        SafetyClass::Dangerous => "DANGEROUS".to_string(),
    }
}

fn extract_file_refs(stdout: &str) -> Vec<String> {
    let mut refs = std::collections::BTreeSet::new();
    // Simple regex-free extraction: look for path-like tokens.
    for word in stdout.split_whitespace() {
        let cleaned = word.trim_matches(|c| {
            c == '`' || c == '\'' || c == '"' || c == '(' || c == ')' || c == ',' || c == ':'
        });
        if cleaned.contains('/') && !cleaned.starts_with("http") {
            // Take first reasonable length token
            if cleaned.len() > 2 && cleaned.len() < 256 {
                refs.insert(cleaned.to_string());
            }
        }
    }
    refs.into_iter().collect()
}

fn extract_test_refs(stdout: &str) -> Vec<String> {
    let mut refs = std::collections::BTreeSet::new();
    for line in stdout.lines() {
        // Look for "test " prefix
        if let Some(pos) = line.find("test ") {
            let after = &line[pos + 5..];
            let name = after.split_whitespace().next().unwrap_or("");
            if !name.is_empty() && name.len() < 128 {
                refs.insert(name.to_string());
            }
        }
        // Look for tokens like "foo::test_bar" or "foo::test_bar_baz"
        for word in line.split_whitespace() {
            let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':');
            if (cleaned.contains("::test_") || cleaned.starts_with("test_")) && cleaned.len() < 128
            {
                refs.insert(cleaned.to_string());
            }
        }
    }
    refs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_echo() {
        let config = RunCommandConfig {
            card_id: "echo-1",
            name: "echo",
            cmd: "echo hello",
            cwd: ".",
            timeout_ms: 5_000,
            allow_dangerous: false,
            run_dir: None,
        };
        let result = run_command(&config).await.unwrap();
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout_preview.contains("hello"));
        assert_eq!(result.schema_version, 1);
        assert_eq!(result.safety_class, "READ");
    }

    #[tokio::test]
    async fn run_dangerous_denied() {
        let config = RunCommandConfig {
            card_id: "rm-1",
            name: "rm",
            cmd: "rm -rf /tmp/test",
            cwd: ".",
            timeout_ms: 5_000,
            allow_dangerous: false,
            run_dir: None,
        };
        let result = run_command(&config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("policy_denied"));
    }

    #[tokio::test]
    async fn run_dangerous_allowed() {
        let config = RunCommandConfig {
            card_id: "echo-2",
            name: "echo",
            cmd: "echo hello",
            cwd: ".",
            timeout_ms: 5_000,
            allow_dangerous: true,
            run_dir: None,
        };
        let result = run_command(&config).await.unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn cap_preview_works() {
        assert_eq!(cap_preview("hello", 10), "hello");
        let long = "a".repeat(3000);
        let capped = cap_preview(&long, 2000);
        assert!(capped.contains("…(+1000 chars)"));
    }

    #[test]
    fn cap_preview_does_not_panic_on_multibyte_boundary() {
        // Place a 2-byte char straddling the cap; a naive byte slice panics.
        let mut text = "a".repeat(1999);
        text.push('é'); // bytes 1999..2001, cap=2000 falls inside it
        text.push_str(&"b".repeat(100));
        let capped = cap_preview(&text, 2000);
        assert!(capped.contains("…(+"));
    }

    #[test]
    fn extract_file_refs_finds_paths() {
        let text = "error in src/auth/session.ts at line 42\nsee also tests/auth/index.ts";
        let refs = extract_file_refs(text);
        assert!(refs.contains(&"src/auth/session.ts".to_string()));
        assert!(refs.contains(&"tests/auth/index.ts".to_string()));
    }

    #[test]
    fn extract_test_refs_finds_tests() {
        let text = "test validateSession returns ok\nmodule::test_foo_bar failed";
        let refs = extract_test_refs(text);
        assert!(refs.contains(&"validateSession".to_string()));
        assert!(refs.contains(&"module::test_foo_bar".to_string()));
    }
}
