//! Fresh review packet builder.
//!
//! Builds a review packet that contains diff + evidence only;
//! no editor transcript leaks into the review context.

use serde::{Deserialize, Serialize};

use crate::diff::FileDiff;
use mimir_edit::test_runner::TestRunResult;

/// A review packet: diff + evidence, no editor transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewPacket {
    /// Commit or run being reviewed.
    pub target: String,
    /// Files changed.
    pub diffs: Vec<FileDiffSummary>,
    /// Test evidence.
    pub test_evidence: Option<TestRunResult>,
    /// Check findings.
    pub check_findings: Vec<crate::Finding>,
    /// Override requests pending.
    pub pending_overrides: Vec<String>,
}

/// Summarized file diff for review context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffSummary {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    pub is_generated: bool,
    pub summary: String,
}

impl ReviewPacket {
    /// Build a review packet from raw diff and test data.
    pub fn build(
        target: impl Into<String>,
        diffs: &[FileDiff],
        test_evidence: Option<&TestRunResult>,
        check_findings: &[crate::Finding],
    ) -> Self {
        Self {
            target: target.into(),
            diffs: diffs.iter().map(|d| FileDiffSummary {
                path: d.path.clone(),
                additions: d.additions,
                deletions: d.deletions,
                is_generated: d.is_generated,
                summary: format!("{}+ {}-", d.additions, d.deletions),
            }).collect(),
            test_evidence: test_evidence.cloned(),
            check_findings: check_findings.to_vec(),
            pending_overrides: Vec::new(),
        }
    }

    /// Estimate token count for the packet (heuristic).
    pub fn estimated_tokens(&self) -> u32 {
        let mut chars = 0;
        for diff in &self.diffs {
            chars += diff.path.len() + diff.summary.len() + 50;
        }
        if let Some(ref test) = self.test_evidence {
            chars += test.stdout.len().min(2000) + test.stderr.len().min(500);
        }
        for finding in &self.check_findings {
            chars += finding.description.len() + finding.category.len() + 30;
        }
        (chars / 4) as u32
    }

    /// Check if the packet exceeds a token cap.
    pub fn exceeds_cap(&self, cap: u32) -> bool {
        self.estimated_tokens() > cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::FileDiff;

    #[test]
    fn test_review_packet_build() {
        let diffs = vec![FileDiff {
            path: "src/main.rs".into(),
            is_new: false,
            is_deleted: false,
            is_generated: false,
            additions: 5,
            deletions: 2,
            hunks: vec![],
        }];
        let packet = ReviewPacket::build("abc123", &diffs, None, &[]);
        assert_eq!(packet.diffs.len(), 1);
        assert_eq!(packet.diffs[0].additions, 5);
        assert!(packet.estimated_tokens() > 0);
    }

    #[test]
    fn test_review_packet_cap() {
        let diffs = vec![FileDiff {
            path: "src/main.rs".into(),
            is_new: false,
            is_deleted: false,
            is_generated: false,
            additions: 5,
            deletions: 2,
            hunks: vec![],
        }];
        let packet = ReviewPacket::build("abc123", &diffs, None, &[]);
        assert!(!packet.exceeds_cap(10000));
    }
}
