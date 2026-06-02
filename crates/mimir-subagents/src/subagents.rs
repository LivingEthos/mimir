//! Read-only subagent execution.
//!
//! Deterministic local evidence is available without provider calls. Provider-backed
//! subagents currently fall back to the same read-only local evidence path.

use std::{fs, path::Path};

use camino::Utf8Path;

use crate::{analyst::FileAnalyst, EvidenceSummary, Result, SubagentDef, SubagentError};

const MAX_SEARCH_FILES: usize = 2_000;
const MAX_FILE_BYTES: usize = 256 * 1024;
const MAX_FINDINGS: usize = 20;
const MAX_RELEVANT_PATHS: usize = 20;

/// Registry of available subagents.
pub struct SubagentRegistry {
    agents: Vec<SubagentDef>,
}

impl Default for SubagentRegistry {
    fn default() -> Self {
        Self {
            agents: vec![
                SubagentDef {
                    name: "file-analyst".into(),
                    description: "Deterministic file analysis (no LLM)".into(),
                    uses_llm: false,
                    cost_tier: crate::CostTier::Free,
                    token_cap: 0,
                    allowed_tools: vec![],
                    read_only: true,
                },
                SubagentDef {
                    name: "search".into(),
                    description: "Semantic and lexical search across codebase".into(),
                    uses_llm: false,
                    cost_tier: crate::CostTier::Free,
                    token_cap: 1000,
                    allowed_tools: vec!["grep".into(), "find".into()],
                    read_only: true,
                },
                SubagentDef {
                    name: "file-analyst-llm".into(),
                    description: "LLM-powered file analysis for complex patterns".into(),
                    uses_llm: true,
                    cost_tier: crate::CostTier::Cheap,
                    token_cap: 8000,
                    allowed_tools: vec!["read_file".into()],
                    read_only: true,
                },
                SubagentDef {
                    name: "reviewer".into(),
                    description: "Code review subagent".into(),
                    uses_llm: true,
                    cost_tier: crate::CostTier::Standard,
                    token_cap: 16000,
                    allowed_tools: vec!["read_file".into(), "diff".into()],
                    read_only: true,
                },
                SubagentDef {
                    name: "test-summarizer".into(),
                    description: "Summarize test output into TestCard".into(),
                    uses_llm: true,
                    cost_tier: crate::CostTier::Cheap,
                    token_cap: 4000,
                    allowed_tools: vec!["read_file".into()],
                    read_only: true,
                },
            ],
        }
    }
}

impl SubagentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&SubagentDef> {
        self.agents.iter().find(|a| a.name == name)
    }

    pub fn list(&self) -> &[SubagentDef] {
        &self.agents
    }

    pub fn by_tier(&self, tier: crate::CostTier) -> Vec<&SubagentDef> {
        self.agents.iter().filter(|a| a.cost_tier == tier).collect()
    }
}

/// Execute a subagent by name using read-only local evidence.
pub fn execute(name: &str, query: &str, parent_run_id: Option<&str>) -> Result<EvidenceSummary> {
    execute_in(Utf8Path::new("."), name, query, parent_run_id)
}

/// Execute a subagent by name against an explicit workspace root.
pub fn execute_in(
    base: &Utf8Path,
    name: &str,
    query: &str,
    parent_run_id: Option<&str>,
) -> Result<EvidenceSummary> {
    let registry = SubagentRegistry::new();
    let def = registry
        .get(name)
        .ok_or_else(|| SubagentError::UnknownSubagent(name.to_string()))?;

    let mut evidence = if name == "file-analyst" {
        file_analysis_evidence(base, query)
    } else {
        local_search_evidence(base, def, name, query)
    };

    evidence.subagent = name.to_string();
    evidence.parent_run_id = parent_run_id.map(|s| s.to_string());
    evidence.run_id = format!("{}-{}", name, uuid::Uuid::new_v4());
    Ok(evidence)
}

fn file_analysis_evidence(base: &Utf8Path, query: &str) -> EvidenceSummary {
    let findings = FileAnalyst::analyze_dir(base);
    let mut evidence = FileAnalyst::summarize(&findings, query);
    if evidence.findings.is_empty() {
        evidence
            .findings
            .push("No common risk patterns found in local source files".to_string());
    }
    evidence
}

#[derive(Debug)]
struct SearchHit {
    path: String,
    score: usize,
    matched_terms: usize,
    lines: Vec<usize>,
}

fn local_search_evidence(
    base: &Utf8Path,
    def: &SubagentDef,
    name: &str,
    query: &str,
) -> EvidenceSummary {
    let terms = query_terms(query);
    let mut files = Vec::new();
    collect_search_files(base.as_std_path(), base.as_std_path(), &mut files);

    let mut hits = files
        .into_iter()
        .filter_map(|path| score_file(base.as_std_path(), &path, &terms))
        .collect::<Vec<_>>();

    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut findings = hits
        .iter()
        .take(MAX_FINDINGS)
        .map(|hit| {
            let line_summary = if hit.lines.is_empty() {
                "path match".to_string()
            } else {
                format!(
                    "line{} {}",
                    if hit.lines.len() == 1 { "" } else { "s" },
                    hit.lines
                        .iter()
                        .map(|line| line.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            format!(
                "Matched {} query term(s) in {} ({})",
                hit.matched_terms, hit.path, line_summary
            )
        })
        .collect::<Vec<_>>();

    if def.uses_llm {
        findings.insert(
            0,
            format!(
                "Provider-backed subagent '{}' used deterministic read-only local evidence fallback",
                name
            ),
        );
    }

    if findings.is_empty() {
        findings.push(format!(
            "No local files matched query terms: {}",
            terms.join(", ")
        ));
    }

    let relevant_paths = hits
        .iter()
        .take(MAX_RELEVANT_PATHS)
        .map(|hit| hit.path.clone())
        .collect::<Vec<_>>();
    let tokens_consumed = estimate_evidence_tokens(&findings, &relevant_paths, def.token_cap);

    EvidenceSummary {
        subagent: name.to_string(),
        query: query.to_string(),
        findings,
        relevant_paths,
        confidence: if hits.is_empty() { 0.35 } else { 0.82 },
        tokens_consumed,
        cost_usd: 0.0,
        parent_run_id: None,
        run_id: String::new(),
    }
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = query
        .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .filter_map(|term| {
            let normalized = term.trim().to_lowercase();
            (normalized.len() >= 2).then_some(normalized)
        })
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    if terms.is_empty() && !query.trim().is_empty() {
        terms.push(query.trim().to_lowercase());
    }
    terms
}

fn collect_search_files(root: &Path, dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    if files.len() >= MAX_SEARCH_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_SEARCH_FILES {
            return;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_descend(root, &path) {
                collect_search_files(root, &path, files);
            }
        } else if file_type.is_file() && is_searchable_file(&path) {
            files.push(path);
        }
    }
}

fn should_descend(root: &Path, path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    !matches!(
        name,
        ".git" | ".mimir" | ".serena" | "target" | "node_modules" | "dist"
    ) && path.starts_with(root)
}

fn is_searchable_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if matches!(name, "Cargo.lock" | "package-lock.json") {
        return false;
    }
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "rs" | "toml"
                | "md"
                | "json"
                | "yaml"
                | "yml"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "mjs"
                | "py"
                | "rb"
                | "sh"
                | "txt"
                | "go"
                | "java"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
        )
    )
}

fn score_file(root: &Path, path: &Path, terms: &[String]) -> Option<SearchHit> {
    let relative = path
        .strip_prefix(root)
        .ok()
        .and_then(|path| path.to_str())
        .unwrap_or_else(|| path.to_str().unwrap_or("<non-utf8>"))
        .trim_start_matches("./")
        .to_string();
    let relative_lower = relative.to_lowercase();
    let path_matches = terms
        .iter()
        .filter(|term| relative_lower.contains(term.as_str()))
        .count();

    let data = fs::read(path).ok()?;
    if data.len() > MAX_FILE_BYTES || data.contains(&0) {
        return None;
    }
    let content = String::from_utf8(data).ok()?;
    let content_lower = content.to_lowercase();
    let content_matches = terms
        .iter()
        .filter(|term| content_lower.contains(term.as_str()))
        .count();
    if path_matches == 0 && content_matches == 0 {
        return None;
    }

    let mut lines = Vec::new();
    for (index, line) in content_lower.lines().enumerate() {
        if terms.iter().any(|term| line.contains(term.as_str())) {
            lines.push(index + 1);
            if lines.len() == 5 {
                break;
            }
        }
    }

    Some(SearchHit {
        path: relative,
        score: path_matches * 3 + content_matches * 2 + lines.len(),
        matched_terms: path_matches.max(content_matches),
        lines,
    })
}

fn estimate_evidence_tokens(findings: &[String], paths: &[String], cap: u32) -> u32 {
    let chars = findings.iter().map(String::len).sum::<usize>()
        + paths.iter().map(String::len).sum::<usize>();
    let estimate = ((chars / 4) + 1) as u32;
    if cap == 0 {
        estimate
    } else {
        estimate.min(cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_registry_lookup() {
        let reg = SubagentRegistry::new();
        assert!(reg.get("file-analyst").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_by_tier() {
        let reg = SubagentRegistry::new();
        let free = reg.by_tier(crate::CostTier::Free);
        assert_eq!(free.len(), 2); // file-analyst, search
    }

    #[test]
    fn search_subagent_returns_real_local_matches() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/context.rs"),
            "pub fn build_context_packet() {}\n",
        )
        .unwrap();

        let base = Utf8Path::from_path(dir.path()).unwrap();
        let result = execute_in(base, "search", "where is context built", Some("run-1")).unwrap();

        assert_eq!(result.subagent, "search");
        assert_eq!(result.cost_usd, 0.0);
        assert_eq!(result.parent_run_id, Some("run-1".into()));
        assert!(result
            .relevant_paths
            .contains(&"src/context.rs".to_string()));
        assert!(!result
            .findings
            .iter()
            .any(|finding| finding.contains("Stub result")));
    }

    #[test]
    fn file_analyst_reports_patterns_with_parent_run() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("src/lib.rs"),
            "fn main() { thing.unwrap(); }\n",
        )
        .unwrap();

        let base = Utf8Path::from_path(dir.path()).unwrap();
        let result = execute_in(base, "file-analyst", "find panics", Some("run-2")).unwrap();

        assert_eq!(result.subagent, "file-analyst");
        assert_eq!(result.parent_run_id, Some("run-2".into()));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.contains("Unwrap")));
    }

    #[test]
    fn provider_backed_subagent_uses_read_only_fallback_without_cost() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("tests")).unwrap();
        fs::write(
            dir.path().join("tests/failing.rs"),
            "#[test] fn context_test() {}\n",
        )
        .unwrap();

        let base = Utf8Path::from_path(dir.path()).unwrap();
        let result = execute_in(base, "reviewer", "context test", None).unwrap();

        assert_eq!(result.cost_usd, 0.0);
        assert!(result
            .findings
            .first()
            .unwrap()
            .contains("deterministic read-only local evidence fallback"));
        assert!(result
            .relevant_paths
            .contains(&"tests/failing.rs".to_string()));
    }

    #[test]
    fn search_subagent_reports_no_local_matches_without_stub_text() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/lib.rs"), "pub fn feature() {}\n").unwrap();

        let base = Utf8Path::from_path(dir.path()).unwrap();
        let result = execute_in(base, "search", "zanzibar quokka nebula", None).unwrap();

        assert_eq!(result.confidence, 0.35);
        assert_eq!(result.cost_usd, 0.0);
        assert!(result.relevant_paths.is_empty());
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.contains("No local files matched")));
        assert!(!result
            .findings
            .iter()
            .any(|finding| finding.contains("Stub result")));
    }

    #[test]
    fn search_subagent_ignores_generated_and_internal_directories() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::create_dir_all(dir.path().join(".mimir/cache")).unwrap();
        fs::create_dir_all(dir.path().join("target/debug")).unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(dir.path().join("src/real.rs"), "pub fn search_index() {}\n").unwrap();
        fs::write(
            dir.path().join(".mimir/cache/hidden.rs"),
            "pub fn search_index() {}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("target/debug/hidden.rs"),
            "pub fn search_index() {}\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("node_modules/pkg/hidden.js"),
            "function search_index() {}\n",
        )
        .unwrap();

        let base = Utf8Path::from_path(dir.path()).unwrap();
        let result = execute_in(base, "search", "search index", None).unwrap();

        assert!(result.relevant_paths.contains(&"src/real.rs".to_string()));
        assert!(!result
            .relevant_paths
            .iter()
            .any(|path| path.contains(".mimir")
                || path.contains("target")
                || path.contains("node_modules")));
    }

    #[test]
    fn test_execute_unknown_subagent() {
        let err = execute("unknown", "q", None).unwrap_err();
        assert!(matches!(err, SubagentError::UnknownSubagent(_)));
    }
}
