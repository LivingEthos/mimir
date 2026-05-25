//! Deterministic file-analyst (no LLM).

use std::fs;

use camino::Utf8Path;
use regex::Regex;

use crate::EvidenceSummary;

/// A pattern to search for in source code.
#[derive(Debug, Clone)]
pub struct AnalysisPattern {
    pub name: String,
    pub regex: Regex,
    pub severity: String,
    pub description: String,
}

/// Production-quality deterministic file analyst.
pub struct FileAnalyst;

impl FileAnalyst {
    /// Analyze a single file for common issues.
    pub fn analyze_file(path: &str, content: &str) -> Vec<Finding> {
        let mut findings = Vec::new();
        let patterns = Self::default_patterns();

        for (line_num, line) in content.lines().enumerate() {
            for pattern in &patterns {
                if pattern.regex.is_match(line) {
                    findings.push(Finding {
                        line: (line_num + 1) as u32,
                        pattern: pattern.name.clone(),
                        severity: pattern.severity.clone(),
                        description: pattern.description.replace("{line}", line.trim()),
                        path: path.to_string(),
                    });
                }
            }
        }

        findings
    }

    /// Analyze a directory recursively.
    pub fn analyze_dir(base: &Utf8Path) -> Vec<Finding> {
        let mut all = Vec::new();
        for entry in walkdir(base) {
            if let Ok(content) = fs::read_to_string(&entry) {
                all.extend(Self::analyze_file(&entry, &content));
            }
        }
        all
    }

    /// Produce an EvidenceSummary from findings.
    pub fn summarize(findings: &[Finding], query: &str) -> EvidenceSummary {
        let mut paths: Vec<String> = findings.iter().map(|f| f.path.clone()).collect();
        paths.sort();
        paths.dedup();

        let bullet_points: Vec<String> = findings
            .iter()
            .take(20)
            .map(|f| format!("[{}] {}:{} — {}", f.severity, f.path, f.line, f.description))
            .collect();

        EvidenceSummary {
            subagent: "file-analyst".into(),
            query: query.into(),
            findings: bullet_points,
            relevant_paths: paths,
            confidence: if findings.is_empty() { 1.0 } else { 0.9 },
            tokens_consumed: 0,
            cost_usd: 0.0,
            parent_run_id: None,
            run_id: format!("fa-{}", uuid::Uuid::new_v4()),
        }
    }

    fn default_patterns() -> Vec<AnalysisPattern> {
        vec![
            AnalysisPattern {
                name: "unwrap".into(),
                regex: Regex::new(r"\.unwrap\(\)").unwrap(),
                severity: "warn".into(),
                description: "Unwrap may panic; consider Result handling".into(),
            },
            AnalysisPattern {
                name: "expect".into(),
                regex: Regex::new(r#"\.expect\(["\x27]"#).unwrap(),
                severity: "info".into(),
                description: "Expect with message — acceptable if invariant".into(),
            },
            AnalysisPattern {
                name: "todo".into(),
                regex: Regex::new(r"TODO|FIXME|XXX|HACK").unwrap(),
                severity: "warn".into(),
                description: "Incomplete work marker found".into(),
            },
            AnalysisPattern {
                name: "unsafe".into(),
                regex: Regex::new(r"\bunsafe\b").unwrap(),
                severity: "warn".into(),
                description: "Unsafe block requires careful review".into(),
            },
            AnalysisPattern {
                name: "panic".into(),
                regex: Regex::new(r"\bpanic!\(").unwrap(),
                severity: "error".into(),
                description: "Explicit panic found".into(),
            },
            AnalysisPattern {
                name: "println".into(),
                regex: Regex::new(r"\bprintln!\(").unwrap(),
                severity: "info".into(),
                description: "Debug print statement in source".into(),
            },
            AnalysisPattern {
                name: "unreachable".into(),
                regex: Regex::new(r"\bunreachable!\(").unwrap(),
                severity: "warn".into(),
                description: "Unreachable macro — document why".into(),
            },
        ]
    }
}

/// A single finding from deterministic analysis.
#[derive(Debug, Clone)]
pub struct Finding {
    pub line: u32,
    pub pattern: String,
    pub severity: String,
    pub description: String,
    pub path: String,
}

/// Simple file walker.
fn walkdir(base: &Utf8Path) -> Vec<String> {
    let mut files = Vec::new();
    let dirs = ["src", "lib", "app", "tests", "crates", "examples"];
    for dir in &dirs {
        let path = base.join(dir);
        if let Ok(entries) = fs::read_dir(&path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    if let Some(s) = p.to_str() {
                        files.push(s.to_string());
                    }
                } else if p.is_dir() {
                    if let Ok(sub) = fs::read_dir(&p) {
                        for se in sub.flatten() {
                            let sp = se.path();
                            if sp.is_file() {
                                if let Some(s) = sp.to_str() {
                                    files.push(s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_analyze_file_panics() {
        let content = r#"
fn main() {
    let x = some_result.unwrap();
    panic!("oh no");
    println!("debug");
}
"#;
        let findings = FileAnalyst::analyze_file("src/main.rs", content);
        assert!(findings.iter().any(|f| f.pattern == "unwrap"));
        assert!(findings.iter().any(|f| f.pattern == "panic"));
        assert!(findings.iter().any(|f| f.pattern == "println"));
    }

    #[test]
    fn test_analyze_file_todo() {
        let content = "// TODO: fix this\nfn foo() {}\n";
        let findings = FileAnalyst::analyze_file("src/lib.rs", content);
        assert!(findings.iter().any(|f| f.pattern == "todo"));
    }

    #[test]
    fn test_summarize() {
        let findings = vec![Finding {
            line: 5,
            pattern: "unwrap".into(),
            severity: "warn".into(),
            description: "Unwrap may panic".into(),
            path: "src/main.rs".into(),
        }];
        let summary = FileAnalyst::summarize(&findings, "Find panics");
        assert_eq!(summary.subagent, "file-analyst");
        assert_eq!(summary.findings.len(), 1);
        assert_eq!(summary.cost_usd, 0.0);
    }

    #[test]
    fn test_analyze_dir() {
        let dir = TempDir::new().unwrap();
        let base = Utf8Path::from_path(dir.path()).unwrap();
        fs::create_dir(base.join("src")).unwrap();
        {
            let mut f = fs::File::create(base.join("src/main.rs")).unwrap();
            f.write_all(b"fn main() { panic!(\"x\"); }\n").unwrap();
        }
        let findings = FileAnalyst::analyze_dir(base);
        assert!(findings.iter().any(|f| f.pattern == "panic"));
    }
}
