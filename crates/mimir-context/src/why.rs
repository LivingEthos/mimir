//! `context why` — explain why a file was included or omitted in a context packet.
//!
//! Given a file path and a run ID, looks up the context packet for that run
//! and returns the reason code (or omission reason) for the specified path.

use mimir_runs::{RunDir, RunId};
use mimir_schemas::ContextPacket;
use std::path::Path;

/// Result of a `context_why` lookup.
#[derive(Debug, Clone, PartialEq)]
pub enum WhyResult {
    /// The file was included with this reason code.
    Included {
        /// Reason code from the candidate.
        reason_code: String,
        /// Token count of the included item.
        token_count: u32,
    },
    /// The file was omitted with this reason.
    Omitted {
        /// Reason for omission.
        reason: String,
        /// Token count of the omitted candidate.
        token_count: u32,
    },
    /// The file was not found in the packet.
    NotFound,
}

/// Look up why a file was included or omitted in a context packet.
///
/// # Arguments
///
/// * `path` — The file path to look up.
/// * `run_id` — The run identifier (used to locate the packet under `.mimir/runs/`).
///
/// # Returns
///
/// A [`WhyResult`] indicating inclusion, omission, or not found.
///
/// # Errors
///
/// Returns an error if the packet file cannot be read or parsed.
///
/// # Example
///
/// ```no_run
/// use mimir_context::why::context_why;
///
/// // This will return NotFound because the run does not exist:
/// let result = context_why("src/main.rs", "20260101-120000-abcdef01").unwrap();
/// assert!(matches!(result, mimir_context::why::WhyResult::NotFound));
/// ```
pub fn context_why(path: impl AsRef<Path>, run_id: &str) -> Result<WhyResult, anyhow::Error> {
    let run_id = RunId::parse(run_id.to_string()).map_err(|_| anyhow::anyhow!("invalid run id"))?;
    let run_dir = RunDir::open(&camino::Utf8PathBuf::from(".mimir"), &run_id)
        .map_err(|_| anyhow::anyhow!("No packet found for run {}", run_id))?;
    let packet_path = run_dir.context_packet_path();
    let data = match std::fs::read_to_string(&packet_path) {
        Ok(d) => d,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to read packet for run {}: {}",
                run_id,
                e
            ));
        }
    };
    let packet: ContextPacket = serde_json::from_str(&data)?;

    let lookup = path.as_ref().to_string_lossy().to_string();

    // Search included items first.
    for item in &packet.included {
        if item.path == lookup {
            return Ok(WhyResult::Included {
                reason_code: item.reason_code.clone(),
                token_count: item.tokens,
            });
        }
    }

    // Search omitted candidates.
    for omitted in &packet.omitted_candidates {
        if omitted.path == lookup {
            return Ok(WhyResult::Omitted {
                reason: omitted.reason_for_omission.clone(),
                token_count: omitted.estimated_tokens,
            });
        }
    }

    Ok(WhyResult::NotFound)
}

/// Look up why a file was included or omitted using an in-memory packet.
///
/// This is useful for testing and for callers that already have the packet loaded.
///
/// # Arguments
///
/// * `packet` — The context packet to search.
/// * `path` — The file path to look up.
///
/// # Returns
///
/// A [`WhyResult`] indicating inclusion, omission, or not found.
pub fn context_why_packet(packet: &ContextPacket, path: &str) -> WhyResult {
    for item in &packet.included {
        if item.path == path {
            return WhyResult::Included {
                reason_code: item.reason_code.clone(),
                token_count: item.tokens,
            };
        }
    }
    for omitted in &packet.omitted_candidates {
        if omitted.path == path {
            return WhyResult::Omitted {
                reason: omitted.reason_for_omission.clone(),
                token_count: omitted.estimated_tokens,
            };
        }
    }
    WhyResult::NotFound
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimir_schemas::{ContextPacket, ContextRange, IncludedItem, OmittedCandidate, TaskCard};

    fn sample_packet() -> ContextPacket {
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
            included: vec![IncludedItem {
                path: "src/main.rs".to_string(),
                ranges: vec![ContextRange { start: 1, end: 1 }],
                candidate_kind: "full_file".to_string(),
                reason_code: "direct_user_mention".to_string(),
                tokens: 10,
                source_hash: "0".repeat(64),
                trust_level: "trusted".to_string(),
                editable: false,
                compression: None,
            }],
            omitted_candidates: vec![OmittedCandidate {
                schema_version: 1,
                path: "src/old.rs".to_string(),
                ranges: Vec::new(),
                candidate_kind: "full_file".to_string(),
                reason_code: "embedding_match".to_string(),
                score: 0.0,
                features: serde_json::json!({}),
                estimated_tokens: 500,
                discovered_by: vec!["manifest".to_string()],
                source_hash: None,
                reason_for_omission: "budget_overflow".to_string(),
                risk: None,
                what_would_trigger_inclusion: "Increase the context budget.".to_string(),
            }],
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

    #[test]
    fn why_included() {
        let packet = sample_packet();
        let result = context_why_packet(&packet, "src/main.rs");
        assert_eq!(
            result,
            WhyResult::Included {
                reason_code: "direct_user_mention".to_string(),
                token_count: 10,
            }
        );
    }

    #[test]
    fn why_omitted() {
        let packet = sample_packet();
        let result = context_why_packet(&packet, "src/old.rs");
        assert_eq!(
            result,
            WhyResult::Omitted {
                reason: "budget_overflow".to_string(),
                token_count: 500,
            }
        );
    }

    #[test]
    fn why_not_found() {
        let packet = sample_packet();
        let result = context_why_packet(&packet, "src/missing.rs");
        assert_eq!(result, WhyResult::NotFound);
    }
}
