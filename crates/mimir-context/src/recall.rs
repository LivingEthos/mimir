//! Recall guard: flag high-risk omissions per Stage 7 of 09-RETRIEVAL-PIPELINE.md.
//!
//! The recall guard performs a fast, deterministic pass over the candidate
//! manifest to flag omissions that could materially reduce model performance.

use mimir_schemas::{CandidateManifest, ContextPacket, RecallGuardFlag};

/// Risk levels for recall guard flags.
pub mod risk {
    /// An included file imports an omitted file with lower relevance score.
    pub const IMPORT_ORPHAN: &str = "import_orphan";
    /// An omitted candidate is a route/schema/config file linked to an included edit target.
    pub const CONFIG_MISSING: &str = "config_missing";
    /// An omitted candidate is a schema file linked to an included edit target.
    pub const SCHEMA_MISSING: &str = "schema_missing";
    /// An omitted candidate is a test file for an included edit target.
    pub const TEST_MISSING: &str = "test_missing";
    /// An omitted candidate has a caller relationship to an included edit target.
    pub const CALLER_MISSING: &str = "caller_missing";
}

/// The recall guard analyzes a [`ContextPacket`] against its
/// [`CandidateManifest`] and produces [`RecallGuardFlag`] entries.
#[derive(Debug, Clone)]
pub struct RecallGuard {
    flags: Vec<RecallGuardFlag>,
}

impl RecallGuard {
    /// Create a new recall guard from a packet and its candidate manifest.
    ///
    /// # Arguments
    ///
    /// * `packet` — The packed context packet (contains `included` and `omitted_candidates`).
    /// * `manifest` — The full candidate manifest produced by retrieval.
    ///
    /// # Example
    ///
    /// ```
    /// use mimir_schemas::{CandidateManifest, ContextPacket, TaskCard};
    /// use mimir_context::recall::RecallGuard;
    ///
    /// let packet = ContextPacket {
    ///     schema_version: 1,
    ///     packet_id: "pkt-1".to_string(),
    ///     packet_hash: "0".repeat(64),
    ///     run_id: "r1".to_string(),
    ///     task_card: TaskCard {
    ///         goal: "test".to_string(),
    ///         acceptance_criteria: vec![],
    ///         likely_files: vec![],
    ///         risk_level: None,
    ///         expected_test_command: None,
    ///         unknowns: vec![],
    ///         need_for_large_context: None,
    ///         complexity: "low".to_string(),
    ///     },
    ///     mode: "code".to_string(),
    ///     cap_tokens: 64000,
    ///     target_tokens: 32000,
    ///     output_reserve_tokens: 4096,
    ///     count_drift_reserve_tokens: 512,
    ///     provider: "anthropic".to_string(),
    ///     model: "claude-sonnet-4-20250514".to_string(),
    ///     capability_snapshot_ref: "anthropic.yaml".to_string(),
    ///     prompt_contract_version: 1,
    ///     included: vec![],
    ///     omitted_candidates: vec![],
    ///     tool_schemas: vec![],
    ///     evidence_cards: vec![],
    ///     memory_entries: vec![],
    ///     budget_ledger_ref: ".mimir/runs/r1/budget_ledger.json".to_string(),
    ///     estimated_input_tokens: 0,
    ///     count_provenance: "local_estimate_only".to_string(),
    ///     created_at: "2026-01-01T00:00:00Z".to_string(),
    ///     authoritative_input_tokens: None,
    ///     recall_guard_flags: vec![],
    /// };
    /// let manifest = CandidateManifest {
    ///     schema_version: 1,
    ///     run_id: "r1".to_string(),
    ///     candidates: vec![],
    /// };
    /// let guard = RecallGuard::new(&packet, &manifest);
    /// assert!(guard.flags().is_empty());
    /// ```
    pub fn new(packet: &ContextPacket, manifest: &CandidateManifest) -> Self {
        let mut flags = Vec::new();

        let included_paths: std::collections::HashSet<&str> =
            packet.included.iter().map(|i| i.path.as_str()).collect();

        // Stage 7 rules from 09-RETRIEVAL-PIPELINE.md:
        // 1. If an included file imports an omitted file with reason lower_relevance_score, flag.
        // 2. If an omitted candidate is a route/schema/config file linked to an included edit target, flag.
        // 3. If an omitted candidate is a test file for an included edit target, flag.
        // 4. If an omitted candidate has failing_test_reference reason but was dropped for budget, flag.

        for omitted in &packet.omitted_candidates {
            let path = &omitted.path;
            let reason = &omitted.reason_for_omission;

            // Rule 1: import orphan — omitted file is imported by an included file.
            // (Heuristic: if the omitted path appears in the manifest with a caller/callee
            // relationship to an included path, flag it.)
            if (reason.contains("relevance") || reason.contains("lower"))
                && manifest_has_link_to_included(manifest, path, &included_paths)
            {
                flags.push(RecallGuardFlag {
                    risk: risk::IMPORT_ORPHAN.to_string(),
                    path: path.clone(),
                    reason: format!(
                        "Omitted file {} is linked to an included file but was dropped for lower relevance",
                        path
                    ),
                    suggestion: Some(
                        "Consider expanding retrieval or relaxing relevance threshold.".to_string(),
                    ),
                });
            }

            // Rule 2: config/schema missing
            if is_config_or_schema(path) && is_linked_to_included(manifest, path, &included_paths) {
                flags.push(RecallGuardFlag {
                    risk: risk::CONFIG_MISSING.to_string(),
                    path: path.clone(),
                    reason: format!(
                        "Omitted config/schema file {} is linked to an included edit target",
                        path
                    ),
                    suggestion: Some(
                        "Include this config file or verify the edit target does not need it."
                            .to_string(),
                    ),
                });
            }

            // Rule 3: test missing
            if is_test_file(path) && is_linked_to_included(manifest, path, &included_paths) {
                flags.push(RecallGuardFlag {
                    risk: risk::TEST_MISSING.to_string(),
                    path: path.clone(),
                    reason: format!("Omitted test file {} covers an included edit target", path),
                    suggestion: Some(
                        "Include the test file to ensure the model can verify changes.".to_string(),
                    ),
                });
            }

            // Rule 4: failing test reference dropped for budget
            if reason.contains("budget") && is_linked_to_included(manifest, path, &included_paths) {
                flags.push(RecallGuardFlag {
                    risk: risk::CALLER_MISSING.to_string(),
                    path: path.clone(),
                    reason: format!(
                        "File {} was dropped due to budget but is linked to an included target",
                        path
                    ),
                    suggestion: Some(
                        "Increase target_tokens or reduce other included files.".to_string(),
                    ),
                });
            }
        }

        Self { flags }
    }

    /// Return the flags produced by the guard.
    pub fn flags(&self) -> &[RecallGuardFlag] {
        &self.flags
    }

    /// Return true if any high-risk flag was raised.
    pub fn has_risk(&self) -> bool {
        !self.flags.is_empty()
    }

    /// Return flags filtered by risk level.
    pub fn by_risk(&self, risk_level: &str) -> Vec<&RecallGuardFlag> {
        self.flags.iter().filter(|f| f.risk == risk_level).collect()
    }
}
/// Check if the manifest shows any link between `path` and an included path.
/// For the stub implementation, we use a simple heuristic: if the path appears
/// in the manifest and shares a directory prefix with any included file.
fn manifest_has_link_to_included(
    manifest: &CandidateManifest,
    path: &str,
    included_paths: &std::collections::HashSet<&str>,
) -> bool {
    let in_manifest = manifest.candidates.iter().any(|c| c.source_path == path);
    in_manifest && shares_directory_prefix(path, included_paths)
}

/// Check if `path` shares a directory prefix with any included path.
fn shares_directory_prefix(path: &str, included_paths: &std::collections::HashSet<&str>) -> bool {
    let path_prefix = std::path::Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or("");
    included_paths.iter().any(|inc| {
        let inc_prefix = std::path::Path::new(inc)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        !path_prefix.is_empty() && !inc_prefix.is_empty() && path_prefix == inc_prefix
    })
}

/// Check if the manifest shows any candidate linking `path` to an included path.
/// For config/schema files, also match by filename stem against included files.
fn is_linked_to_included(
    _manifest: &CandidateManifest,
    path: &str,
    included_paths: &std::collections::HashSet<&str>,
) -> bool {
    if shares_directory_prefix(path, included_paths) {
        return true;
    }
    // For config files, also consider them linked if the filename stem
    // matches any included file's stem (e.g., app.yaml &lt;-&gt; app.rs).
    let path_stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    included_paths.iter().any(|inc| {
        let inc_stem = std::path::Path::new(inc)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        !path_stem.is_empty() && path_stem == inc_stem
    })
}

/// Heuristic: is this path a config or schema file?
fn is_config_or_schema(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".json")
        || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".toml")
        || lower.ends_with(".config.js")
        || lower.ends_with(".config.ts")
        || lower.contains("schema")
        || lower.contains("config")
        || lower.contains("route")
}

/// Heuristic: is this path a test file?
fn is_test_file(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.contains("test")
        || lower.contains("spec")
        || lower.ends_with(".test.js")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".spec.rs")
        || lower.ends_with("_test.go")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimir_schemas::{
        CandidateManifest, ContextCandidate, ContextPacket, ContextRange, IncludedItem,
        OmittedCandidate, TaskCard,
    };

    fn empty_packet() -> ContextPacket {
        ContextPacket {
            schema_version: 1,
            packet_id: "pkt-1".to_string(),
            packet_hash: "0".repeat(64),
            run_id: "20260101-000000-00000001".to_string(),
            task_card: TaskCard {
                goal: "test".to_string(),
                acceptance_criteria: Vec::new(),
                likely_files: Vec::new(),
                risk_level: None,
                expected_test_command: None,
                unknowns: Vec::new(),
                need_for_large_context: None,
                complexity: "tiny".to_string(),
            },
            mode: "code".to_string(),
            cap_tokens: 64000,
            target_tokens: 32000,
            output_reserve_tokens: 4096,
            count_drift_reserve_tokens: 512,
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            capability_snapshot_ref: "anthropic.yaml".to_string(),
            prompt_contract_version: 1,
            included: vec![],
            omitted_candidates: vec![],
            tool_schemas: vec![],
            evidence_cards: vec![],
            memory_entries: vec![],
            budget_ledger_ref: ".mimir/runs/20260101-000000-00000001/budget_ledger.json"
                .to_string(),
            estimated_input_tokens: 0,
            count_provenance: "local_estimate_only".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            authoritative_input_tokens: None,
            recall_guard_flags: vec![],
        }
    }

    fn included(path: &str, tokens: u32) -> IncludedItem {
        IncludedItem {
            path: path.to_string(),
            ranges: vec![ContextRange { start: 1, end: 1 }],
            candidate_kind: "full_file".to_string(),
            reason_code: "direct_user_mention".to_string(),
            tokens,
            source_hash: "0".repeat(64),
            trust_level: "trusted".to_string(),
            editable: false,
        }
    }

    fn omitted(path: &str, reason: &str, tokens: u32) -> OmittedCandidate {
        OmittedCandidate {
            schema_version: 1,
            path: path.to_string(),
            ranges: Vec::new(),
            candidate_kind: "full_file".to_string(),
            reason_code: "embedding_match".to_string(),
            score: 0.0,
            features: serde_json::json!({}),
            estimated_tokens: tokens,
            discovered_by: vec!["manifest".to_string()],
            source_hash: None,
            reason_for_omission: reason.to_string(),
            risk: None,
            what_would_trigger_inclusion: "Increase retrieval relevance.".to_string(),
        }
    }

    fn empty_manifest() -> CandidateManifest {
        CandidateManifest {
            schema_version: 1,
            run_id: "r1".to_string(),
            candidates: vec![],
        }
    }

    #[test]
    fn empty_packet_no_flags() {
        let packet = empty_packet();
        let manifest = empty_manifest();
        let guard = RecallGuard::new(&packet, &manifest);
        assert!(!guard.has_risk());
        assert!(guard.flags().is_empty());
    }

    #[test]
    fn flags_omitted_test_file() {
        let mut packet = empty_packet();
        packet.included.push(included("src/lib.rs", 100));
        packet
            .omitted_candidates
            .push(omitted("src/lib_test.rs", "lower_relevance_score", 50));
        let manifest = empty_manifest();
        let guard = RecallGuard::new(&packet, &manifest);
        assert!(guard.has_risk());
        let test_flags = guard.by_risk(risk::TEST_MISSING);
        assert_eq!(test_flags.len(), 1);
        assert_eq!(test_flags[0].path, "src/lib_test.rs");
    }

    #[test]
    fn flags_omitted_config_file() {
        let mut packet = empty_packet();
        packet.included.push(included("src/app.rs", 200));
        packet
            .omitted_candidates
            .push(omitted("config/app.yaml", "budget_overflow", 30));
        let manifest = empty_manifest();
        let guard = RecallGuard::new(&packet, &manifest);
        assert!(guard.has_risk());
        let config_flags = guard.by_risk(risk::CONFIG_MISSING);
        assert_eq!(config_flags.len(), 1);
        assert_eq!(config_flags[0].path, "config/app.yaml");
    }

    #[test]
    fn flags_budget_dropped_linked_file() {
        let mut packet = empty_packet();
        packet.included.push(included("src/main.rs", 300));
        packet
            .omitted_candidates
            .push(omitted("src/helper.rs", "budget_overflow", 100));
        let manifest = empty_manifest();
        let guard = RecallGuard::new(&packet, &manifest);
        assert!(guard.has_risk());
        let caller_flags = guard.by_risk(risk::CALLER_MISSING);
        assert_eq!(caller_flags.len(), 1);
        assert_eq!(caller_flags[0].path, "src/helper.rs");
    }

    #[test]
    fn flags_import_orphan() {
        let mut packet = empty_packet();
        packet.included.push(included("src/lib.rs", 100));
        packet
            .omitted_candidates
            .push(omitted("src/deps.rs", "lower_relevance_score", 40));
        let mut manifest = empty_manifest();
        manifest.candidates.push(ContextCandidate {
            source_path: "src/deps.rs".to_string(),
            token_count: 40,
            relevance_score: 0.5,
        });
        manifest.candidates.push(ContextCandidate {
            source_path: "src/lib.rs".to_string(),
            token_count: 100,
            relevance_score: 0.9,
        });
        let guard = RecallGuard::new(&packet, &manifest);
        assert!(guard.has_risk());
        let orphan_flags = guard.by_risk(risk::IMPORT_ORPHAN);
        assert_eq!(orphan_flags.len(), 1);
        assert_eq!(orphan_flags[0].path, "src/deps.rs");
    }

    #[test]
    fn no_flag_for_unlinked_omission() {
        let mut packet = empty_packet();
        packet.included.push(included("src/main.rs", 100));
        packet
            .omitted_candidates
            .push(omitted("other/unrelated.rs", "lower_relevance_score", 20));
        let manifest = empty_manifest();
        let guard = RecallGuard::new(&packet, &manifest);
        // The omitted file is in a different directory, so it should not be flagged.
        assert!(!guard.has_risk());
    }
}
