//! Diff review: uninspected-diff detector, generated-file edit detector.

use std::collections::HashSet;
use std::process::Command;

use camino::Utf8Path;
use regex::Regex;

use crate::{Finding, Result, ReviewError};

/// Diff metadata for a single file.
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub is_new: bool,
    pub is_deleted: bool,
    pub is_generated: bool,
    pub additions: u32,
    pub deletions: u32,
    pub hunks: Vec<DiffHunk>,
}

/// A single hunk in a diff.
#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<String>,
}

/// Collect diff since a git ref.
pub fn diff_since(base: &Utf8Path, since_ref: &str) -> Result<Vec<FileDiff>> {
    let output = Command::new("git")
        .args(["diff", since_ref, "--"])
        .current_dir(base.as_std_path())
        .output()
        .map_err(|e| ReviewError::GitError(format!("git diff failed: {}", e)))?;

    if !output.status.success() {
        return Err(ReviewError::GitError(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let diff_text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_diff(&diff_text, base))
}

/// Parse unified diff text into structured FileDiffs.
pub fn parse_diff(diff_text: &str, base: &Utf8Path) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut current_file: Option<FileDiff> = None;
    let mut current_hunk: Option<DiffHunk> = None;

    let file_header_re = Regex::new(r"^diff --git a/(.+) b/(.+)$").unwrap();
    let hunk_re = Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap();

    for line in diff_text.lines() {
        if let Some(caps) = file_header_re.captures(line) {
            if let Some(hunk) = current_hunk.take() {
                if let Some(ref mut f) = current_file {
                    f.hunks.push(hunk);
                }
            }
            if let Some(f) = current_file.take() {
                files.push(f);
            }
            let path = caps[2].to_string();
            current_file = Some(FileDiff {
                path: path.clone(),
                is_new: false,
                is_deleted: false,
                is_generated: is_generated_file(&path, base),
                additions: 0,
                deletions: 0,
                hunks: Vec::new(),
            });
        } else if let Some(caps) = hunk_re.captures(line) {
            if let Some(hunk) = current_hunk.take() {
                if let Some(ref mut f) = current_file {
                    f.hunks.push(hunk);
                }
            }
            let old_start = caps[1].parse::<u32>().unwrap_or(0);
            let old_lines = caps
                .get(2)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            let new_start = caps[3].parse::<u32>().unwrap_or(0);
            let new_lines = caps
                .get(4)
                .and_then(|m| m.as_str().parse::<u32>().ok())
                .unwrap_or(0);
            current_hunk = Some(DiffHunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines: Vec::new(),
            });
        } else if line.starts_with("new file mode") {
            if let Some(ref mut f) = current_file {
                f.is_new = true;
            }
        } else if line.starts_with("deleted file mode") {
            if let Some(ref mut f) = current_file {
                f.is_deleted = true;
            }
        } else if let Some(ref mut h) = current_hunk {
            if line.starts_with('+') && !line.starts_with("+++") {
                h.lines.push(line.to_string());
                if let Some(ref mut f) = current_file {
                    f.additions += 1;
                }
            } else if line.starts_with('-') && !line.starts_with("---") {
                h.lines.push(line.to_string());
                if let Some(ref mut f) = current_file {
                    f.deletions += 1;
                }
            } else {
                h.lines.push(line.to_string());
            }
        }
    }

    if let Some(hunk) = current_hunk {
        if let Some(ref mut f) = current_file {
            f.hunks.push(hunk);
        }
    }
    if let Some(f) = current_file {
        files.push(f);
    }

    files
}

/// Detect if a file is generated (should not be manually edited).
fn is_generated_file(path: &str, base: &Utf8Path) -> bool {
    let generated_patterns = [
        "Cargo.lock",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "poetry.lock",
        "go.sum",
        "Gemfile.lock",
        ".terraform/",
        "generated/",
        "gen/",
        "dist/",
        "build/",
        "target/",
        ".docusaurus/",
        ".next/",
        "__pycache__/",
    ];

    let full_path = base.join(path);
    if let Ok(content) = std::fs::read_to_string(&full_path) {
        if content.contains("Generated by")
            || content.contains("DO NOT EDIT")
            || content.contains("Auto-generated")
        {
            return true;
        }
    }

    generated_patterns.iter().any(|p| path.contains(p))
}

/// Detect uninspected files (files changed but not in the reviewer's view).
pub fn detect_uninspected(diffs: &[FileDiff], inspected_paths: &HashSet<String>) -> Vec<Finding> {
    let mut findings = Vec::new();
    for diff in diffs {
        if !inspected_paths.contains(&diff.path) {
            findings.push(Finding {
                category: "uninspected-diff".into(),
                paths: vec![diff.path.clone()],
                description: format!(
                    "File has {} additions/{} deletions but was not inspected",
                    diff.additions, diff.deletions
                ),
                severity: "warn".into(),
                line_numbers: None,
            });
        }
    }
    findings
}

/// Detect edits to generated files.
pub fn detect_generated_edits(diffs: &[FileDiff]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for diff in diffs {
        if diff.is_generated && (diff.additions > 0 || diff.deletions > 0) {
            findings.push(Finding {
                category: "generated-file-edit".into(),
                paths: vec![diff.path.clone()],
                description: format!(
                    "Generated file was edited ({}+ {}-). Regenerate instead.",
                    diff.additions, diff.deletions
                ),
                severity: "error".into(),
                line_numbers: None,
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_diff() {
        let diff = r"diff --git a/src/main.rs b/src/main.rs
index abc..def 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
 line1
-line2
+line2_modified
 line3
";
        let dir = TempDir::new().unwrap();
        let base = Utf8Path::from_path(dir.path()).unwrap();
        let files = parse_diff(diff, base);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[0].additions, 1);
        assert_eq!(files[0].deletions, 1);
        assert_eq!(files[0].hunks.len(), 1);
    }

    #[test]
    fn test_detect_generated_edits() {
        let diffs = vec![FileDiff {
            path: "Cargo.lock".into(),
            is_new: false,
            is_deleted: false,
            is_generated: true,
            additions: 5,
            deletions: 2,
            hunks: vec![],
        }];
        let findings = detect_generated_edits(&diffs);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn test_detect_uninspected() {
        let diffs = vec![FileDiff {
            path: "src/main.rs".into(),
            is_new: false,
            is_deleted: false,
            is_generated: false,
            additions: 3,
            deletions: 0,
            hunks: vec![],
        }];
        let inspected: HashSet<String> = ["src/other.rs".into()].into();
        let findings = detect_uninspected(&diffs, &inspected);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, "uninspected-diff");
    }
}
