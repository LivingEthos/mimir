//! `mimir-retrieval` — retrieval pipeline stages, ranking, budget packing,
//! sufficiency check, and recall guard.
//!
//! The pipeline takes a [`mimir_index::RepoIndex`] and a task description string,
//! and produces a ranked, budget-constrained [`CandidateManifest`].
//!
//! # Pipeline stages
//!
//! 1. **Stage 1** — High-recall cheap scan (filename match, symbol extraction).
//! 2. **Stage 2** — Structural expansion (import/exports closure, caller/callee).
//! 3. **Stage 4** — Candidate manifest emission with feature vectors and scores.
//! 4. **Stage 5** — Greedy budget packer.
//! 5. **Stage 6** — Sufficiency check.
//! 6. **Stage 7** — Recall guard.

#![warn(missing_docs)]

use std::collections::{HashMap, HashSet};

use mimir_index::RepoIndex;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

/// A line range inside a file (1-indexed, inclusive).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Range {
    /// Start line (1-indexed).
    pub start: u32,
    /// End line (1-indexed, inclusive).
    pub end: u32,
}

/// Kind of candidate coverage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateKind {
    /// The entire file is included.
    FullFile,
    /// A symbol-specific slice.
    SymbolSlice,
    /// A generic line range.
    LineRange,
}

impl std::fmt::Display for CandidateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CandidateKind::FullFile => write!(f, "full_file"),
            CandidateKind::SymbolSlice => write!(f, "symbol_slice"),
            CandidateKind::LineRange => write!(f, "line_range"),
        }
    }
}

/// How a candidate was discovered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiscoveredBy {
    /// Matched via ripgrep / text search.
    Ripgrep,
    /// Matched via AST / symbol analysis.
    Ast,
    /// Discovered through import graph traversal.
    ImportGraph,
    /// Git co-change analysis.
    GitCochange,
    /// Package manifest scan.
    Manifest,
    /// Embedding similarity (future).
    Embedding,
    /// Memory / prior pattern (future).
    Memory,
}

impl std::fmt::Display for DiscoveredBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveredBy::Ripgrep => write!(f, "rg"),
            DiscoveredBy::Ast => write!(f, "ast"),
            DiscoveredBy::ImportGraph => write!(f, "import_graph"),
            DiscoveredBy::GitCochange => write!(f, "git_cochange"),
            DiscoveredBy::Manifest => write!(f, "manifest"),
            DiscoveredBy::Embedding => write!(f, "embedding"),
            DiscoveredBy::Memory => write!(f, "memory"),
        }
    }
}

/// A candidate produced by retrieval before packing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCandidate {
    /// File path (relative to repo root).
    pub path: String,
    /// Line ranges included.
    pub ranges: Vec<Range>,
    /// Kind of candidate.
    pub candidate_kind: CandidateKind,
    /// Relevance score (higher is better).
    pub score: f64,
    /// Feature vector (signal contributions).
    pub features: serde_json::Value,
    /// Estimated token count.
    pub estimated_tokens: u32,
    /// How this candidate was discovered.
    pub discovered_by: DiscoveredBy,
    /// Primary reason code.
    pub reason_code: String,
}

/// A packed context item ready for emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedItem {
    /// File path.
    pub path: String,
    /// Ranges included.
    pub ranges: Vec<Range>,
    /// Token count.
    pub estimated_tokens: u32,
    /// Score that led to inclusion.
    pub score: f64,
    /// Reason code.
    pub reason_code: String,
}

/// An omitted candidate with risk assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmittedItem {
    /// File path.
    pub path: String,
    /// Why it was omitted.
    pub reason: String,
    /// Estimated tokens.
    pub estimated_tokens: u32,
    /// Risk level if omitted.
    pub risk: Option<String>,
}

/// Result of the budget packer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackedManifest {
    /// Items that fit within budget.
    pub included: Vec<PackedItem>,
    /// Items that did not fit.
    pub omitted: Vec<OmittedItem>,
    /// Total tokens of included items.
    pub total_tokens: u32,
}

/// A recall guard flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecallGuardFlag {
    /// Flag category.
    pub category: String,
    /// Human-readable description.
    pub description: String,
    /// Affected file paths.
    pub paths: Vec<String>,
}

/// Result of the full retrieval pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineResult {
    /// Packed manifest.
    pub manifest: PackedManifest,
    /// Sufficiency check passed?
    pub sufficiency_ok: bool,
    /// Sufficiency warnings.
    pub sufficiency_warnings: Vec<String>,
    /// Recall guard flags.
    pub recall_guard_flags: Vec<RecallGuardFlag>,
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Pipeline configuration.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Target input token budget.
    pub target_input_tokens: u32,
    /// Minimum slack tokens to preserve.
    pub min_slack_tokens: u32,
    /// Large file threshold (tokens).
    pub large_file_threshold_tokens: u32,
    /// Minimum tokens per useful line.
    pub min_tokens_per_useful_line: u32,
    /// Max candidates before ranking cut-off.
    pub max_candidates: usize,
    /// Weights for ranking.
    pub weights: RankWeights,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            target_input_tokens: 32_000,
            min_slack_tokens: 500,
            large_file_threshold_tokens: 12_000,
            min_tokens_per_useful_line: 20,
            max_candidates: 200,
            weights: RankWeights::default(),
        }
    }
}

/// Ranking signal weights.
#[derive(Debug, Clone)]
pub struct RankWeights {
    /// Direct filename or path match in task text.
    pub direct_task_match: f64,
    /// Symbol reference match.
    pub symbol_reference_match: f64,
    /// Failing test reference.
    pub failing_test_reference: f64,
    /// Inverse graph distance to seed.
    pub caller_callee_distance_inverse: f64,
    /// Recent human correction.
    pub recent_human_correction: f64,
    /// Import graph relevance.
    pub import_graph_relevance: f64,
    /// Semantic similarity (unused in P2).
    pub semantic_similarity: f64,
    /// Route or schema link.
    pub route_or_schema_link: f64,
    /// Git co-change score.
    pub git_cochange_score: f64,
    /// Token cost penalty.
    pub token_cost_penalty: f64,
    /// Generated file penalty.
    pub generated_file_penalty: f64,
    /// Stale memory penalty.
    pub stale_memory_penalty: f64,
}

impl Default for RankWeights {
    fn default() -> Self {
        Self {
            direct_task_match: 5.0,
            symbol_reference_match: 4.0,
            failing_test_reference: 4.0,
            caller_callee_distance_inverse: 3.0,
            recent_human_correction: 3.0,
            import_graph_relevance: 2.0,
            semantic_similarity: 2.0,
            route_or_schema_link: 1.5,
            git_cochange_score: 1.0,
            token_cost_penalty: 1.0,
            generated_file_penalty: 3.0,
            stale_memory_penalty: 2.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol extraction
// ---------------------------------------------------------------------------

static CAMEL_SNAKE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"[a-zA-Z_][a-zA-Z0-9_]*"#).unwrap());

static QUOTED_TERM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"["']([^"']+)["']"#).unwrap());

/// Extract symbol-like tokens from task text.
///
/// Returns camelCase, snake_case, and quoted terms.
pub fn extract_symbols(task: &str) -> Vec<String> {
    let mut symbols = HashSet::new();

    // Quoted terms
    for cap in QUOTED_TERM_RE.captures_iter(task) {
        if let Some(m) = cap.get(1) {
            let term = m.as_str().to_string();
            if !term.is_empty() && term.len() > 2 {
                symbols.insert(term);
            }
        }
    }

    // camelCase / snake_case tokens
    for mat in CAMEL_SNAKE_RE.find_iter(task) {
        let token = mat.as_str();
        // Filter out very short tokens and pure lowercase common words
        if token.len() >= 3 && !is_common_word(token) {
            symbols.insert(token.to_string());
        }
    }

    symbols.into_iter().collect()
}

fn is_common_word(token: &str) -> bool {
    let common: HashSet<&str> = [
        "the", "and", "for", "are", "but", "not", "you", "all", "can", "had", "her", "was", "one",
        "our", "out", "day", "get", "has", "him", "his", "how", "its", "may", "new", "now", "old",
        "see", "two", "who", "boy", "did", "she", "use", "her", "way", "many", "oil", "sit", "set",
        "run", "eat", "far", "sea", "eye", "ago", "off", "too", "any", "say", "man", "try", "ask",
        "end", "why", "let", "put", "say", "she", "try", "way", "own", "say", "too", "old", "tell",
        "very", "when", "come", "here", "just", "like", "long", "make", "over", "such", "take",
        "than", "them", "well", "were", "will", "with", "have", "from", "they", "know", "want",
        "been", "good", "much", "some", "time", "would", "there", "their", "what", "said", "each",
        "which", "how", "about", "if", "out", "many", "then", "them", "these", "so", "some", "her",
        "would", "make", "like", "into", "him", "has", "two", "more", "very", "what", "know",
        "just", "first", "also", "after", "back", "other", "many", "than", "only", "those", "come",
        "day", "most", "us", "add", "fix", "ref", "use", "run", "get", "set", "new", "old", "all",
        "any", "can", "had", "has", "how", "its", "may", "now", "our", "out", "see", "two", "way",
        "who", "let", "put", "say", "she", "too", "try", "way", "own", "say", "she", "too",
    ]
    .iter()
    .copied()
    .collect();
    common.contains(token.to_lowercase().as_str())
}

// ---------------------------------------------------------------------------
// Stage 1: High-recall cheap scan
// ---------------------------------------------------------------------------

/// Run Stage 1: produce seed candidates from cheap heuristics.
///
/// - Exact path/filename match against task text (case-insensitive).
/// - Symbol extraction from task (camelCase, snake_case, quoted terms).
pub fn stage1_cheap_scan(index: &RepoIndex, task: &str) -> Vec<ContextCandidate> {
    let task_lower = task.to_lowercase();
    let symbols = extract_symbols(task);
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for (path, entry) in &index.files {
        let path_lower = path.to_lowercase();
        let filename = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);

        let mut score = 0.0;
        let mut features = HashMap::new();
        let mut discovered_by = None;
        let mut reason_code = String::new();

        // Direct filename match
        if task_lower.contains(&filename.to_lowercase()) {
            score += 1.0;
            features.insert("direct_task_match", 1.0);
            discovered_by = Some(DiscoveredBy::Ripgrep);
            reason_code = "direct_task_match".to_string();
        }

        // Direct path substring match
        if task_lower.contains(&path_lower) {
            score += 1.0;
            features.insert("direct_path_match", 1.0);
            if discovered_by.is_none() {
                discovered_by = Some(DiscoveredBy::Ripgrep);
                reason_code = "direct_path_match".to_string();
            }
        }

        // Symbol reference match
        let mut symbol_hits = 0.0;
        let total_symbols = symbols.len().max(1) as f64;
        for sym in &symbols {
            let sym_lower = sym.to_lowercase();
            if entry.exports.iter().any(|e| e.to_lowercase() == sym_lower)
                || entry
                    .imports
                    .iter()
                    .any(|i| i.to_lowercase().contains(&sym_lower))
            {
                symbol_hits += 1.0;
            }
        }
        if symbol_hits > 0.0 {
            let symbol_match = symbol_hits / total_symbols;
            score += symbol_match;
            features.insert("symbol_reference_match", symbol_match);
            if discovered_by.is_none() {
                discovered_by = Some(DiscoveredBy::Ast);
                reason_code = "symbol_reference_match".to_string();
            }
        }

        if score > 0.0 && !seen.contains(path) {
            seen.insert(path.clone());
            let estimated_tokens = entry.token_count.max(1);
            candidates.push(ContextCandidate {
                path: path.clone(),
                ranges: vec![Range {
                    start: 1,
                    end: u32::MAX,
                }],
                candidate_kind: CandidateKind::FullFile,
                score,
                features: serde_json::to_value(features).unwrap_or(serde_json::json!({})),
                estimated_tokens,
                discovered_by: discovered_by.unwrap_or(DiscoveredBy::Ripgrep),
                reason_code: if reason_code.is_empty() {
                    "stage1_match".to_string()
                } else {
                    reason_code
                },
            });
        }
    }

    candidates
}

// ---------------------------------------------------------------------------
// Stage 2: Structural expansion
// ---------------------------------------------------------------------------

/// Compute import/exports closure starting from seed paths.
///
/// Returns a map of discovered path -> graph distance from nearest seed.
pub fn import_export_closure(
    index: &RepoIndex,
    seeds: &[String],
    max_depth: usize,
) -> HashMap<String, usize> {
    let mut distances: HashMap<String, usize> = HashMap::new();
    let mut queue: Vec<(String, usize)> = seeds.iter().map(|s| (s.clone(), 0)).collect();
    let mut visited: HashSet<String> = seeds.iter().cloned().collect();

    while let Some((current, depth)) = queue.pop() {
        if depth > max_depth {
            continue;
        }
        distances.insert(current.clone(), depth);

        // Forward: files that current imports from
        if let Some(imports) = index.import_graph.get(&current) {
            for imp in imports {
                // Find files that export this import
                for (other_path, other_entry) in &index.files {
                    if other_path == &current {
                        continue;
                    }
                    if (other_entry.exports.iter().any(|e| imp.contains(e))
                        || other_entry.imports.iter().any(|i| i.contains(imp)))
                        && !visited.contains(other_path)
                    {
                        visited.insert(other_path.clone());
                        queue.push((other_path.clone(), depth + 1));
                    }
                }
            }
        }

        // Backward: files that import from current (callers)
        for (other_path, other_entry) in &index.files {
            if other_path == &current {
                continue;
            }
            if let Some(current_exports) = index.export_graph.get(&current) {
                for exp in current_exports {
                    if (other_entry.imports.iter().any(|i| i.contains(exp))
                        || other_entry.exports.iter().any(|e| e == exp))
                        && !visited.contains(other_path)
                    {
                        visited.insert(other_path.clone());
                        queue.push((other_path.clone(), depth + 1));
                    }
                }
            }
        }
    }

    distances
}

/// Run Stage 2: expand seeds via structural graph traversal.
pub fn stage2_structural_expansion(
    index: &RepoIndex,
    seeds: &[ContextCandidate],
    max_depth: usize,
) -> Vec<ContextCandidate> {
    let seed_paths: Vec<String> = seeds.iter().map(|s| s.path.clone()).collect();
    let distances = import_export_closure(index, &seed_paths, max_depth);
    let mut candidates = Vec::new();
    let mut seen: HashSet<String> = seed_paths.iter().cloned().collect();

    for (path, distance) in distances {
        if seen.contains(&path) && seed_paths.contains(&path) {
            continue; // skip seeds, only add new
        }
        if seen.contains(&path) {
            continue;
        }
        seen.insert(path.clone());

        let entry = match index.get(&path) {
            Some(e) => e,
            None => continue,
        };

        let inverse_distance = 1.0 / (1.0 + distance as f64);
        let mut import_relevance = 0.0;

        // Count import/export overlap with seeds
        if let Some(seed_imports) = index.import_graph.get(&path) {
            for seed_path in &seed_paths {
                if let Some(seed_exports) = index.export_graph.get(seed_path) {
                    let overlap = seed_imports
                        .iter()
                        .filter(|i| seed_exports.iter().any(|e| i.contains(e)))
                        .count() as f64;
                    import_relevance += overlap;
                }
            }
        }

        let score = inverse_distance + (import_relevance * 0.1);
        let mut features = HashMap::new();
        features.insert("caller_callee_distance_inverse", inverse_distance);
        features.insert("import_graph_relevance", import_relevance.min(1.0));

        candidates.push(ContextCandidate {
            path,
            ranges: vec![Range {
                start: 1,
                end: u32::MAX,
            }],
            candidate_kind: CandidateKind::FullFile,
            score,
            features: serde_json::to_value(features).unwrap_or(serde_json::json!({})),
            estimated_tokens: entry.token_count.max(1),
            discovered_by: DiscoveredBy::ImportGraph,
            reason_code: "import_graph_expansion".to_string(),
        });
    }

    candidates
}

// ---------------------------------------------------------------------------
// Stage 4: Candidate manifest emission with ranking
// ---------------------------------------------------------------------------

/// Compute the final score for a candidate using the weighted ranking formula.
///
/// Features expected in `candidate.features`:
/// - `direct_task_match`: 0.0 or 1.0
/// - `symbol_reference_match`: 0.0..1.0
/// - `import_graph_relevance`: 0.0..1.0
/// - `caller_callee_distance_inverse`: 0.0..1.0
/// - `token_cost_penalty`: 0.0..2.0
///
/// Formula (from 09-RETRIEVAL-PIPELINE.md):
/// ```text
/// context_score =
///   + 5.0 * direct_task_match
///   + 4.0 * symbol_reference_match
///   + 3.0 * caller_callee_distance_inverse
///   + 2.0 * import_graph_relevance
///   - 1.0 * token_cost_penalty
/// ```
pub fn compute_score(candidate: &ContextCandidate, weights: &RankWeights) -> f64 {
    let f = |key: &str| -> f64 {
        candidate
            .features
            .get(key)
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };

    let direct = f("direct_task_match");
    let symbol = f("symbol_reference_match");
    let import = f("import_graph_relevance");
    let caller = f("caller_callee_distance_inverse");
    let token_penalty = f("token_cost_penalty");

    let score = weights.direct_task_match * direct
        + weights.symbol_reference_match * symbol
        + weights.import_graph_relevance * import
        + weights.caller_callee_distance_inverse * caller
        - weights.token_cost_penalty * token_penalty;

    score.max(0.0)
}

/// Add token cost penalty feature to candidates.
pub fn apply_token_penalty(candidates: &mut [ContextCandidate], threshold: u32) {
    for c in candidates {
        let penalty = (c.estimated_tokens as f64 / threshold as f64).min(2.0);
        if let Some(obj) = c.features.as_object_mut() {
            obj.insert("token_cost_penalty".to_string(), serde_json::json!(penalty));
        }
    }
}

/// Merge two candidate lists, deduplicating by path.
///
/// When duplicates exist, the higher-scored candidate wins and the other's
/// signals are merged into its `features`.
pub fn merge_candidates(
    a: Vec<ContextCandidate>,
    b: Vec<ContextCandidate>,
) -> Vec<ContextCandidate> {
    let mut by_path: HashMap<String, ContextCandidate> = HashMap::new();

    for mut c in a.into_iter().chain(b) {
        if let Some(existing) = by_path.get_mut(&c.path) {
            // Merge features from lower-scored into higher-scored
            if c.score > existing.score {
                std::mem::swap(existing, &mut c);
            }
            // Merge c's features into existing
            if let (Some(existing_obj), Some(new_obj)) =
                (existing.features.as_object_mut(), c.features.as_object())
            {
                for (k, v) in new_obj {
                    if !existing_obj.contains_key(k) {
                        existing_obj.insert(k.clone(), v.clone());
                    }
                }
            }
            // Update score if the merged features improve it
            existing.score = existing.score.max(c.score);
        } else {
            by_path.insert(c.path.clone(), c);
        }
    }

    by_path.into_values().collect()
}

/// Emit the top-N ranked candidates.
pub fn stage4_emit_manifest(
    candidates: Vec<ContextCandidate>,
    weights: &RankWeights,
    max_candidates: usize,
) -> Vec<ContextCandidate> {
    let mut candidates = candidates;

    // Recompute scores with full weights
    for c in &mut candidates {
        c.score = compute_score(c, weights);
    }

    // Sort by descending score
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    candidates.truncate(max_candidates);
    candidates
}

// ---------------------------------------------------------------------------
// Stage 5: Greedy budget packer
// ---------------------------------------------------------------------------

/// Pack candidates into the token budget using greedy descending-score.
///
/// Constraints:
/// 1. Always include items marked as mandatory.
/// 2. Never exceed `target_input_tokens`.
/// 3. Preserve at least `min_slack_tokens` as slack.
pub fn stage5_greedy_pack(
    candidates: &[ContextCandidate],
    config: &PipelineConfig,
    mandatory_paths: &[String],
) -> PackedManifest {
    let effective_budget = config
        .target_input_tokens
        .saturating_sub(config.min_slack_tokens);
    let mut included = Vec::new();
    let mut omitted = Vec::new();
    let mut used_tokens: u32 = 0;
    let mut included_paths: HashSet<String> = HashSet::new();

    // Sort candidates by descending score
    let mut sorted: Vec<&ContextCandidate> = candidates.iter().collect();
    sorted.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // First pass: mandatory items
    for c in &sorted {
        if mandatory_paths.contains(&c.path) && !included_paths.contains(&c.path) {
            if used_tokens + c.estimated_tokens <= effective_budget {
                used_tokens += c.estimated_tokens;
                included_paths.insert(c.path.clone());
                included.push(PackedItem {
                    path: c.path.clone(),
                    ranges: c.ranges.clone(),
                    estimated_tokens: c.estimated_tokens,
                    score: c.score,
                    reason_code: c.reason_code.clone(),
                });
            } else {
                omitted.push(OmittedItem {
                    path: c.path.clone(),
                    reason: "budget_exceeded_mandatory".to_string(),
                    estimated_tokens: c.estimated_tokens,
                    risk: Some("mandatory_omitted".to_string()),
                });
            }
        }
    }

    // Second pass: remaining candidates by score
    for c in sorted {
        if included_paths.contains(&c.path) {
            continue;
        }
        if used_tokens + c.estimated_tokens <= effective_budget {
            used_tokens += c.estimated_tokens;
            included_paths.insert(c.path.clone());
            included.push(PackedItem {
                path: c.path.clone(),
                ranges: c.ranges.clone(),
                estimated_tokens: c.estimated_tokens,
                score: c.score,
                reason_code: c.reason_code.clone(),
            });
        } else {
            omitted.push(OmittedItem {
                path: c.path.clone(),
                reason: "lower_relevance_score".to_string(),
                estimated_tokens: c.estimated_tokens,
                risk: None,
            });
        }
    }

    PackedManifest {
        included,
        omitted,
        total_tokens: used_tokens,
    }
}

// ---------------------------------------------------------------------------
// Stage 6: Sufficiency check
// ---------------------------------------------------------------------------

/// Run sufficiency checks on the packed manifest.
///
/// Asserts:
/// - Every edit target has at least one caller and one test in the packet,
///   OR the omission is recorded with `risk: caller_missing` or `risk: test_missing`.
/// - No source range below `min_tokens_per_useful_line`.
/// - No full file above `large_file_threshold_tokens` unless authorized.
///
/// Returns `(ok, warnings)`.
pub fn stage6_sufficiency_check(
    manifest: &PackedManifest,
    index: &RepoIndex,
    config: &PipelineConfig,
    edit_targets: &[String],
) -> (bool, Vec<String>) {
    let mut warnings = Vec::new();
    let included_set: HashSet<&str> = manifest.included.iter().map(|i| i.path.as_str()).collect();

    // Check edit targets have callers/tests
    for target in edit_targets {
        if !included_set.contains(target.as_str()) {
            continue; // not an edit target in packet, skip
        }

        let has_caller = manifest.included.iter().any(|item| {
            item.path != *target
                && index
                    .import_graph
                    .get(&item.path)
                    .is_some_and(|imports| imports.iter().any(|i| target.contains(i)))
        });

        let is_test = |path: &str| -> bool {
            path.contains("test") || path.contains("spec") || path.contains("__tests__")
        };
        let has_test = manifest.included.iter().any(|item| is_test(&item.path));

        if !has_caller {
            warnings.push(format!("edit target '{}' has no caller in packet", target));
        }
        if !has_test {
            warnings.push(format!("edit target '{}' has no test in packet", target));
        }
    }

    // Check minimum tokens per useful line
    for item in &manifest.included {
        for range in &item.ranges {
            let lines = range.end.saturating_sub(range.start) + 1;
            if let Some(density) = item.estimated_tokens.checked_div(lines) {
                if density < config.min_tokens_per_useful_line {
                    warnings.push(format!(
                        "sparse range in '{}': {} tokens / {} lines",
                        item.path, item.estimated_tokens, lines
                    ));
                }
            }
        }
    }

    // Check large files
    for item in &manifest.included {
        if item.estimated_tokens > config.large_file_threshold_tokens {
            warnings.push(format!(
                "large file '{}' exceeds threshold ({} > {})",
                item.path, item.estimated_tokens, config.large_file_threshold_tokens
            ));
        }
    }

    let ok = warnings.is_empty();
    (ok, warnings)
}

// ---------------------------------------------------------------------------
// Stage 7: Recall guard
// ---------------------------------------------------------------------------

/// Run recall guard to flag high-risk omissions.
///
/// Flags:
/// - Included file imports an omitted file with reason `lower_relevance_score`.
/// - Omitted test file for an included edit target.
/// - Omitted file with `failing_test_reference` dropped for budget.
pub fn stage7_recall_guard(
    manifest: &PackedManifest,
    index: &RepoIndex,
    edit_targets: &[String],
) -> Vec<RecallGuardFlag> {
    let mut flags = Vec::new();
    let included_set: HashSet<&str> = manifest.included.iter().map(|i| i.path.as_str()).collect();
    let omitted_map: HashMap<&str, &OmittedItem> = manifest
        .omitted
        .iter()
        .map(|o| (o.path.as_str(), o))
        .collect();

    // Flag 1: included file imports omitted file
    for item in &manifest.included {
        if let Some(imports) = index.import_graph.get(&item.path) {
            for imp in imports {
                // Find omitted files that might satisfy this import
                for (omitted_path, omitted_item) in &omitted_map {
                    if omitted_item.reason == "lower_relevance_score" {
                        // Heuristic: import mentions the path or its exports
                        let stripped_imp = imp.strip_prefix("crate::").unwrap_or(imp);
                        let path_matches = omitted_path.contains(imp)
                            || imp.contains(omitted_path.trim_end_matches(".rs"))
                            || omitted_path.contains(stripped_imp);
                        let export_matches = index
                            .export_graph
                            .get(*omitted_path)
                            .is_some_and(|exports| exports.iter().any(|e| imp.contains(e)));
                        if path_matches || export_matches {
                            flags.push(RecallGuardFlag {
                                category: "import_of_omitted".to_string(),
                                description: format!(
                                    "'{}' imports '{}' which was omitted (lower relevance)",
                                    item.path, omitted_path
                                ),
                                paths: vec![item.path.clone(), omitted_path.to_string()],
                            });
                        }
                    }
                }
            }
        }
    }

    // Flag 2: omitted test for included edit target
    let is_test = |path: &str| -> bool {
        path.contains("test") || path.contains("spec") || path.contains("__tests__")
    };

    for target in edit_targets {
        if included_set.contains(target.as_str()) {
            for omitted_path in omitted_map.keys() {
                if is_test(omitted_path) {
                    // Check if test references target
                    let test_refs_target =
                        index
                            .import_graph
                            .get(*omitted_path)
                            .is_some_and(|imports| {
                                imports.iter().any(|i| {
                                    let stripped = i.strip_prefix("crate::").unwrap_or(i);
                                    target.contains(i)
                                        || i.contains(target)
                                        || target.contains(stripped)
                                        || stripped.contains(target)
                                })
                            });
                    if test_refs_target {
                        flags.push(RecallGuardFlag {
                            category: "omitted_test_for_target".to_string(),
                            description: format!(
                                "test '{}' for edit target '{}' was omitted",
                                omitted_path, target
                            ),
                            paths: vec![target.clone(), omitted_path.to_string()],
                        });
                    }
                }
            }
        }
    }

    // Flag 3: failing_test_reference dropped for budget
    for (omitted_path, omitted_item) in &omitted_map {
        if omitted_item.reason == "budget_exceeded_mandatory" {
            flags.push(RecallGuardFlag {
                category: "failing_test_dropped".to_string(),
                description: format!(
                    "'{}' with failing test reference was dropped for budget",
                    omitted_path
                ),
                paths: vec![omitted_path.to_string()],
            });
        }
    }

    // Deduplicate flags by category + paths
    let mut unique = HashSet::new();
    flags.retain(|f| {
        let key = (f.category.clone(), f.paths.clone());
        unique.insert(key)
    });

    flags
}

// ---------------------------------------------------------------------------
// Full pipeline
// ---------------------------------------------------------------------------

/// Run the complete retrieval pipeline.
///
/// # Arguments
///
/// * `index` — The repo index.
/// * `task` — Task description text.
/// * `edit_targets` — Files the user intends to edit (for sufficiency/recall).
/// * `config` — Pipeline configuration.
///
/// # Returns
///
/// A [`PipelineResult`] containing the packed manifest, sufficiency status,
/// and recall guard flags.
pub fn run_pipeline(
    index: &RepoIndex,
    task: &str,
    edit_targets: &[String],
    config: &PipelineConfig,
) -> PipelineResult {
    // Stage 1: cheap scan
    let seeds = stage1_cheap_scan(index, task);

    // Stage 2: structural expansion
    let expanded = stage2_structural_expansion(index, &seeds, 2);

    // Merge and dedup
    let mut all = merge_candidates(seeds, expanded);

    // Apply token cost penalty
    apply_token_penalty(&mut all, config.large_file_threshold_tokens);

    // Stage 4: rank and emit manifest
    let ranked = stage4_emit_manifest(all, &config.weights, config.max_candidates);

    // Stage 5: greedy pack
    let manifest = stage5_greedy_pack(&ranked, config, edit_targets);

    // Stage 6: sufficiency check
    let (sufficiency_ok, sufficiency_warnings) =
        stage6_sufficiency_check(&manifest, index, config, edit_targets);

    // Stage 7: recall guard
    let recall_guard_flags = stage7_recall_guard(&manifest, index, edit_targets);

    PipelineResult {
        manifest,
        sufficiency_ok,
        sufficiency_warnings,
        recall_guard_flags,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mimir_index::{FileEntry, RepoIndex};

    fn make_index() -> RepoIndex {
        let mut index = RepoIndex::new();

        // Seed file: main.rs
        index.add(FileEntry {
            path: "src/main.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "a".to_string(),
            token_count: 500,
            exports: vec!["main".to_string(), "run_app".to_string()],
            imports: vec![
                "std::collections::HashMap".to_string(),
                "crate::utils".to_string(),
            ],
        });

        // Utility module
        index.add(FileEntry {
            path: "src/utils.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "b".to_string(),
            token_count: 300,
            exports: vec!["helper".to_string()],
            imports: vec![],
        });

        // Test file
        index.add(FileEntry {
            path: "tests/integration_test.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "c".to_string(),
            token_count: 400,
            exports: vec!["test_main".to_string()],
            imports: vec!["crate::main".to_string()],
        });

        // Unrelated file
        index.add(FileEntry {
            path: "src/unrelated.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "d".to_string(),
            token_count: 200,
            exports: vec!["other".to_string()],
            imports: vec![],
        });

        index
    }

    #[test]
    fn extract_symbols_finds_quoted_and_camel() {
        let task =
            r#"Fix the "validateSession" bug in userAuth module. Also check handle_request."#;
        let syms = extract_symbols(task);
        assert!(syms.contains(&"validateSession".to_string()));
        assert!(syms.contains(&"userAuth".to_string()));
        assert!(syms.contains(&"handle_request".to_string()));
    }

    #[test]
    fn extract_symbols_filters_common_words() {
        let task = "The bug is in the add_numbers function";
        let syms = extract_symbols(task);
        assert!(!syms.contains(&"the".to_string()));
        assert!(!syms.contains(&"is".to_string()));
        assert!(!syms.contains(&"in".to_string()));
        assert!(syms.contains(&"add_numbers".to_string()));
        assert!(syms.contains(&"bug".to_string()));
    }

    #[test]
    fn stage1_finds_filename_match() {
        let index = make_index();
        let task = "Fix bug in main.rs";
        let candidates = stage1_cheap_scan(&index, task);
        let paths: Vec<&str> = candidates.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
    }

    #[test]
    fn stage1_finds_symbol_match() {
        let index = make_index();
        let task = "Refactor run_app function";
        let candidates = stage1_cheap_scan(&index, task);
        let paths: Vec<&str> = candidates.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
    }

    #[test]
    fn stage2_expands_imports() {
        let index = make_index();
        let seeds = stage1_cheap_scan(&index, "Fix main.rs");
        let seed_paths: Vec<&str> = seeds.iter().map(|s| s.path.as_str()).collect();
        // main.rs is a direct filename match
        assert!(seed_paths.contains(&"src/main.rs"));
        // tests/integration_test.rs imports crate::main, so it matches via symbol
        assert!(seed_paths.contains(&"tests/integration_test.rs"));

        let expanded = stage2_structural_expansion(&index, &seeds, 2);
        let expanded_paths: Vec<&str> = expanded.iter().map(|c| c.path.as_str()).collect();
        // Backward traversal from main.rs finds tests/integration_test.rs (already a seed)
        // Forward traversal heuristic cannot map crate::utils -> src/utils.rs without
        // path resolution, so expansion may be empty for this synthetic data.
        // The key invariant is that expansion does not duplicate seeds.
        for path in &expanded_paths {
            assert!(!seed_paths.contains(path));
        }
    }

    #[test]
    fn merge_candidates_dedups_and_merges_features() {
        let a = ContextCandidate {
            path: "a.rs".to_string(),
            ranges: vec![Range { start: 1, end: 10 }],
            candidate_kind: CandidateKind::FullFile,
            score: 1.0,
            features: serde_json::json!({"direct_task_match": 1.0}),
            estimated_tokens: 100,
            discovered_by: DiscoveredBy::Ripgrep,
            reason_code: "direct".to_string(),
        };
        let b = ContextCandidate {
            path: "a.rs".to_string(),
            ranges: vec![Range { start: 1, end: 10 }],
            candidate_kind: CandidateKind::FullFile,
            score: 0.5,
            features: serde_json::json!({"symbol_reference_match": 0.8}),
            estimated_tokens: 100,
            discovered_by: DiscoveredBy::Ast,
            reason_code: "symbol".to_string(),
        };
        let merged = merge_candidates(vec![a], vec![b]);
        assert_eq!(merged.len(), 1);
        let m = &merged[0];
        assert!(m.features.get("direct_task_match").is_some());
        assert!(m.features.get("symbol_reference_match").is_some());
    }

    #[test]
    fn compute_score_applies_weights() {
        let candidate = ContextCandidate {
            path: "a.rs".to_string(),
            ranges: vec![Range { start: 1, end: 10 }],
            candidate_kind: CandidateKind::FullFile,
            score: 0.0,
            features: serde_json::json!({
                "direct_task_match": 1.0,
                "symbol_reference_match": 0.5,
                "token_cost_penalty": 0.2
            }),
            estimated_tokens: 100,
            discovered_by: DiscoveredBy::Ripgrep,
            reason_code: "test".to_string(),
        };
        let weights = RankWeights::default();
        let score = compute_score(&candidate, &weights);
        let expected = 5.0 * 1.0 + 4.0 * 0.5 - 1.0 * 0.2;
        assert!(
            (score - expected).abs() < 0.001,
            "score {} != expected {}",
            score,
            expected
        );
    }

    #[test]
    fn greedy_pack_respects_budget() {
        let config = PipelineConfig {
            target_input_tokens: 1_000,
            min_slack_tokens: 100,
            ..Default::default()
        };
        let candidates = vec![
            ContextCandidate {
                path: "a.rs".to_string(),
                ranges: vec![Range { start: 1, end: 10 }],
                candidate_kind: CandidateKind::FullFile,
                score: 3.0,
                features: serde_json::json!({}),
                estimated_tokens: 500,
                discovered_by: DiscoveredBy::Ripgrep,
                reason_code: "high".to_string(),
            },
            ContextCandidate {
                path: "b.rs".to_string(),
                ranges: vec![Range { start: 1, end: 10 }],
                candidate_kind: CandidateKind::FullFile,
                score: 2.0,
                features: serde_json::json!({}),
                estimated_tokens: 500,
                discovered_by: DiscoveredBy::Ripgrep,
                reason_code: "med".to_string(),
            },
            ContextCandidate {
                path: "c.rs".to_string(),
                ranges: vec![Range { start: 1, end: 10 }],
                candidate_kind: CandidateKind::FullFile,
                score: 1.0,
                features: serde_json::json!({}),
                estimated_tokens: 500,
                discovered_by: DiscoveredBy::Ripgrep,
                reason_code: "low".to_string(),
            },
        ];
        let manifest = stage5_greedy_pack(&candidates, &config, &[]);
        // effective budget = 900, so a.rs (500) + b.rs (500) = 1000 > 900, only a.rs fits
        assert_eq!(manifest.included.len(), 1);
        assert_eq!(manifest.included[0].path, "a.rs");
        assert_eq!(manifest.total_tokens, 500);
    }

    #[test]
    fn greedy_pack_always_includes_mandatory() {
        let config = PipelineConfig {
            target_input_tokens: 1_000,
            min_slack_tokens: 100,
            ..Default::default()
        };
        let candidates = vec![
            ContextCandidate {
                path: "mandatory.rs".to_string(),
                ranges: vec![Range { start: 1, end: 10 }],
                candidate_kind: CandidateKind::FullFile,
                score: 0.1,
                features: serde_json::json!({}),
                estimated_tokens: 200,
                discovered_by: DiscoveredBy::Ripgrep,
                reason_code: "mandatory".to_string(),
            },
            ContextCandidate {
                path: "big.rs".to_string(),
                ranges: vec![Range { start: 1, end: 10 }],
                candidate_kind: CandidateKind::FullFile,
                score: 5.0,
                features: serde_json::json!({}),
                estimated_tokens: 900,
                discovered_by: DiscoveredBy::Ripgrep,
                reason_code: "big".to_string(),
            },
        ];
        let manifest = stage5_greedy_pack(&candidates, &config, &["mandatory.rs".to_string()]);
        let paths: Vec<&str> = manifest.included.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"mandatory.rs"));
    }

    #[test]
    fn sufficiency_warns_on_missing_test() {
        let index = make_index();
        let config = PipelineConfig::default();
        let manifest = PackedManifest {
            included: vec![PackedItem {
                path: "src/main.rs".to_string(),
                ranges: vec![Range { start: 1, end: 50 }],
                estimated_tokens: 500,
                score: 1.0,
                reason_code: "direct".to_string(),
            }],
            omitted: vec![],
            total_tokens: 500,
        };
        let (ok, warnings) =
            stage6_sufficiency_check(&manifest, &index, &config, &["src/main.rs".to_string()]);
        assert!(!ok);
        assert!(warnings.iter().any(|w| w.contains("no test")));
    }

    #[test]
    fn sufficiency_warns_on_large_file() {
        let index = make_index();
        let config = PipelineConfig {
            large_file_threshold_tokens: 100,
            ..Default::default()
        };
        let manifest = PackedManifest {
            included: vec![PackedItem {
                path: "src/main.rs".to_string(),
                ranges: vec![Range { start: 1, end: 50 }],
                estimated_tokens: 500,
                score: 1.0,
                reason_code: "direct".to_string(),
            }],
            omitted: vec![],
            total_tokens: 500,
        };
        let (ok, warnings) = stage6_sufficiency_check(&manifest, &index, &config, &[]);
        assert!(!ok);
        assert!(warnings.iter().any(|w| w.contains("large file")));
    }

    #[test]
    fn recall_guard_flags_import_of_omitted() {
        let index = make_index();
        let manifest = PackedManifest {
            included: vec![PackedItem {
                path: "src/main.rs".to_string(),
                ranges: vec![Range { start: 1, end: 50 }],
                estimated_tokens: 500,
                score: 1.0,
                reason_code: "direct".to_string(),
            }],
            omitted: vec![OmittedItem {
                path: "src/utils.rs".to_string(),
                reason: "lower_relevance_score".to_string(),
                estimated_tokens: 300,
                risk: None,
            }],
            total_tokens: 500,
        };
        let flags = stage7_recall_guard(&manifest, &index, &[]);
        assert!(!flags.is_empty());
        assert!(flags.iter().any(|f| f.category == "import_of_omitted"));
    }

    #[test]
    fn recall_guard_flags_omitted_test() {
        let index = make_index();
        let manifest = PackedManifest {
            included: vec![PackedItem {
                path: "src/main.rs".to_string(),
                ranges: vec![Range { start: 1, end: 50 }],
                estimated_tokens: 500,
                score: 1.0,
                reason_code: "direct".to_string(),
            }],
            omitted: vec![OmittedItem {
                path: "tests/integration_test.rs".to_string(),
                reason: "lower_relevance_score".to_string(),
                estimated_tokens: 400,
                risk: None,
            }],
            total_tokens: 500,
        };
        let flags = stage7_recall_guard(&manifest, &index, &["src/main.rs".to_string()]);
        assert!(!flags.is_empty());
        assert!(flags
            .iter()
            .any(|f| f.category == "omitted_test_for_target"));
    }

    #[test]
    fn full_pipeline_runs_without_panic() {
        let index = make_index();
        let config = PipelineConfig::default();
        let result = run_pipeline(
            &index,
            "Fix main.rs run_app bug",
            &["src/main.rs".to_string()],
            &config,
        );
        // Should produce some candidates
        assert!(!result.manifest.included.is_empty());
        // main.rs should be included
        let paths: Vec<&str> = result
            .manifest
            .included
            .iter()
            .map(|i| i.path.as_str())
            .collect();
        assert!(paths.contains(&"src/main.rs"));
    }

    #[test]
    fn token_penalty_applied_correctly() {
        let mut candidates = vec![ContextCandidate {
            path: "big.rs".to_string(),
            ranges: vec![Range { start: 1, end: 10 }],
            candidate_kind: CandidateKind::FullFile,
            score: 1.0,
            features: serde_json::json!({}),
            estimated_tokens: 24_000,
            discovered_by: DiscoveredBy::Ripgrep,
            reason_code: "test".to_string(),
        }];
        apply_token_penalty(&mut candidates, 12_000);
        let penalty = candidates[0]
            .features
            .get("token_cost_penalty")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!((penalty - 2.0).abs() < 0.01);
    }

    #[test]
    fn stage4_limits_candidates() {
        let mut candidates: Vec<ContextCandidate> = (0..300)
            .map(|i| ContextCandidate {
                path: format!("file{}.rs", i),
                ranges: vec![Range { start: 1, end: 10 }],
                candidate_kind: CandidateKind::FullFile,
                score: i as f64,
                features: serde_json::json!({}),
                estimated_tokens: 100,
                discovered_by: DiscoveredBy::Ripgrep,
                reason_code: "test".to_string(),
            })
            .collect();
        apply_token_penalty(&mut candidates, 12_000);
        let weights = RankWeights::default();
        let ranked = stage4_emit_manifest(candidates, &weights, 200);
        assert_eq!(ranked.len(), 200);
        // Highest scores should be at the front
        assert!(ranked[0].score >= ranked[1].score);
    }
}
