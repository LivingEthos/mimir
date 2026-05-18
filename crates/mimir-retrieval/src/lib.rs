//! `mimir-retrieval` — ripgrep, AST search, ranking, candidate manifest emission.

#![warn(missing_docs)]

use mimir_index::RepoIndex;
use serde::{Deserialize, Serialize};

/// A candidate produced by retrieval before packing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCandidate {
    /// File path.
    pub path: String,
    /// Line ranges.
    pub ranges: Vec<Range>,
    /// Kind of candidate.
    pub candidate_kind: String,
    /// Relevance score.
    pub score: f64,
    /// Feature vector.
    pub features: serde_json::Value,
    /// Estimated tokens.
    pub estimated_tokens: u32,
    /// How this candidate was discovered.
    pub discovered_by: String,
    /// Primary reason code.
    pub reason_code: String,
}

/// A line range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Range {
    /// Start line (1-indexed).
    pub start: u32,
    /// End line (1-indexed).
    pub end: u32,
}

/// Run the retrieval pipeline against a repo index.
pub fn run_pipeline(index: &RepoIndex, task: &str) -> Vec<ContextCandidate> {
    // Phase 0 stub: return empty candidates.
    let _ = (index, task);
    Vec::new()
}
