//! Generated types from JSON Schema (Phase 0: hand-written stubs).
//!
//! In a later phase, this module will be auto-generated from `schemas/*.schema.json`
//! via `typify`. For now, each type is a minimal struct matching the schema shape
//! so that downstream crates can compile.

#![allow(missing_docs)]

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
    pub task_card: TaskCard,
    pub mode: String,
    pub cap_tokens: u32,
    pub target_tokens: u32,
    pub output_reserve_tokens: u32,
    pub count_drift_reserve_tokens: u32,
    pub provider: String,
    pub model: String,
    pub capability_snapshot_ref: String,
    pub prompt_contract_version: u32,
    pub included: Vec<IncludedItem>,
    pub omitted_candidates: Vec<OmittedCandidate>,
    pub tool_schemas: Vec<ToolSchema>,
    pub evidence_cards: Vec<EvidenceCard>,
    pub memory_entries: Vec<ContextMemoryEntryRef>,
    pub budget_ledger_ref: String,
    pub estimated_input_tokens: u32,
    pub count_provenance: String,
    pub created_at: String,
    pub authoritative_input_tokens: Option<u32>,
    /// High-risk omissions flagged by the recall guard (Stage 7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recall_guard_flags: Vec<RecallGuardFlag>,
}

/// A compact task card carried by every context packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCard {
    pub goal: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acceptance_criteria: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub likely_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_test_command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknowns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub need_for_large_context: Option<String>,
    pub complexity: String,
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
    pub path: String,
    pub ranges: Vec<ContextRange>,
    pub candidate_kind: String,
    pub reason_code: String,
    pub tokens: u32,
    pub source_hash: String,
    pub trust_level: String,
    pub editable: bool,
}

/// An omitted candidate with reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmittedCandidate {
    pub schema_version: u32,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<ContextRange>,
    pub candidate_kind: String,
    pub reason_code: String,
    pub score: f64,
    pub features: serde_json::Value,
    pub estimated_tokens: u32,
    pub discovered_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    pub reason_for_omission: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
    pub what_would_trigger_inclusion: String,
}

/// A 1-indexed inclusive source range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRange {
    pub start: u32,
    pub end: u32,
}

/// A tool schema entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred: Option<bool>,
}

/// Evidence card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceCard {
    pub kind: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u32>,
}

/// Memory entry reference included in a context packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMemoryEntryRef {
    pub entry_id: String,
    pub tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<String>,
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
    pub packet_id: String,
    pub files_to_edit: Vec<PatchFileEdit>,
    pub editable_target_set: Vec<String>,
    pub reasoning_per_edit: Vec<PatchEditReasoning>,
    pub tests_to_run: Vec<String>,
    pub risks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub what_needs_more_context: Vec<String>,
}

/// A file the patch plan expects to edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchFileEdit {
    pub path: String,
    pub edit_kind: PatchEditKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ranges: Vec<PatchRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_new_content_hash: Option<String>,
}

/// The kind of planned edit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatchEditKind {
    AstReplace,
    LineRangeReplace,
    UnifiedDiff,
    WholeFileRewrite,
}

/// A line range referenced by a planned edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchRange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u32>,
}

/// Reasoning for a planned edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchEditReasoning {
    pub path: String,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disproving_evidence: Option<String>,
}

/// Persisted implementation-plan artifact written by `mimir plan`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanArtifact {
    pub schema_version: u32,
    pub run_id: String,
    pub packet_id: String,
    pub packet_hash: String,
    pub provider: String,
    pub model: String,
    pub task: String,
    pub editable_target_set: Vec<String>,
    pub steps: Vec<String>,
    pub risks: Vec<String>,
    pub files_likely_affected: Vec<String>,
    pub tests_to_run: Vec<String>,
    pub assumptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_response: Option<String>,
}

/// Executable patch recipe used by Mimir's safe patch engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutablePatchPlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub packet_id: String,
    #[serde(deserialize_with = "deserialize_non_empty_patch_steps")]
    pub steps: Vec<PatchStep>,
}

fn deserialize_non_empty_patch_steps<'de, D>(deserializer: D) -> Result<Vec<PatchStep>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let steps = Vec::<PatchStep>::deserialize(deserializer)?;
    if steps.is_empty() {
        return Err(serde::de::Error::custom(
            "steps must contain at least one patch step",
        ));
    }
    Ok(steps)
}

/// A single patch step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
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

#[cfg(test)]
mod patch_plan_tests {
    use super::{ExecutablePatchPlan, PatchEditKind, PatchPlan, PatchStep, PlanArtifact};

    fn assert_example_validates(schema_json: &str, example_json: &str) {
        let schema: serde_json::Value = serde_json::from_str(schema_json).unwrap();
        let example: serde_json::Value = serde_json::from_str(example_json).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors = validator
            .iter_errors(&example)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();

        assert!(errors.is_empty(), "schema validation failed: {errors:#?}");
    }

    fn assert_example_rejected(schema_json: &str, example: serde_json::Value) {
        let schema: serde_json::Value = serde_json::from_str(schema_json).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();

        assert!(
            !validator.is_valid(&example),
            "schema unexpectedly accepted {example}"
        );
    }

    #[test]
    fn deserializes_patch_plan_example() {
        assert_example_validates(
            include_str!("../../../schemas/PatchPlan.schema.json"),
            include_str!("../../../examples/patch-plan.example.json"),
        );

        let plan: PatchPlan =
            serde_json::from_str(include_str!("../../../examples/patch-plan.example.json"))
                .unwrap();

        assert_eq!(plan.schema_version, 1);
        assert_eq!(plan.files_to_edit[0].edit_kind, PatchEditKind::AstReplace);
        assert_eq!(plan.editable_target_set[0], "src/auth/session.ts");
    }

    #[test]
    fn deserializes_executable_patch_plan_example() {
        assert_example_validates(
            include_str!("../../../schemas/ExecutablePatchPlan.schema.json"),
            include_str!("../../../examples/executable-patch-plan.example.json"),
        );

        let plan: ExecutablePatchPlan = serde_json::from_str(include_str!(
            "../../../examples/executable-patch-plan.example.json"
        ))
        .unwrap();

        assert_eq!(plan.schema_version, 1);
        assert!(matches!(
            plan.steps[0],
            PatchStep::UnifiedDiff { ref path, .. } if path == "src/auth/session.ts"
        ));
    }

    #[test]
    fn deserializes_plan_artifact_example() {
        assert_example_validates(
            include_str!("../../../schemas/PlanArtifact.schema.json"),
            include_str!("../../../examples/plan-artifact.example.json"),
        );

        let artifact: PlanArtifact =
            serde_json::from_str(include_str!("../../../examples/plan-artifact.example.json"))
                .unwrap();
        assert_eq!(artifact.schema_version, 1);
        assert_eq!(artifact.task, "Plan the session refresh fix");
    }

    #[test]
    fn executable_patch_plan_schema_rejects_incomplete_payloads() {
        let schema = include_str!("../../../schemas/ExecutablePatchPlan.schema.json");
        assert_example_rejected(
            schema,
            serde_json::json!({
                "schema_version": 1,
                "plan_id": "plan-example",
                "steps": [{
                    "action": "delete",
                    "path": "src/auth/session.ts"
                }]
            }),
        );
        assert_example_rejected(
            schema,
            serde_json::json!({
                "schema_version": 1,
                "plan_id": "plan-example",
                "packet_id": "packet-example",
                "steps": []
            }),
        );
        assert_example_rejected(
            schema,
            serde_json::json!({
                "schema_version": 1,
                "plan_id": "plan-example",
                "packet_id": "packet-example",
                "steps": [{
                    "action": "delete",
                    "path": "src/auth/session.ts",
                    "extra": true
                }]
            }),
        );
    }

    #[test]
    fn rust_executable_patch_plan_deserializer_is_strict() {
        let missing_packet = serde_json::json!({
            "schema_version": 1,
            "plan_id": "plan-example",
            "steps": [{
                "action": "delete",
                "path": "src/auth/session.ts"
            }]
        });
        assert!(serde_json::from_value::<ExecutablePatchPlan>(missing_packet).is_err());

        let empty_steps = serde_json::json!({
            "schema_version": 1,
            "plan_id": "plan-example",
            "packet_id": "packet-example",
            "steps": []
        });
        assert!(serde_json::from_value::<ExecutablePatchPlan>(empty_steps).is_err());

        let extra_step_field = serde_json::json!({
            "schema_version": 1,
            "plan_id": "plan-example",
            "packet_id": "packet-example",
            "steps": [{
                "action": "delete",
                "path": "src/auth/session.ts",
                "extra": true
            }]
        });
        assert!(serde_json::from_value::<ExecutablePatchPlan>(extra_step_field).is_err());
    }
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
