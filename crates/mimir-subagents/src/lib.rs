//! `mimir-subagents` — Subagent orchestration, tool catalog, cost-tier routing.
//!
//! Phase 5 deliverables:
//! - Deterministic file-analyst (no LLM)
//! - Deterministic provider-free subagent execution with read-only local evidence
//! - EvidenceSummary schema
//! - Subagent packet lineage
//! - Tool schema compiler with deferred catalog
//! - Tool-schema token budget line
//! - .mimir/skills/ path-gated skills
//! - Task router
//! - Cost-tier mapping

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod analyst;
pub mod catalog;
pub mod cost;
pub mod lineage;
pub mod router;
pub mod skills;
pub mod subagents;

/// Errors from the subagent system.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum SubagentError {
    #[error("gateway_bypass: subagent attempted to bypass gateway")]
    GatewayBypass,
    #[error("cap_exceeded: {subagent} exceeded {cap} tokens")]
    CapExceeded { subagent: String, cap: u32 },
    #[error("unknown_subagent: {0}")]
    UnknownSubagent(String),
    #[error("tool_not_found: {0}")]
    ToolNotFound(String),
    #[error("io_error: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, SubagentError>;

/// EvidenceSummary: structured return value from any subagent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EvidenceSummary {
    /// Subagent that produced this evidence.
    pub subagent: String,
    /// What was asked.
    pub query: String,
    /// Key findings (bullet points).
    pub findings: Vec<String>,
    /// Relevant file paths.
    pub relevant_paths: Vec<String>,
    /// Confidence (0.0-1.0).
    pub confidence: f64,
    /// Tokens consumed.
    pub tokens_consumed: u32,
    /// Cost in dollars.
    pub cost_usd: f64,
    /// Parent run_id for lineage.
    pub parent_run_id: Option<String>,
    /// Subagent run_id.
    pub run_id: String,
}

/// A subagent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentDef {
    /// Unique name.
    pub name: String,
    /// Description of what this subagent does.
    pub description: String,
    /// Whether this subagent uses an LLM.
    pub uses_llm: bool,
    /// Cost tier: free, cheap, standard, expensive.
    pub cost_tier: CostTier,
    /// Token cap for this subagent.
    pub token_cap: u32,
    /// Allowed tools (empty = all).
    pub allowed_tools: Vec<String>,
    /// Read-only (cannot mutate files).
    pub read_only: bool,
}

/// Cost tier for subagent routing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CostTier {
    /// Deterministic, no LLM (free).
    Free,
    /// Cheap LLM (e.g. Haiku).
    Cheap,
    /// Standard LLM (e.g. Sonnet).
    Standard,
    /// Expensive LLM (e.g. Opus).
    Expensive,
}

impl std::fmt::Display for CostTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CostTier::Free => write!(f, "free"),
            CostTier::Cheap => write!(f, "cheap"),
            CostTier::Standard => write!(f, "standard"),
            CostTier::Expensive => write!(f, "expensive"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evidence_summary_default() {
        let es = EvidenceSummary {
            subagent: "file-analyst".into(),
            query: "Find all panics".into(),
            findings: vec!["3 panics found".into()],
            relevant_paths: vec!["src/lib.rs".into()],
            confidence: 0.95,
            tokens_consumed: 0,
            cost_usd: 0.0,
            parent_run_id: Some("run-1".into()),
            run_id: "run-1a".into(),
        };
        assert_eq!(es.subagent, "file-analyst");
        assert_eq!(es.confidence, 0.95);
    }

    #[test]
    fn test_cost_tier_display() {
        assert_eq!(CostTier::Free.to_string(), "free");
        assert_eq!(CostTier::Cheap.to_string(), "cheap");
    }
}
