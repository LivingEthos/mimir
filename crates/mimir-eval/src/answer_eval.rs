//! Provider-backed answer-quality eval tier.
//!
//! This is **opt-in and key-gated**: it requires a provider API key in env.
//! If absent, the run exits with a clear "skipped: no key" status.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use mimir_context::{ContextBuilder, TokenPolicy};
use mimir_runs::RunId;

use serde::Deserialize;

/// A complete answer-quality eval dataset.
#[derive(Debug, Clone, Deserialize)]
pub struct AnswerQualityDataset {
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub cases: Vec<AnswerQualityCase>,
}

/// A single answer-quality case.
#[derive(Debug, Clone, Deserialize)]
pub struct AnswerQualityCase {
    pub id: String,
    pub repo_path: String,
    pub base_commit: String,
    pub task: String,
    pub gold_answer: String,
    #[serde(default = "default_grading")]
    pub grading: String,
}

fn default_grading() -> String {
    "exact_match".to_string()
}

/// Result of running one case in one arm.
#[derive(Debug, Clone)]
pub struct AnswerQualityRun {
    pub case_id: String,
    pub arm: String,
    pub answer: String,
    pub correct: bool,
    pub tokens_in: u32,
    pub provider: String,
    pub model: String,
}

/// Summary across arms.
#[derive(Debug, Clone)]
pub struct AnswerQualitySummary {
    pub dataset_id: String,
    pub arm_results: BTreeMap<String, Vec<AnswerQualityRun>>,
}

/// Load an answer-quality dataset from YAML.
pub fn load_answer_dataset(path: impl AsRef<Path>) -> Result<AnswerQualityDataset> {
    let path = path.as_ref();
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read answer dataset {}", path.display()))?;
    let dataset: AnswerQualityDataset = serde_yaml::from_str(&data)
        .with_context(|| format!("failed to parse answer dataset {}", path.display()))?;
    if dataset.schema_version != 1 {
        anyhow::bail!(
            "unsupported answer dataset schema_version {}; expected 1",
            dataset.schema_version
        );
    }
    Ok(dataset)
}

/// Build packets for both arms of a case and return them with grading metadata.
pub fn build_answer_packets(
    case: &AnswerQualityCase,
    provider: &str,
    model: &str,
) -> Result<(mimir_schemas::ContextPacket, mimir_schemas::ContextPacket)> {
    let repo_root = PathBuf::from(&case.repo_path);
    let repo_index = Arc::new(
        mimir_index::build_index(&repo_root)
            .with_context(|| format!("failed to build index for case {}", case.id))?,
    );

    let verbatim = ContextBuilder::new()
        .run_id(RunId::generate())
        .task_card(&case.task)
        .mode("ask")
        .provider(provider)
        .model(model)
        .repo_root(&repo_root)
        .repo_index(repo_index.clone())
        .token_policy(TokenPolicy {
            compression_enabled: false,
            ..TokenPolicy::default()
        })
        .build()
        .with_context(|| format!("failed to build verbatim packet for case {}", case.id))?;

    let compressed = ContextBuilder::new()
        .run_id(RunId::generate())
        .task_card(&case.task)
        .mode("ask")
        .provider(provider)
        .model(model)
        .repo_root(&repo_root)
        .repo_index(repo_index)
        .token_policy(TokenPolicy {
            compression_enabled: true,
            ..TokenPolicy::default()
        })
        .build()
        .with_context(|| format!("failed to build compressed packet for case {}", case.id))?;

    Ok((verbatim, compressed))
}

/// Build both arms for every case and report input-token savings.
///
/// This is the **offline, key-free, CI-safe** core of the answer-quality tier:
/// it builds the verbatim and compressed packets for each case and compares
/// `estimated_input_tokens`, proving the compressed arm sends fewer tokens for
/// the same retrieval. It performs no provider dispatch, so it runs anywhere.
///
/// # Errors
/// Returns an error if a case repo cannot be indexed or a packet cannot be built.
pub fn token_savings_report(
    dataset: &AnswerQualityDataset,
    provider: &str,
    model: &str,
) -> Result<serde_json::Value> {
    let mut cases = Vec::new();
    let mut total_verbatim: u64 = 0;
    let mut total_compressed: u64 = 0;

    for case in &dataset.cases {
        let (verbatim, compressed) = build_answer_packets(case, provider, model)?;
        let v = u64::from(verbatim.estimated_input_tokens);
        let c = u64::from(compressed.estimated_input_tokens);
        let compressed_items = compressed
            .included
            .iter()
            .filter(|item| item.compression.is_some())
            .count();
        total_verbatim += v;
        total_compressed += c;
        cases.push(serde_json::json!({
            "case_id": case.id,
            "verbatim_tokens_in": v,
            "compressed_tokens_in": c,
            "tokens_saved": v.saturating_sub(c),
            "compressed_items": compressed_items,
        }));
    }

    let saved = total_verbatim.saturating_sub(total_compressed);
    let savings_pct = if total_verbatim > 0 {
        (saved as f64 / total_verbatim as f64) * 100.0
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "dataset_id": dataset.id,
        "provider": provider,
        "model": model,
        "cases": cases,
        "totals": {
            "verbatim_tokens_in": total_verbatim,
            "compressed_tokens_in": total_compressed,
            "tokens_saved": saved,
            "savings_pct": savings_pct,
        },
        "answer_grading": "not_run",
    }))
}

/// Grade an answer against the gold standard.
pub fn grade_answer(answer: &str, gold: &str, grading: &str) -> bool {
    match grading {
        "exact_match" => answer == gold,
        "contains" => answer.contains(gold),
        "contains_ci" => answer.to_lowercase().contains(&gold.to_lowercase()),
        _ => answer == gold,
    }
}

/// Summarize per-arm statistics.
pub fn summarize(summary: &AnswerQualitySummary) -> serde_json::Value {
    let mut arms = serde_json::Map::new();
    for (arm, runs) in &summary.arm_results {
        let total = runs.len();
        let correct = runs.iter().filter(|r| r.correct).count();
        let mean_tokens = if total > 0 {
            runs.iter().map(|r| r.tokens_in as f64).sum::<f64>() / total as f64
        } else {
            0.0
        };
        arms.insert(
            arm.clone(),
            serde_json::json!({
                "cases": total,
                "correct": correct,
                "accuracy": if total > 0 { correct as f64 / total as f64 } else { 0.0 },
                "mean_tokens_in": mean_tokens,
            }),
        );
    }
    serde_json::json!({
        "dataset_id": summary.dataset_id,
        "arms": arms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_exact_match() {
        assert!(grade_answer("hello", "hello", "exact_match"));
        assert!(!grade_answer("hello ", "hello", "exact_match"));
    }

    #[test]
    fn grade_contains() {
        assert!(grade_answer("fn foo() { hello }", "hello", "contains"));
        assert!(!grade_answer("fn foo() { hi }", "hello", "contains"));
    }

    #[test]
    fn token_savings_report_is_well_formed_and_never_increases_tokens() {
        let dir = tempfile::tempdir().unwrap();
        // A large, compressible file whose symbol matches the task so retrieval
        // is likely to include it.
        let body = "    let x = 1;\n".repeat(1200);
        let big = format!("pub fn bigsymbol() {{\n{body}\n}}\n");
        std::fs::write(dir.path().join("bigsymbol.rs"), big).unwrap();

        let dataset = AnswerQualityDataset {
            schema_version: 1,
            id: "test-ds".to_string(),
            description: String::new(),
            cases: vec![AnswerQualityCase {
                id: "c1".to_string(),
                repo_path: dir.path().to_string_lossy().to_string(),
                base_commit: "HEAD".to_string(),
                task: "Explain bigsymbol".to_string(),
                gold_answer: "x".to_string(),
                grading: "contains".to_string(),
            }],
        };

        let report = token_savings_report(&dataset, "glm", "glm-5.1").unwrap();
        let totals = &report["totals"];
        let verbatim = totals["verbatim_tokens_in"].as_u64().unwrap();
        let compressed = totals["compressed_tokens_in"].as_u64().unwrap();
        // Compression must never inflate the input.
        assert!(
            compressed <= verbatim,
            "compressed {compressed} should be <= verbatim {verbatim}"
        );
        assert_eq!(report["answer_grading"], "not_run");
        assert_eq!(report["cases"].as_array().unwrap().len(), 1);
    }
}
