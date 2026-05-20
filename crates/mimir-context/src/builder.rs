//! Context packet builder.

use std::path::PathBuf;

use mimir_runs::RunId;
use mimir_schemas::{
    ContextPacket, ContextRange, IncludedItem, OmittedCandidate, RecallGuardFlag, TaskCard,
};
use sha2::{Digest, Sha256};

/// Builder for ContextPacket.
#[derive(Debug, Default)]
pub struct ContextBuilder {
    run_id: Option<RunId>,
    task_card: Option<String>,
    mode: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    repo_root: Option<PathBuf>,
    edit_targets: Vec<String>,
}

struct RetrievedContext {
    included: Vec<IncludedItem>,
    omitted_candidates: Vec<OmittedCandidate>,
    recall_guard_flags: Vec<RecallGuardFlag>,
    included_tokens: u32,
}

impl ContextBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the run ID assigned by the caller.
    pub fn run_id(mut self, run_id: RunId) -> Self {
        self.run_id = Some(run_id);
        self
    }

    /// Set task card.
    pub fn task_card(mut self, card: impl Into<String>) -> Self {
        self.task_card = Some(card.into());
        self
    }

    /// Set mode.
    pub fn mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = Some(mode.into());
        self
    }

    /// Set provider.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Set model.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set repository root for retrieval-backed packet construction.
    pub fn repo_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.repo_root = Some(root.into());
        self
    }

    /// Set explicit edit targets for retrieval sufficiency checks.
    pub fn edit_targets(mut self, targets: Vec<String>) -> Self {
        self.edit_targets = targets;
        self
    }

    /// Build the packet.
    pub fn build(self) -> Result<ContextPacket, anyhow::Error> {
        let run_id = self.run_id.unwrap_or_else(RunId::generate);
        let task_goal = self.task_card.unwrap_or_default();
        if contains_secret_like_text(&task_goal) {
            anyhow::bail!("secret_risk: task text contains secret-like content");
        }
        let mode = normalize_mode(&self.mode.unwrap_or_else(|| "ask".to_string()));
        let provider = self.provider.unwrap_or_else(|| "anthropic".to_string());
        let model = self
            .model
            .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
        let resolved_capabilities =
            mimir_providers::capabilities::resolve_provider_capabilities(&provider, &model)
                .map_err(anyhow::Error::msg)?;
        let model_capabilities = resolved_capabilities
            .capabilities
            .models
            .get(&model)
            .ok_or_else(|| {
                anyhow::anyhow!("provider capabilities missing resolved model {provider}/{model}")
            })?;
        let output_reserve_tokens = model_capabilities.output_reserve_tokens;
        let count_drift_reserve_tokens =
            mimir_providers::capabilities::DEFAULT_COUNT_DRIFT_RESERVE_TOKENS;

        let retrieved = build_retrieved_context(self.repo_root, &task_goal, &self.edit_targets)?;
        let task_tokens = mimir_providers::count::count_local(&task_goal);
        let estimated_input_tokens = task_tokens
            .saturating_add(retrieved.included_tokens)
            .saturating_add(8);
        let task_card = TaskCard {
            goal: task_goal,
            acceptance_criteria: Vec::new(),
            likely_files: self.edit_targets.clone(),
            risk_level: None,
            expected_test_command: None,
            unknowns: Vec::new(),
            need_for_large_context: None,
            complexity: if mode == "code" || mode == "plan" {
                "standard".to_string()
            } else {
                "tiny".to_string()
            },
        };

        let capability_snapshot_ref = resolved_capabilities.snapshot_ref;

        let mut packet = ContextPacket {
            schema_version: 1,
            packet_id: format!("pkt-{}", run_id),
            packet_hash: String::new(),
            run_id: run_id.to_string(),
            task_card,
            mode,
            cap_tokens: 64000,
            target_tokens: 32000,
            output_reserve_tokens,
            count_drift_reserve_tokens,
            provider,
            model,
            capability_snapshot_ref,
            prompt_contract_version: 1,
            included: retrieved.included,
            omitted_candidates: retrieved.omitted_candidates,
            tool_schemas: vec![],
            evidence_cards: vec![],
            memory_entries: vec![],
            budget_ledger_ref: format!(".mimir/runs/{run_id}/budget_ledger.json"),
            estimated_input_tokens,
            count_provenance: "local_estimate_only".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            authoritative_input_tokens: None,
            recall_guard_flags: retrieved.recall_guard_flags,
        };
        packet.packet_hash = crate::hash::hash_packet(&packet);
        Ok(packet)
    }
}

fn normalize_mode(mode: &str) -> String {
    match mode {
        "ask"
        | "plan"
        | "code"
        | "review"
        | "explain"
        | "subagent_search"
        | "subagent_file_analyst"
        | "subagent_reviewer"
        | "subagent_test_summarizer"
        | "committee_specialist" => mode.to_string(),
        "standard" => "ask".to_string(),
        _ => "ask".to_string(),
    }
}

fn build_retrieved_context(
    repo_root: Option<PathBuf>,
    task_card: &str,
    edit_targets: &[String],
) -> Result<RetrievedContext, anyhow::Error> {
    let Some(root) = repo_root else {
        return Ok(RetrievedContext {
            included: Vec::new(),
            omitted_candidates: Vec::new(),
            recall_guard_flags: Vec::new(),
            included_tokens: 0,
        });
    };

    let index = mimir_index::build_index(&root)?;
    let config = mimir_retrieval::PipelineConfig::default();
    let pipeline = mimir_retrieval::run_pipeline(&index, task_card, edit_targets, &config);

    let mut total_tokens = 0u32;
    let mut included = Vec::new();
    let mut omitted_candidates = Vec::new();
    for item in pipeline.manifest.included {
        let path = root.join(&item.path);
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let token_count = mimir_providers::count::count_local(&content);
        let source_hash = sha256_hex(content.as_bytes());
        if contains_secret_like_text(&content) {
            omitted_candidates.push(OmittedCandidate {
                schema_version: 1,
                path: item.path.clone(),
                ranges: Vec::new(),
                candidate_kind: "full_file".to_string(),
                reason_code: normalize_reason_code(&item.reason_code),
                score: 0.0,
                features: serde_json::json!({ "secret_risk": 1.0 }),
                estimated_tokens: token_count,
                discovered_by: vec!["manifest".to_string()],
                source_hash: Some(source_hash),
                reason_for_omission: "secret_risk".to_string(),
                risk: None,
                what_would_trigger_inclusion:
                    "Remove secret-like material before including this file in provider context."
                        .to_string(),
            });
            continue;
        }
        total_tokens = total_tokens.saturating_add(token_count);
        included.push(IncludedItem {
            path: item.path.clone(),
            ranges: item
                .ranges
                .into_iter()
                .map(|range| ContextRange {
                    start: range.start,
                    end: range.end,
                })
                .collect(),
            candidate_kind: "full_file".to_string(),
            reason_code: normalize_reason_code(&item.reason_code),
            tokens: token_count,
            source_hash,
            trust_level: "trusted".to_string(),
            editable: edit_targets.iter().any(|target| target == &item.path),
        });
    }

    omitted_candidates.extend(
        pipeline
            .manifest
            .omitted
            .into_iter()
            .map(|item| OmittedCandidate {
                schema_version: 1,
                path: item.path,
                ranges: Vec::new(),
                candidate_kind: "full_file".to_string(),
                reason_code: "embedding_match".to_string(),
                score: 0.0,
                features: serde_json::json!({}),
                estimated_tokens: item.estimated_tokens,
                discovered_by: vec!["manifest".to_string()],
                source_hash: None,
                reason_for_omission: normalize_omission_reason(&item.reason),
                risk: item.risk.as_deref().and_then(normalize_omission_risk),
                what_would_trigger_inclusion: omission_trigger(&item.reason),
            }),
    );

    let recall_guard_flags = pipeline
        .recall_guard_flags
        .into_iter()
        .map(|flag| RecallGuardFlag {
            risk: normalize_recall_risk(&flag.category),
            path: flag.paths.first().cloned().unwrap_or_default(),
            reason: flag.description,
            suggestion: None,
        })
        .collect();

    Ok(RetrievedContext {
        included,
        omitted_candidates,
        recall_guard_flags,
        included_tokens: total_tokens,
    })
}

fn contains_secret_like_text(text: &str) -> bool {
    mimir_security::redact_secrets(text) != text
}

fn normalize_reason_code(reason: &str) -> String {
    match reason {
        "direct_task_match" | "direct_path_match" | "direct" => "direct_user_mention",
        "symbol_reference_match" | "symbol" => "symbol_definition",
        "import_graph_expansion" => "caller",
        "test" => "failing_test_reference",
        "config" => "config_dependency",
        "schema" => "route_or_schema_link",
        "route" => "route_or_schema_link",
        "git" => "git_cochange",
        "memory" => "prior_memory_match",
        "manifest" | "mandatory" => "manifest_reference",
        _ => "embedding_match",
    }
    .to_string()
}

fn normalize_omission_reason(reason: &str) -> String {
    match reason {
        "budget_exceeded_mandatory" | "budget_overflow" => "budget_overflow",
        "generated_file_policy" => "generated_file_policy",
        "secret_risk" => "secret_risk",
        "large_file_threshold" => "large_file_threshold",
        "untrusted_repo_policy" => "untrusted_repo_policy",
        "redundant_with_included" => "redundant_with_included",
        _ => "lower_relevance_score",
    }
    .to_string()
}

fn normalize_omission_risk(risk: &str) -> Option<String> {
    Some(
        match risk {
            "caller_missing" => "caller_missing",
            "test_missing" | "omitted_test_for_target" | "failing_test_dropped" => "test_missing",
            "config_missing" => "config_missing",
            "schema_missing" => "schema_missing",
            "import_orphan" | "import_of_omitted" => "import_orphan",
            "mandatory_omitted" => "caller_missing",
            _ => return None,
        }
        .to_string(),
    )
}

fn normalize_recall_risk(risk: &str) -> String {
    normalize_omission_risk(risk).unwrap_or_else(|| "caller_missing".to_string())
}

fn omission_trigger(reason: &str) -> String {
    match normalize_omission_reason(reason).as_str() {
        "budget_overflow" => "Increase the context budget or reduce higher-priority context.",
        "generated_file_policy" => "Allow generated files for this run.",
        "secret_risk" => "Remove or redact secret-like content from the candidate.",
        "large_file_threshold" => "Request this file explicitly or narrow it to a smaller range.",
        "untrusted_repo_policy" => "Mark the source as trusted for provider context.",
        "redundant_with_included" => "Remove the overlapping included candidate.",
        _ => "Increase relevance through a direct task mention, failing test, import, or manifest reference.",
    }
    .to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_preserves_external_run_id_and_hashes_packet() {
        let run_id = RunId("20260101-120000-abcdef01".to_string());
        let packet = ContextBuilder::new()
            .run_id(run_id.clone())
            .task_card("Explain ContextBuilder")
            .provider("glm")
            .model("glm-5.1")
            .build()
            .unwrap();

        assert_eq!(packet.run_id, run_id.to_string());
        assert_eq!(packet.packet_id, format!("pkt-{}", run_id));
        assert_eq!(packet.provider, "glm");
        assert_eq!(packet.model, "glm-5.1");
        assert_eq!(packet.packet_hash.len(), 64);
        assert!(packet.estimated_input_tokens > 0);
        assert_eq!(packet.task_card.goal, "Explain ContextBuilder");
        assert_eq!(packet.count_provenance, "local_estimate_only");
    }

    #[test]
    fn build_can_include_retrieved_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("ContextBuilder.rs"),
            "pub struct ContextBuilder;\n",
        )
        .unwrap();

        let packet = ContextBuilder::new()
            .run_id(RunId("20260101-120000-abcdef02".to_string()))
            .task_card("ContextBuilder")
            .repo_root(dir.path())
            .build()
            .unwrap();

        assert_eq!(packet.included.len(), 1);
        assert_eq!(packet.included[0].path, "ContextBuilder.rs");
        assert!(packet.included[0].tokens > 0);
        assert_eq!(packet.included[0].source_hash.len(), 64);
    }

    #[test]
    fn anthropic_packet_uses_registry_reserve_and_snapshot_ref() {
        let packet = ContextBuilder::new()
            .run_id(RunId("20260101-120000-abcdef03".to_string()))
            .task_card("Explain registry-backed capabilities")
            .provider("anthropic")
            .model("claude-sonnet-4-20250514")
            .build()
            .unwrap();

        assert_eq!(packet.output_reserve_tokens, 64_000);
        assert!(packet
            .capability_snapshot_ref
            .starts_with("providers/anthropic.yaml@sha256:"));
    }

    #[test]
    fn dynamic_provider_packet_uses_generated_snapshot_hash() {
        let packet = ContextBuilder::new()
            .run_id(RunId("20260101-120000-abcdef04".to_string()))
            .task_card("Explain dynamic capabilities")
            .provider("glm")
            .model("glm-5.1")
            .build()
            .unwrap();

        assert!(packet
            .capability_snapshot_ref
            .starts_with("generated:glm/glm-5.1@sha256:"));
    }
}
