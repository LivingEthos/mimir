//! `mimir-security` — Safety classification, secret redaction, and permission gating.

#![warn(missing_docs)]

use regex::Regex;
use std::sync::OnceLock;

/// Safety classification for commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyClass {
    /// Read-only operations.
    Read,
    /// Local verification (e.g., tests, lints).
    LocalVerify,
    /// Mutating operations (e.g., git commit, file write).
    Mutate,
    /// Dangerous operations (e.g., force push, rm -rf).
    Dangerous,
}

/// Classify a shell command by its safety level.
pub fn classify_command(cmd: &str) -> SafetyClass {
    let lower = cmd.to_lowercase();
    if lower.contains("rm -rf")
        || lower.contains("force")
        || lower.contains("drop")
        || lower.contains("truncate")
    {
        return SafetyClass::Dangerous;
    }
    if lower.contains("git commit")
        || lower.contains("git push")
        || lower.contains("git merge")
        || lower.contains("write")
        || lower.contains("create")
    {
        return SafetyClass::Mutate;
    }
    if lower.starts_with("cargo test")
        || lower.starts_with("cargo clippy")
        || lower.starts_with("cargo check")
        || lower.starts_with("cargo fmt")
        || lower.starts_with("test ")
        || lower.starts_with("lint ")
        || lower.starts_with("check ")
        || lower.starts_with("fmt ")
    {
        return SafetyClass::LocalVerify;
    }
    SafetyClass::Read
}

/// Redact known secret patterns from text.
pub mod redactor;

pub use redactor::redact_secrets;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_read() {
        assert_eq!(classify_command("cat file.txt"), SafetyClass::Read);
        assert_eq!(classify_command("ls -la"), SafetyClass::Read);
    }

    #[test]
    fn test_classify_local_verify() {
        assert_eq!(classify_command("cargo test"), SafetyClass::LocalVerify);
        assert_eq!(classify_command("cargo clippy"), SafetyClass::LocalVerify);
        assert_eq!(classify_command("cargo check"), SafetyClass::LocalVerify);
        assert_eq!(classify_command("cargo fmt"), SafetyClass::LocalVerify);
    }

    #[test]
    fn test_classify_mutate() {
        assert_eq!(classify_command("git commit -m 'msg'"), SafetyClass::Mutate);
        assert_eq!(classify_command("git push origin main"), SafetyClass::Mutate);
    }

    #[test]
    fn test_classify_dangerous() {
        assert_eq!(classify_command("rm -rf /"), SafetyClass::Dangerous);
        assert_eq!(classify_command("git push --force"), SafetyClass::Dangerous);
    }

    #[test]
    fn test_redact_aws_key() {
        let text = "key=AKIAIOSFODNN7EXAMPLE";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(redacted.contains("<REDACTED:AWS_KEY>"));
    }

    #[test]
    fn test_redact_openai_key() {
        let text = "sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKL";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKL"));
        assert!(redacted.contains("<REDACTED:OPENAI_KEY>"));
    }

    #[test]
    fn test_no_false_redaction() {
        let text = "hello world";
        assert_eq!(redact_secrets(text), text);
    }
}
