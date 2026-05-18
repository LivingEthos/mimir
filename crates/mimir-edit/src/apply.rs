//! Patch application engine.

use std::fs;

use camino::Utf8Path;
use mimir_schemas::PatchStep;

use crate::{EditError, Result};

/// Engine for applying patches.
pub struct PatchEngine;

impl PatchEngine {
    /// Apply a single patch step.
    pub fn apply(step: &PatchStep, base_path: &Utf8Path) -> Result<()> {
        match step {
            PatchStep::LineRange { path, start_line, end_line, content } => {
                Self::apply_line_range(base_path, path, *start_line, *end_line, content)
            }
            PatchStep::UnifiedDiff { path, diff } => {
                Self::apply_unified_diff(base_path, path, diff)
            }
            PatchStep::WholeFile { path, content } => {
                Self::apply_whole_file(base_path, path, content)
            }
            PatchStep::Create { path, content } => Self::apply_create(base_path, path, content),
            PatchStep::Delete { path } => Self::apply_delete(base_path, path),
            PatchStep::Move { from, to } => Self::apply_move(base_path, from, to),
        }
    }

    fn apply_line_range(
        base: &Utf8Path,
        path: &str,
        start_line: usize,
        end_line: usize,
        content: &str,
    ) -> Result<()> {
        let full_path = base.join(path);
        let file_content = fs::read_to_string(&full_path)
            .map_err(|e| EditError::Io(format!("read {}: {}", path, e)))?;
        let lines: Vec<&str> = file_content.lines().collect();

        if start_line == 0 || end_line < start_line || end_line > lines.len() {
            return Err(EditError::InvalidLineRange {
                path: path.to_string(),
                start: start_line,
                end: end_line,
                file_lines: lines.len(),
            });
        }

        let mut result = String::new();
        for line in lines.iter().take(start_line - 1) {
            result.push_str(line);
            result.push('\n');
        }
        result.push_str(content);
        if !content.ends_with('\n') {
            result.push('\n');
        }
        for line in lines.iter().skip(end_line) {
            result.push_str(line);
            result.push('\n');
        }

        fs::write(&full_path, result)
            .map_err(|e| EditError::Io(format!("write {}: {}", path, e)))?;
        Ok(())
    }

    fn apply_unified_diff(base: &Utf8Path, path: &str, diff: &str) -> Result<()> {
        let full_path = base.join(path);
        let original = fs::read_to_string(&full_path)
            .map_err(|e| EditError::Io(format!("read {}: {}", path, e)))?;

        let patched = apply_unified_diff_text(&original, diff).map_err(|reason| {
            EditError::DiffApplyFailed { path: path.to_string(), reason }
        })?;

        fs::write(&full_path, patched)
            .map_err(|e| EditError::Io(format!("write {}: {}", path, e)))?;
        Ok(())
    }

    fn apply_whole_file(base: &Utf8Path, path: &str, content: &str) -> Result<()> {
        let full_path = base.join(path);
        fs::write(&full_path, content)
            .map_err(|e| EditError::Io(format!("write {}: {}", path, e)))?;
        Ok(())
    }

    fn apply_create(base: &Utf8Path, path: &str, content: &str) -> Result<()> {
        let full_path = base.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| EditError::Io(format!("mkdir {}: {}", parent, e)))?;
        }
        fs::write(&full_path, content)
            .map_err(|e| EditError::Io(format!("write {}: {}", path, e)))?;
        Ok(())
    }

    fn apply_delete(base: &Utf8Path, path: &str) -> Result<()> {
        let full_path = base.join(path);
        fs::remove_file(&full_path)
            .map_err(|e| EditError::Io(format!("delete {}: {}", path, e)))?;
        Ok(())
    }

    fn apply_move(base: &Utf8Path, from: &str, to: &str) -> Result<()> {
        let from_path = base.join(from);
        let to_path = base.join(to);
        if let Some(parent) = to_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| EditError::Io(format!("mkdir {}: {}", parent, e)))?;
        }
        fs::rename(&from_path, &to_path)
            .map_err(|e| EditError::Io(format!("move {} -> {}: {}", from, to, e)))?;
        Ok(())
    }
}

/// Apply a unified diff to original text (simplified implementation).
fn apply_unified_diff_text(original: &str, diff: &str) -> std::result::Result<String, String> {
    let mut result_lines: Vec<String> = original.lines().map(String::from).collect();
    let mut line_idx = 0usize;
    let mut in_hunk = false;

    for line in diff.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            if let Some(start_pos) = line.find('+') {
                let after_plus = &line[start_pos + 1..];
                if let Some(comma_pos) = after_plus.find(',') {
                    let start_str = &after_plus[..comma_pos];
                    if let Ok(start) = start_str.parse::<i64>() {
                        line_idx = (start - 1).max(0) as usize;
                    }
                } else if let Some(end_pos) = after_plus.find(' ') {
                    let start_str = &after_plus[..end_pos];
                    if let Ok(start) = start_str.parse::<i64>() {
                        line_idx = (start - 1).max(0) as usize;
                    }
                }
            }
        } else if in_hunk {
            if line.starts_with('-') && !line.starts_with("---") {
                if line_idx < result_lines.len() {
                    result_lines.remove(line_idx);
                }
            } else if line.starts_with('+') && !line.starts_with("+++") {
                let content = &line[1..];
                result_lines.insert(line_idx, content.to_string());
                line_idx += 1;
            } else if !line.starts_with("---") && !line.starts_with("+++") {
                // Context line — strip leading space if present
                let content = if line.starts_with(' ') { &line[1..] } else { line };
                if line_idx < result_lines.len() && result_lines[line_idx] == content {
                    line_idx += 1;
                } else if !content.trim().is_empty() {
                    return Err(format!(
                        "context mismatch at line {}: expected '{}', got '{}'",
                        line_idx + 1,
                        content,
                        result_lines.get(line_idx).map_or("<eof>", |s| s.as_str())
                    ));
                }
            }
        }
    }

    Ok(result_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, path: &str, content: &str) {
        let full = dir.path().join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(full).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn read_file(dir: &TempDir, path: &str) -> String {
        fs::read_to_string(dir.path().join(path)).unwrap()
    }

    #[test]
    fn test_apply_line_range() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\nline3\nline4\nline5\n");

        let step = PatchStep::LineRange {
            path: "test.txt".into(),
            start_line: 2,
            end_line: 3,
            content: "replaced\n".into(),
        };

        PatchEngine::apply(&step, &Utf8Path::from_path(dir.path()).unwrap()).unwrap();
        let result = read_file(&dir, "test.txt");
        assert_eq!(result, "line1\nreplaced\nline4\nline5\n");
    }

    #[test]
    fn test_apply_whole_file() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "old content");

        let step = PatchStep::WholeFile {
            path: "test.txt".into(),
            content: "new content".into(),
        };

        PatchEngine::apply(&step, &Utf8Path::from_path(dir.path()).unwrap()).unwrap();
        assert_eq!(read_file(&dir, "test.txt"), "new content");
    }

    #[test]
    fn test_apply_create() {
        let dir = TempDir::new().unwrap();
        let step = PatchStep::Create {
            path: "nested/file.txt".into(),
            content: "hello".into(),
        };

        PatchEngine::apply(&step, &Utf8Path::from_path(dir.path()).unwrap()).unwrap();
        assert_eq!(read_file(&dir, "nested/file.txt"), "hello");
    }

    #[test]
    fn test_apply_delete() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "content");

        let step = PatchStep::Delete {
            path: "test.txt".into(),
        };

        PatchEngine::apply(&step, &Utf8Path::from_path(dir.path()).unwrap()).unwrap();
        assert!(!dir.path().join("test.txt").exists());
    }

    #[test]
    fn test_apply_move() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "old.txt", "content");

        let step = PatchStep::Move {
            from: "old.txt".into(),
            to: "new.txt".into(),
        };

        PatchEngine::apply(&step, &Utf8Path::from_path(dir.path()).unwrap()).unwrap();
        assert!(!dir.path().join("old.txt").exists());
        assert_eq!(read_file(&dir, "new.txt"), "content");
    }

    #[test]
    fn test_apply_unified_diff() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\nline3\n");

        let diff = "--- a/test.txt\n+++ b/test.txt\n@@ -1,3 +1,3 @@\n line1\n-line2\n+line2_modified\n line3\n";

        let step = PatchStep::UnifiedDiff {
            path: "test.txt".into(),
            diff: diff.into(),
        };

        PatchEngine::apply(
            &step, &Utf8Path::from_path(dir.path()).unwrap()
        ).unwrap();
        let result = read_file(&dir, "test.txt");
        assert_eq!(result, "line1\nline2_modified\nline3");
    }

    #[test]
    fn test_invalid_line_range() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "test.txt", "line1\nline2\n");

        let step = PatchStep::LineRange {
            path: "test.txt".into(),
            start_line: 5,
            end_line: 10,
            content: "x".into(),
        };

        let err = PatchEngine::apply(
            &step, &Utf8Path::from_path(dir.path()).unwrap()
        ).unwrap_err();
        assert!(matches!(err, EditError::InvalidLineRange { .. }));
    }
}
