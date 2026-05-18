//! Generated types from JSON Schema (Phase 0: hand-written stubs).
//!
//! In a later phase, this module will be auto-generated from `schemas/*.schema.json`
//! via `typify`. For now, each type is a minimal struct matching the schema shape
//! so that downstream crates can compile.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ContextPacket
// ---------------------------------------------------------------------------

/// A hashable, replayable context packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPacket {
    pub schema_version: u32,
    pub packet_id: String,
    pub packet_hash: String,
    pub run_id: String,
    pub task_card: String,
    pub mode: String,
    pub cap_tokens: u32,
    pub target_tokens: u32,
    pub output_reserve_tokens: u32,
    pub count_drift_reserve_tokens: u32,
    pub provider: String,
    pub model: String,
    pub capability_snapshot_ref: String,
    pub prompt_contract_version: String,
    pub included: Vec<IncludedItem>,
    pub omitted_candidates: Vec<OmittedCandidate>,
    pub tool_schemas: Vec<ToolSchema>,
    pub evidence_cards: Vec<EvidenceCard>,
    pub memory_entries: Vec<MemoryEntry>,
    pub budget_ledger_ref: Option<String>,
    pub estimated_input_tokens: u32,
    pub count_provenance: String,
    pub created_at: String,
    pub authoritative_input_tokens: Option<u32>,
    /// High-risk omissions flagged by the recall guard (Stage 7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recall_guard_flags: Vec<RecallGuardFlag>,
}

/// A recall guard flag indicating a high-risk omission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallGuardFlag {
    /// Risk category.
    pub risk: String,
    /// Path of the omitted or at-risk file.
    pub path: String,
    /// Human-readable reason for the flag.
    pub reason: String,
    /// Suggested remediation action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// An included context item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncludedItem {
    pub kind: String,
    pub source_path: String,
    pub content: String,
    pub token_count: u32,
}

/// An omitted candidate with reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmittedCandidate {
    pub source_path: String,
    pub reason: String,
    pub token_count: u32,
}

/// A tool schema entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub schema: serde_json::Value,
}

/// Evidence card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceCard {
    pub kind: String,
    pub content: String,
}

// ---------------------------------------------------------------------------
// MemoryEntry
// ---------------------------------------------------------------------------

/// A durable lesson with source, confidence, scope, and freshness metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub schema_version: u32,
    pub entry_id: String,
    pub kind: String,
    pub body: String,
    pub source_evidence: Vec<SourceEvidence>,
    pub confidence: String,
    pub promotion_score: Option<f64>,
    pub promotion_breakdown: Option<PromotionBreakdown>,
    pub scope: String,
    pub safe_to_send: bool,
    pub created_at: String,
    pub last_verified_at: Option<String>,
    pub staleness_policy: Option<StalenessPolicy>,
    pub retrieval_tags: Vec<String>,
    pub imported_from: Option<ImportedFrom>,
}

/// Source evidence for a memory entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceEvidence {
    pub kind: String,
    pub ref_: String,
}

/// Promotion score breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionBreakdown {
    pub severity_score: f64,
    pub recurrence_score: f64,
    pub success_rate_score: f64,
    pub task_relevance_score: f64,
    pub token_savings_score: f64,
    pub total: f64,
}

/// Staleness policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StalenessPolicy {
    pub max_age_days: Option<u32>,
    pub auto_revoke_after_failures: Option<u32>,
}

/// Imported-from metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedFrom {
    pub tool: String,
    pub session_id: String,
}

// ---------------------------------------------------------------------------
// EvalCase & EvalResult
// ---------------------------------------------------------------------------

/// A single eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub case_id: String,
    pub description: String,
    pub input: serde_json::Value,
    pub expected: serde_json::Value,
}

/// Result of running an eval case.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub case_id: String,
    pub passed: bool,
    pub score: f64,
    pub notes: String,
}

// ---------------------------------------------------------------------------
// BudgetLedger
// ---------------------------------------------------------------------------

/// Budget ledger tracking token usage by category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLedger {
    pub schema_version: u32,
    pub run_id: String,
    pub categories: Vec<BudgetCategory>,
    pub total_tokens: u32,
}

/// A single budget category.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetCategory {
    pub name: String,
    pub tokens: u32,
    pub percentage: f64,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Structured error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// ProviderCapabilities
// ---------------------------------------------------------------------------

/// Provider capabilities snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub schema_version: u32,
    pub provider: String,
    pub models: serde_json::Value,
}

// ---------------------------------------------------------------------------
// PatchPlan
// ---------------------------------------------------------------------------

/// A patch plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub steps: Vec<PatchStep>,
}

/// A single patch step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum PatchStep {
    /// Replace a line range with new content.
    LineRange {
        /// Target file path.
        path: String,
        /// 1-based start line (inclusive).
        start_line: usize,
        /// 1-based end line (inclusive).
        end_line: usize,
        /// Replacement content.
        content: String,
    },
    /// Apply a unified diff.
    UnifiedDiff {
        /// Target file path.
        path: String,
        /// Diff text in unified diff format.
        diff: String,
    },
    /// Write entire file content.
    WholeFile {
        /// Target file path.
        path: String,
        /// Full file content.
        content: String,
    },
    /// Create a new file.
    Create {
        /// Target file path.
        path: String,
        /// File content.
        content: String,
    },
    /// Delete a file.
    Delete {
        /// Target file path.
        path: String,
    },
    /// Move/rename a file.
    Move {
        /// Source path.
        from: String,
        /// Destination path.
        to: String,
    },
}

// ---------------------------------------------------------------------------
// AuditEvent
// ---------------------------------------------------------------------------

/// An audit event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub timestamp: String,
    pub event_type: String,
    pub details: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// TraceSpan
// ---------------------------------------------------------------------------

/// A trace span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    pub span_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub start: String,
    pub end: Option<String>,
    pub attributes: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// DriftReport
// ---------------------------------------------------------------------------

/// Token count drift report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub schema_version: u32,
    pub run_id: String,
    pub observed_drift_percent: f64,
    pub p95_drift_percent: f64,
}

// ---------------------------------------------------------------------------
// CandidateManifest
// ---------------------------------------------------------------------------

/// Candidate manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateManifest {
    pub schema_version: u32,
    pub run_id: String,
    pub candidates: Vec<ContextCandidate>,
}

/// Context candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCandidate {
    pub source_path: String,
    pub token_count: u32,
    pub relevance_score: f64,
}

// ---------------------------------------------------------------------------
// ContextPlan
// ---------------------------------------------------------------------------

/// Context plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPlan {
    pub schema_version: u32,
    pub run_id: String,
    pub plan_items: Vec<PlanItem>,
}

/// Plan item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanItem {
    pub kind: String,
    pub source_path: String,
    pub token_budget: u32,
}

// ---------------------------------------------------------------------------
// OverrideRequest
// ---------------------------------------------------------------------------

/// Override request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrideRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub run_id: String,
    pub reason: String,
    pub requested_by: String,
}

// ---------------------------------------------------------------------------
// ReviewResult
// ---------------------------------------------------------------------------

/// Review result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResult {
    pub schema_version: u32,
    pub review_id: String,
    pub passed: bool,
    pub findings: Vec<String>,
}

// ---------------------------------------------------------------------------
// RetryPolicy
// ---------------------------------------------------------------------------

/// Retry policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub schema_version: u32,
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter: f64,
}

// ---------------------------------------------------------------------------
// TestCard
// ---------------------------------------------------------------------------

/// Test card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCard {
    pub schema_version: u32,
    pub card_id: String,
    pub description: String,
    pub command: String,
    pub expected_exit_code: i32,
}

// ---------------------------------------------------------------------------
// ToolResultCard
// ---------------------------------------------------------------------------

/// Tool result card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultCard {
    pub schema_version: u32,
    pub tool_name: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

// ---------------------------------------------------------------------------
// EvidenceSummary
// ---------------------------------------------------------------------------

/// Evidence summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub schema_version: u32,
    pub summary_id: String,
    pub content: String,
}
