//! Eval harness for local, replayable context quality checks.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use mimir_context::ContextBuilder;
use mimir_runs::RunId;
use mimir_schemas::{ContextPacket, EvalCase, EvalCaseGold, EvalMetrics, EvalResult};
use serde::Deserialize;

const DEFAULT_PROVIDER: &str = "glm";
const DEFAULT_MODEL: &str = "glm-5.1";
#[cfg(test)]
const DEFAULT_CAP_TOKENS: u32 = 64_000;

/// A complete context eval dataset.
#[derive(Debug, Clone, Deserialize)]
pub struct ContextEvalDataset {
    /// Dataset schema version.
    pub schema_version: u32,
    /// Stable dataset identifier.
    pub id: String,
    /// Human-readable dataset description.
    #[serde(default)]
    pub description: String,
    /// Eval cases to run.
    pub cases: Vec<EvalCase>,
}

/// Summary for a completed context dataset run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextEvalSummary {
    /// Dataset identifier.
    pub dataset_id: String,
    /// Number of cases executed.
    pub case_count: usize,
    /// Number of cases that met basic local quality gates.
    pub passed_count: usize,
    /// Mean file recall across cases.
    pub mean_file_recall: f64,
    /// Mean precision across cases.
    pub mean_precision: f64,
    /// Whether every packet stayed within the requested cap.
    pub cap_compliance: bool,
    /// Mean file recall grouped by eval mode.
    pub mode_file_recall: BTreeMap<String, f64>,
    /// Whether mode 4 beats mode 0 on mean file recall in this dataset.
    pub mode4_outperforms_mode0: bool,
    /// Total local input tokens across packets.
    pub tokens_in_total: u32,
}

/// Result of running a context eval dataset.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextEvalRun {
    /// Aggregate run summary.
    pub summary: ContextEvalSummary,
    /// Per-case schema-shaped results.
    pub results: Vec<EvalResult>,
}

/// Load a context eval dataset from a YAML file.
pub fn load_context_dataset(path: impl AsRef<Path>) -> Result<ContextEvalDataset> {
    let path = path.as_ref();
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read eval dataset {}", path.display()))?;
    let dataset: ContextEvalDataset = serde_yaml::from_str(&data)
        .with_context(|| format!("failed to parse eval dataset {}", path.display()))?;
    if dataset.schema_version != 1 {
        anyhow::bail!(
            "unsupported eval dataset schema_version {}; expected 1",
            dataset.schema_version
        );
    }
    if dataset.cases.is_empty() {
        anyhow::bail!("eval dataset {} contains no cases", path.display());
    }
    Ok(dataset)
}

/// Run all cases in a context eval dataset.
pub fn run_context_dataset(path: impl AsRef<Path>, cap_tokens: u32) -> Result<ContextEvalRun> {
    let path = path.as_ref();
    let dataset = load_context_dataset(path)?;
    let dataset_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut results = Vec::with_capacity(dataset.cases.len());
    for case in &dataset.cases {
        results.push(run_context_case(case, dataset_dir, cap_tokens)?);
    }
    let summary = summarize_results(&dataset.id, &results);
    Ok(ContextEvalRun { summary, results })
}

/// Run one context eval case without dispatching to a provider.
pub fn run_context_case(
    case: &EvalCase,
    dataset_dir: &Path,
    cap_tokens: u32,
) -> Result<EvalResult> {
    let start = Instant::now();
    let repo_root = resolve_repo_path(dataset_dir, &case.repo_path);
    let run_id = RunId::generate();
    let packet = ContextBuilder::new()
        .run_id(run_id.clone())
        .task_card(&case.task)
        .mode(&case.allowed_mode)
        .provider(DEFAULT_PROVIDER)
        .model(DEFAULT_MODEL)
        .repo_root(&repo_root)
        .edit_targets(case.gold.files.clone())
        .build()
        .with_context(|| format!("failed to build context packet for {}", case.id))?;
    Ok(result_from_packet(
        case,
        &packet,
        cap_tokens,
        start.elapsed().as_millis(),
    ))
}

fn resolve_repo_path(dataset_dir: &Path, repo_path: &str) -> PathBuf {
    let path = PathBuf::from(repo_path);
    if path.is_absolute() {
        return path;
    }
    let cwd_candidate = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(&path);
    if cwd_candidate.exists() {
        cwd_candidate
    } else {
        dataset_dir.join(path)
    }
}

fn result_from_packet(
    case: &EvalCase,
    packet: &ContextPacket,
    cap_tokens: u32,
    elapsed_ms: u128,
) -> EvalResult {
    let expected_files = case.gold.files.iter().cloned().collect::<BTreeSet<_>>();
    let included_files = packet
        .included
        .iter()
        .map(|item| item.path.clone())
        .collect::<BTreeSet<_>>();
    let recalled_files = expected_files
        .intersection(&included_files)
        .cloned()
        .collect::<BTreeSet<_>>();
    let file_recall = ratio(recalled_files.len(), expected_files.len());
    let precision = ratio(recalled_files.len(), included_files.len());
    let range_recall = range_recall(&case.gold, packet, file_recall);
    let critical_omission_count = expected_files.difference(&included_files).count() as u32;
    let cap_compliance = packet
        .estimated_input_tokens
        .saturating_add(packet.output_reserve_tokens)
        .saturating_add(packet.count_drift_reserve_tokens)
        <= cap_tokens;
    let useful_lines = useful_line_count(&case.gold);
    let tokens_per_useful_line = if useful_lines == 0 {
        None
    } else {
        Some(packet.estimated_input_tokens as f64 / useful_lines as f64)
    };
    let elapsed_ms = elapsed_ms.min(u32::MAX as u128) as u32;

    EvalResult {
        schema_version: 1,
        case_id: case.id.clone(),
        mode: mode_name_for_case(case),
        cap_tokens,
        model: Some(packet.model.clone()),
        provider: Some(packet.provider.clone()),
        metrics: EvalMetrics {
            file_recall,
            range_recall,
            precision,
            tokens_per_useful_line,
            critical_omission_count,
            cap_compliance,
            token_count_agreement_p95: None,
            packet_build_latency_ms: Some(elapsed_ms),
            retrieval_latency_ms: Some(elapsed_ms),
            repo_map_refresh_latency_ms: Some(0),
            e2e_latency_ms: elapsed_ms,
            tokens_in_total: packet.estimated_input_tokens,
            tokens_out_total: 0,
            cost_usd_total: 0.0,
            repair_turns: Some(0),
            override_used: Some(false),
            outcome: None,
            total_cost_to_success: None,
            index_cache_hit_rate: None,
            prompt_cache_hit_rate: None,
        },
        packet_id: Some(packet.packet_id.clone()),
        run_id: Some(packet.run_id.clone()),
        ran_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn summarize_results(dataset_id: &str, results: &[EvalResult]) -> ContextEvalSummary {
    let case_count = results.len();
    let passed_count = results
        .iter()
        .filter(|result| {
            result.metrics.cap_compliance
                && result.metrics.critical_omission_count == 0
                && result.metrics.file_recall >= 1.0
        })
        .count();
    let mean_file_recall = mean(results.iter().map(|result| result.metrics.file_recall));
    let mean_precision = mean(results.iter().map(|result| result.metrics.precision));
    let cap_compliance = results.iter().all(|result| result.metrics.cap_compliance);
    let mode_file_recall = mean_file_recall_by_mode(results);
    let mode4_outperforms_mode0 = mode_file_recall
        .get("4-rank-recall-subagents")
        .copied()
        .unwrap_or(0.0)
        > mode_file_recall.get("0-baseline").copied().unwrap_or(0.0);
    let tokens_in_total = results
        .iter()
        .map(|result| result.metrics.tokens_in_total)
        .sum();
    ContextEvalSummary {
        dataset_id: dataset_id.to_string(),
        case_count,
        passed_count,
        mean_file_recall,
        mean_precision,
        cap_compliance,
        mode_file_recall,
        mode4_outperforms_mode0,
        tokens_in_total,
    }
}

fn mean_file_recall_by_mode(results: &[EvalResult]) -> BTreeMap<String, f64> {
    let mut grouped = BTreeMap::<String, Vec<f64>>::new();
    for result in results {
        grouped
            .entry(result.mode.clone())
            .or_default()
            .push(result.metrics.file_recall);
    }
    grouped
        .into_iter()
        .map(|(mode, values)| (mode, mean(values.into_iter())))
        .collect()
}

fn range_recall(gold: &EvalCaseGold, packet: &ContextPacket, fallback: f64) -> f64 {
    if gold.ranges.is_empty() {
        return fallback;
    }
    let recalled = gold
        .ranges
        .iter()
        .filter(|gold_range| {
            packet.included.iter().any(|included| {
                included.path == gold_range.path
                    && included.ranges.iter().any(|range| {
                        let included_end = if range.end == u32::MAX {
                            gold_range.end
                        } else {
                            range.end
                        };
                        range.start <= gold_range.end && included_end >= gold_range.start
                    })
            })
        })
        .count();
    ratio(recalled, gold.ranges.len())
}

fn useful_line_count(gold: &EvalCaseGold) -> u32 {
    gold.ranges
        .iter()
        .map(|range| range.end.saturating_sub(range.start).saturating_add(1))
        .sum()
}

fn mode_name_for_case(case: &EvalCase) -> String {
    for (needle, mode) in [
        ("mode0", "0-baseline"),
        ("mode1", "1-cap-only"),
        ("mode2", "2-rank"),
        ("mode3", "3-rank-recall"),
        ("mode4", "4-rank-recall-subagents"),
        ("mode5", "5-full"),
    ] {
        if case.id.contains(needle) {
            return mode.to_string();
        }
    }
    match case.allowed_mode.as_str() {
        "ask" => "0-baseline",
        "plan" => "2-rank",
        "review" | "explain" => "3-rank-recall",
        "code" => "5-full",
        _ => "0-baseline",
    }
    .to_string()
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let mut count = 0usize;
    let mut total = 0.0;
    for value in values {
        count += 1;
        total += value;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_dataset_reads_schema_shaped_cases() {
        let dataset_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/context-recall-v1.yaml");
        let dataset = load_context_dataset(dataset_path).unwrap();
        assert_eq!(dataset.schema_version, 1);
        assert_eq!(dataset.cases.len(), 15);
        assert!(dataset.cases.iter().any(|case| case.id.contains("mode5")));
    }

    #[test]
    fn run_context_case_preserves_schema_result_shape() {
        let case = EvalCase {
            schema_version: 1,
            id: "fixture-01-mode0-simple-ask".to_string(),
            repo_path: "../..".to_string(),
            base_commit: "e51ee9e".to_string(),
            task: "Explain what the ContextBuilder does".to_string(),
            gold: EvalCaseGold {
                files: vec!["crates/mimir-core/src/lib.rs".to_string()],
                ranges: vec![mimir_schemas::EvalCaseGoldRange {
                    path: "crates/mimir-core/src/lib.rs".to_string(),
                    start: 1,
                    end: 3,
                }],
                distractors: Vec::new(),
            },
            expected_tests: Vec::new(),
            allowed_mode: "ask".to_string(),
            allowed_caps_to_test: vec![DEFAULT_CAP_TOKENS],
            generated: false,
        };
        let dataset_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let result = run_context_case(&case, dataset_dir, DEFAULT_CAP_TOKENS).unwrap();
        assert_eq!(result.schema_version, 1);
        assert_eq!(result.mode, "0-baseline");
        assert!(result.metrics.cap_compliance);
    }

    #[test]
    fn summary_reports_mode4_vs_mode0() {
        let results = vec![
            EvalResult {
                schema_version: 1,
                case_id: "m0".to_string(),
                mode: "0-baseline".to_string(),
                cap_tokens: DEFAULT_CAP_TOKENS,
                model: None,
                provider: None,
                metrics: EvalMetrics {
                    file_recall: 0.25,
                    range_recall: 0.25,
                    precision: 1.0,
                    tokens_per_useful_line: None,
                    critical_omission_count: 1,
                    cap_compliance: true,
                    token_count_agreement_p95: None,
                    packet_build_latency_ms: None,
                    retrieval_latency_ms: None,
                    repo_map_refresh_latency_ms: None,
                    e2e_latency_ms: 0,
                    tokens_in_total: 1,
                    tokens_out_total: 0,
                    cost_usd_total: 0.0,
                    repair_turns: Some(0),
                    override_used: Some(false),
                    outcome: None,
                    total_cost_to_success: None,
                    index_cache_hit_rate: None,
                    prompt_cache_hit_rate: None,
                },
                packet_id: None,
                run_id: None,
                ran_at: "2026-05-22T00:00:00Z".to_string(),
            },
            EvalResult {
                schema_version: 1,
                case_id: "m4".to_string(),
                mode: "4-rank-recall-subagents".to_string(),
                cap_tokens: DEFAULT_CAP_TOKENS,
                model: None,
                provider: None,
                metrics: EvalMetrics {
                    file_recall: 1.0,
                    range_recall: 1.0,
                    precision: 1.0,
                    tokens_per_useful_line: None,
                    critical_omission_count: 0,
                    cap_compliance: true,
                    token_count_agreement_p95: None,
                    packet_build_latency_ms: None,
                    retrieval_latency_ms: None,
                    repo_map_refresh_latency_ms: None,
                    e2e_latency_ms: 0,
                    tokens_in_total: 1,
                    tokens_out_total: 0,
                    cost_usd_total: 0.0,
                    repair_turns: Some(0),
                    override_used: Some(false),
                    outcome: None,
                    total_cost_to_success: None,
                    index_cache_hit_rate: None,
                    prompt_cache_hit_rate: None,
                },
                packet_id: None,
                run_id: None,
                ran_at: "2026-05-22T00:00:00Z".to_string(),
            },
        ];
        let summary = summarize_results("test", &results);
        assert!(summary.mode4_outperforms_mode0);
    }
}
