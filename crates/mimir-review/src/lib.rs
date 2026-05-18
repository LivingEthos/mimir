//! `mimir-review` — Review, override, and audit infrastructure.
//!
//! Phase 4 deliverables:
//! - Diff review (`mimir review --since <ref>`)
//! - Fresh review packet builder (no editor transcript leaks)
//! - Uninspected-diff detector
//! - Generated-file edit detector
//! - Test-evidence check
//! - OverrideRequest flow with auto-grant after N failures
//! - Read-only override mode
//! - Override audit log
//! - `.mimir/checks/*.md` source-controlled checks
//! - `.mimir/commands/*.md` repo-defined recipes
//! - Committee reviewer (deterministic deduplicated findings)

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod audit;
pub mod checks;
pub mod committee;
pub mod diff;
pub mod override_req;
pub mod packet;

/// Errors from the review system.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum ReviewError {
    #[error("git_error: {0}")]
    GitError(String),
    #[error("check_failed: {name} — {reason}")]
    CheckFailed { name: String, reason: String },
    #[error("override_denied: {request_id}")]
    OverrideDenied { request_id: String },
    #[error("io_error: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, ReviewError>;

/// A review finding (single issue).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Finding {
    /// Finding category.
    pub category: String,
    /// File path(s) involved.
    pub paths: Vec<String>,
    /// Human-readable description.
    pub description: String,
    /// Severity: info, warn, error, critical.
    pub severity: String,
    /// Line numbers if applicable.
    pub line_numbers: Option<Vec<u32>>,
}

/// A collection of findings for a review run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReviewReport {
    /// Run or commit being reviewed.
    pub target: String,
    /// All findings.
    pub findings: Vec<Finding>,
    /// Whether the review passed (no error/critical findings).
    pub passed: bool,
    /// Number of findings by severity.
    pub summary: HashMap<String, u32>,
}

impl ReviewReport {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            findings: Vec::new(),
            passed: true,
            summary: HashMap::new(),
        }
    }

    pub fn add(&mut self, finding: Finding) {
        let sev = finding.severity.clone();
        *self.summary.entry(sev).or_insert(0) += 1;
        if finding.severity == "error" || finding.severity == "critical" {
            self.passed = false;
        }
        self.findings.push(finding);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_review_report_passed() {
        let mut report = ReviewReport::new("abc123");
        report.add(Finding {
            category: "style".into(),
            paths: vec!["src/main.rs".into()],
            description: "trailing whitespace".into(),
            severity: "info".into(),
            line_numbers: None,
        });
        assert!(report.passed);
        report.add(Finding {
            category: "bug".into(),
            paths: vec!["src/lib.rs".into()],
            description: "null pointer".into(),
            severity: "error".into(),
            line_numbers: Some(vec![42]),
        });
        assert!(!report.passed);
        assert_eq!(report.summary.get("info"), Some(&1));
        assert_eq!(report.summary.get("error"), Some(&1));
    }
}
