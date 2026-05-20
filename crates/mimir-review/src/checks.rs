//! `.mimir/checks/*.md` source-controlled checks.
//!
//! Checks are declarative rules that can fail a packet without a model call.

use std::fs;
use std::path::Path;

use camino::Utf8Path;
use regex::Regex;

use crate::Finding;

/// A source-controlled check loaded from `.mimir/checks/*.md`.
#[derive(Debug, Clone)]
pub struct Check {
    /// Check name (from filename).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Patterns to search for (regex).
    pub patterns: Vec<String>,
    /// Forbidden patterns (if found, check fails).
    pub forbidden: Vec<String>,
    /// Required patterns (if missing, check fails).
    pub required: Vec<String>,
    /// Severity if check fails.
    pub severity: String,
}

/// Load all checks from `.mimir/checks/*.md`.
pub fn load_checks(base: &Utf8Path) -> Vec<Check> {
    let checks_dir = base.join(".mimir/checks");
    let mut checks = Vec::new();

    let entries = match fs::read_dir(&checks_dir) {
        Ok(e) => e,
        Err(_) => return checks,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(check) = parse_check(&path, &content) {
                    checks.push(check);
                }
            }
        }
    }

    checks
}

/// Parse a check markdown file.
fn parse_check(path: &Path, content: &str) -> Option<Check> {
    let name = path.file_stem()?.to_str()?.to_string();
    let mut description = String::new();
    let mut patterns = Vec::new();
    let mut forbidden = Vec::new();
    let mut required = Vec::new();
    let mut severity = "error".to_string();

    let pattern_re = Regex::new(r"(?i)^\s*[-*]\s*pattern:\s*(.+)$").ok()?;
    let forbidden_re = Regex::new(r"(?i)^\s*[-*]\s*forbidden:\s*(.+)$").ok()?;
    let required_re = Regex::new(r"(?i)^\s*[-*]\s*required:\s*(.+)$").ok()?;
    let severity_re = Regex::new(r"(?i)^\s*[-*]\s*severity:\s*(.+)$").ok()?;

    for line in content.lines() {
        if let Some(caps) = pattern_re.captures(line) {
            patterns.push(caps[1].trim().to_string());
        } else if let Some(caps) = forbidden_re.captures(line) {
            forbidden.push(caps[1].trim().to_string());
        } else if let Some(caps) = required_re.captures(line) {
            required.push(caps[1].trim().to_string());
        } else if let Some(caps) = severity_re.captures(line) {
            severity = caps[1].trim().to_string();
        } else if !line.starts_with("#") && !line.trim().is_empty() && description.is_empty() {
            description = line.trim().to_string();
        }
    }

    Some(Check {
        name,
        description,
        patterns,
        forbidden,
        required,
        severity,
    })
}

/// Run all checks against a set of files.
pub fn run_checks(checks: &[Check], base: &Utf8Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    for check in checks {
        findings.extend(run_check(check, base));
    }

    findings
}

fn run_check(check: &Check, base: &Utf8Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Check forbidden patterns
    for pattern in &check.forbidden {
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for entry in walkdir(base) {
            if let Ok(content) = fs::read_to_string(&entry) {
                for (line_num, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        findings.push(Finding {
                            category: format!("check:{}", check.name),
                            paths: vec![entry.to_string()],
                            description: format!(
                                "Forbidden pattern '{}' found: {}",
                                pattern,
                                line.trim()
                            ),
                            severity: check.severity.clone(),
                            line_numbers: Some(vec![(line_num + 1) as u32]),
                        });
                    }
                }
            }
        }
    }

    // Check required patterns
    for pattern in &check.required {
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let mut found = false;
        for entry in walkdir(base) {
            if let Ok(content) = fs::read_to_string(&entry) {
                if re.is_match(&content) {
                    found = true;
                    break;
                }
            }
        }

        if !found {
            findings.push(Finding {
                category: format!("check:{}", check.name),
                paths: vec!["*".into()],
                description: format!("Required pattern '{}' not found in any file", pattern),
                severity: check.severity.clone(),
                line_numbers: None,
            });
        }
    }

    findings
}

/// Simple file walker (non-recursive for speed, checks common source dirs).
fn walkdir(base: &Utf8Path) -> Vec<String> {
    let mut files = Vec::new();
    let dirs = ["src", "lib", "app", "tests", "crates"];
    for dir in &dirs {
        let path = base.join(dir);
        if let Ok(entries) = fs::read_dir(&path) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    if let Some(s) = p.to_str() {
                        files.push(s.to_string());
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
    fn test_parse_check() {
        let content = r"# No TODOs

Ensure no TODO comments remain in production code.

- forbidden: TODO|FIXME|XXX
- severity: error
";
        let check = parse_check(Path::new(".mimir/checks/no-todos.md"), content).unwrap();
        assert_eq!(check.name, "no-todos");
        assert_eq!(check.forbidden.len(), 1);
        assert_eq!(check.severity, "error");
    }

    #[test]
    fn test_run_check_forbidden() {
        let dir = TempDir::new().unwrap();
        let base = Utf8Path::from_path(dir.path()).unwrap();
        fs::create_dir(base.join("src")).unwrap();
        {
            let mut f = fs::File::create(base.join("src/main.rs")).unwrap();
            f.write_all(b"// TODO: fix this\nfn main() {}\n").unwrap();
        }

        let check = Check {
            name: "no-todos".into(),
            description: "No TODOs".into(),
            patterns: vec![],
            forbidden: vec![r"TODO|FIXME".into()],
            required: vec![],
            severity: "error".into(),
        };

        let findings = run_check(&check, base);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn test_run_check_required() {
        let dir = TempDir::new().unwrap();
        let base = Utf8Path::from_path(dir.path()).unwrap();

        let check = Check {
            name: "license-header".into(),
            description: "License header required".into(),
            patterns: vec![],
            forbidden: vec![],
            required: vec![r"Copyright 20\d\d".into()],
            severity: "warn".into(),
        };

        let findings = run_check(&check, base);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "warn");
    }
}
