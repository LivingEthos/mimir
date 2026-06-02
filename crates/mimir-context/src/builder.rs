//! Context packet builder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    repo_index: Option<Arc<mimir_index::RepoIndex>>,
    edit_targets: Vec<String>,
}

struct RetrievedContext {
    included: Vec<IncludedItem>,
    omitted_candidates: Vec<OmittedCandidate>,
    recall_guard_flags: Vec<RecallGuardFlag>,
    included_tokens: u32,
}

const ALWAYS_GUIDANCE_FILES: &[&str] = &[".mimir/project-rules.md", "AGENTS.md", "CLAUDE.md"];
const TASK_RELEVANT_GUIDANCE_FILES: &[&str] = &["README.md", "docs/HANDOFF.md"];
const MAX_GUIDANCE_FILE_TOKENS: u32 = 4_096;
const MAX_GUIDANCE_TOTAL_TOKENS: u32 = 8_192;

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

    /// Reuse a prebuilt repository index for retrieval-backed packet construction.
    pub fn repo_index(mut self, index: impl Into<Arc<mimir_index::RepoIndex>>) -> Self {
        self.repo_index = Some(index.into());
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

        let retrieved = build_retrieved_context(
            self.repo_root,
            self.repo_index.as_deref(),
            &task_goal,
            &self.edit_targets,
        )?;
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
    repo_index: Option<&mimir_index::RepoIndex>,
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

    let owned_index;
    let index = if let Some(index) = repo_index {
        index
    } else {
        owned_index = mimir_index::build_index(&root)?;
        &owned_index
    };
    let config = mimir_retrieval::PipelineConfig::default();
    let pipeline = mimir_retrieval::run_pipeline(index, task_card, edit_targets, &config);

    let mut total_tokens = 0u32;
    let mut included = Vec::new();
    let mut omitted_candidates = Vec::new();
    include_repository_guidance(
        &root,
        task_card,
        edit_targets,
        &mut included,
        &mut omitted_candidates,
        &mut total_tokens,
    );
    let mut included_paths = included
        .iter()
        .map(|item| item.path.clone())
        .collect::<BTreeSet<_>>();
    for item in pipeline.manifest.included {
        if included_paths.contains(&item.path) {
            continue;
        }
        if !mimir_index::is_indexable_path(Path::new(&item.path)) {
            omitted_candidates.push(policy_omission(
                &item.path,
                item.estimated_tokens,
                "generated_file_policy",
                "Use a source or documentation file instead of generated or packaged artifacts.",
            ));
            continue;
        }
        let path = root.join(&item.path);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !mimir_index::is_indexable_file(Path::new(&item.path), metadata.len()) {
            omitted_candidates.push(policy_omission(
                &item.path,
                item.estimated_tokens,
                "large_file_threshold",
                "Request this file explicitly with a narrower range or reduce its size.",
            ));
            continue;
        }
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
        included_paths.insert(item.path.clone());
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

fn include_repository_guidance(
    root: &Path,
    task_card: &str,
    edit_targets: &[String],
    included: &mut Vec<IncludedItem>,
    omitted_candidates: &mut Vec<OmittedCandidate>,
    total_tokens: &mut u32,
) {
    let mut guidance_tokens = 0u32;
    let mut candidates = ALWAYS_GUIDANCE_FILES.to_vec();
    if should_include_task_relevant_guidance(task_card) {
        candidates.extend_from_slice(TASK_RELEVANT_GUIDANCE_FILES);
    }

    for relative_path in candidates {
        let path = root.join(relative_path);
        if !path.is_file() {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let token_count = mimir_providers::count::count_local(&content);
        let source_hash = sha256_hex(content.as_bytes());
        if contains_secret_like_text(&content) {
            omitted_candidates.push(guidance_omission(
                relative_path,
                token_count,
                Some(source_hash),
                "secret_risk",
                "Remove secret-like material before including this repository guidance file.",
            ));
            continue;
        }
        if token_count > MAX_GUIDANCE_FILE_TOKENS
            || guidance_tokens.saturating_add(token_count) > MAX_GUIDANCE_TOTAL_TOKENS
        {
            omitted_candidates.push(guidance_omission(
                relative_path,
                token_count,
                Some(source_hash),
                "large_file_threshold",
                "Shorten this guidance file or split durable rules into .mimir/project-rules.md.",
            ));
            continue;
        }
        guidance_tokens = guidance_tokens.saturating_add(token_count);
        *total_tokens = total_tokens.saturating_add(token_count);
        included.push(IncludedItem {
            path: relative_path.to_string(),
            ranges: vec![ContextRange {
                start: 1,
                end: u32::MAX,
            }],
            candidate_kind: "full_file".to_string(),
            reason_code: "manifest_reference".to_string(),
            tokens: token_count,
            source_hash,
            trust_level: "trusted".to_string(),
            editable: edit_targets.iter().any(|target| target == relative_path),
        });
    }
}

fn should_include_task_relevant_guidance(task_card: &str) -> bool {
    let task = task_card.to_ascii_lowercase();
    [
        "agent",
        "contributor",
        "development",
        "documentation",
        "handoff",
        "onboarding",
        "readme",
        "workflow",
    ]
    .iter()
    .any(|needle| task.contains(needle))
}

fn guidance_omission(
    path: &str,
    token_count: u32,
    source_hash: Option<String>,
    reason_for_omission: &str,
    trigger: &str,
) -> OmittedCandidate {
    OmittedCandidate {
        schema_version: 1,
        path: path.to_string(),
        ranges: Vec::new(),
        candidate_kind: "full_file".to_string(),
        reason_code: "manifest_reference".to_string(),
        score: 0.0,
        features: serde_json::json!({ "repository_guidance": 1.0 }),
        estimated_tokens: token_count,
        discovered_by: vec!["repository_guidance".to_string()],
        source_hash,
        reason_for_omission: reason_for_omission.to_string(),
        risk: None,
        what_would_trigger_inclusion: trigger.to_string(),
    }
}

fn policy_omission(
    path: &str,
    token_count: u32,
    reason_for_omission: &str,
    trigger: &str,
) -> OmittedCandidate {
    OmittedCandidate {
        schema_version: 1,
        path: path.to_string(),
        ranges: Vec::new(),
        candidate_kind: "full_file".to_string(),
        reason_code: "embedding_match".to_string(),
        score: 0.0,
        features: serde_json::json!({ "index_policy": 1.0 }),
        estimated_tokens: token_count,
        discovered_by: vec!["manifest".to_string()],
        source_hash: None,
        reason_for_omission: reason_for_omission.to_string(),
        risk: None,
        what_would_trigger_inclusion: trigger.to_string(),
    }
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
    fn build_uses_supplied_repo_index() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("indexed.rs"),
            "pub struct IndexedContext;\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("unindexed.rs"), "pub struct GhostMatch;\n").unwrap();
        std::fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        std::fs::write(
            dir.path().join("target/debug/generated.rs"),
            "pub struct PackagedArtifact;\n",
        )
        .unwrap();
        let oversized_unit = "pub struct OversizedArtifact;\n";
        let oversized_source = oversized_unit
            .repeat((mimir_index::MAX_INDEXED_FILE_BYTES as usize / oversized_unit.len()) + 1);
        std::fs::write(dir.path().join("huge.rs"), oversized_source).unwrap();

        let mut index = mimir_index::RepoIndex::new();
        index.add(mimir_index::FileEntry {
            path: "indexed.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "indexed".to_string(),
            token_count: 1,
            exports: vec!["IndexedContext".to_string()],
            imports: Vec::new(),
        });
        index.add(mimir_index::FileEntry {
            path: "target/debug/generated.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "generated".to_string(),
            token_count: 1,
            exports: vec!["PackagedArtifact".to_string()],
            imports: Vec::new(),
        });
        index.add(mimir_index::FileEntry {
            path: "huge.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "huge".to_string(),
            token_count: 1,
            exports: vec!["OversizedArtifact".to_string()],
            imports: Vec::new(),
        });

        let packet = ContextBuilder::new()
            .run_id(RunId("20260101-120000-abcdef07".to_string()))
            .task_card("GhostMatch IndexedContext PackagedArtifact OversizedArtifact")
            .repo_root(dir.path())
            .repo_index(Arc::new(index))
            .provider("glm")
            .model("glm-5.1")
            .build()
            .unwrap();

        assert!(packet.included.iter().any(|item| item.path == "indexed.rs"));
        assert!(!packet
            .included
            .iter()
            .any(|item| item.path == "unindexed.rs"));
        assert!(!packet
            .included
            .iter()
            .any(|item| item.path == "target/debug/generated.rs"));
        assert!(!packet.included.iter().any(|item| item.path == "huge.rs"));
        assert!(packet.omitted_candidates.iter().any(|item| {
            item.path == "target/debug/generated.rs"
                && item.reason_for_omission == "generated_file_policy"
        }));
        assert!(packet.omitted_candidates.iter().any(|item| {
            item.path == "huge.rs" && item.reason_for_omission == "large_file_threshold"
        }));
    }

    #[test]
    fn build_includes_repository_guidance_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "# Agent rules\nRead first.\n").unwrap();
        std::fs::write(dir.path().join("feature.rs"), "pub fn feature() {}\n").unwrap();

        let packet = ContextBuilder::new()
            .run_id(RunId("20260101-120000-abcdef05".to_string()))
            .task_card("feature")
            .repo_root(dir.path())
            .provider("glm")
            .model("glm-5.1")
            .build()
            .unwrap();

        let guidance = packet
            .included
            .iter()
            .find(|item| item.path == "AGENTS.md")
            .expect("AGENTS.md should be included as repository guidance");
        assert_eq!(guidance.reason_code, "manifest_reference");
        assert!(!guidance.editable);
    }

    #[test]
    fn build_omits_secret_like_repository_guidance() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("AGENTS.md"),
            [
                "Do not leak sk",
                "-ant-api03-abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuAA.\n",
            ]
            .concat(),
        )
        .unwrap();

        let packet = ContextBuilder::new()
            .run_id(RunId("20260101-120000-abcdef06".to_string()))
            .task_card("feature")
            .repo_root(dir.path())
            .provider("glm")
            .model("glm-5.1")
            .build()
            .unwrap();

        assert!(!packet.included.iter().any(|item| item.path == "AGENTS.md"));
        let omitted = packet
            .omitted_candidates
            .iter()
            .find(|item| item.path == "AGENTS.md")
            .expect("secret-like AGENTS.md should be omitted with evidence");
        assert_eq!(omitted.reason_for_omission, "secret_risk");
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
