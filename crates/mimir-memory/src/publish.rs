//! Marker-block publishing into `.mimir/project-rules.md`.
//!
//! Publishes memory entries between `<!-- MIMIR_PUBLISHED_START -->` and
//! `<!-- MIMIR_PUBLISHED_END -->` markers. If the file or markers do not
//! exist, they are created.

use std::fs;
use std::path::Path;

use camino::Utf8Path;
use tracing::{info, warn};

use mimir_schemas::MemoryEntry;

use crate::MemoryError;

const START_MARKER: &str = "<!-- MIMIR_PUBLISHED_START -->";
const END_MARKER: &str = "<!-- MIMIR_PUBLISHED_END -->";

/// Publish a list of memory entries into the project-rules file.
///
/// # Errors
/// Returns `MemoryError::Io` or `MemoryError::InvalidPath` on failure.
pub fn publish<P: AsRef<Path>>(
    entries: &[MemoryEntry],
    project_rules_path: P,
) -> Result<(), MemoryError> {
    let path = project_rules_path.as_ref();
    let path = Utf8Path::from_path(path)
        .ok_or_else(|| MemoryError::InvalidPath(path.to_string_lossy().to_string()))?;

    let content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    let rendered = render_entries(entries);
    let new_content = replace_or_append_markers(&content, &rendered);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, new_content)?;

    info!(path = %path, count = entries.len(), "Published memory entries");
    Ok(())
}

fn render_entries(entries: &[MemoryEntry]) -> String {
    let mut lines = vec![START_MARKER.to_string(), String::new()];
    lines.push("# Mimir Published Rules".to_string());
    lines.push(String::new());

    for entry in entries {
        lines.push(format!("## {} (`{}`)", entry.kind, entry.entry_id));
        lines.push(String::new());
        lines.push(format!("- **Confidence**: {}", entry.confidence));
        if let Some(score) = entry.promotion_score {
            lines.push(format!("- **Promotion Score**: {:.2}", score));
        }
        lines.push(format!("- **Scope**: {}", entry.scope));
        lines.push(format!("- **Safe to Send**: {}", entry.safe_to_send));
        lines.push(format!("- **Created**: {}", entry.created_at));
        if let Some(ref verified) = entry.last_verified_at {
            lines.push(format!("- **Last Verified**: {}", verified));
        }
        if !entry.retrieval_tags.is_empty() {
            lines.push(format!("- **Tags**: {}", entry.retrieval_tags.join(", ")));
        }
        lines.push(String::new());
        lines.push(entry.body.clone());
        lines.push(String::new());
    }

    lines.push(END_MARKER.to_string());
    lines.join("\n")
}

fn replace_or_append_markers(content: &str, block: &str) -> String {
    let start = content.find(START_MARKER);
    let end = content.find(END_MARKER);

    match (start, end) {
        (Some(s), Some(e)) if s < e => {
            let before = &content[..s];
            let after = &content[e + END_MARKER.len()..];
            format!("{}{}{}", before, block, after)
        }
        _ => {
            let mut result = content.to_string();
            if !result.is_empty() && !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(block);
            result.push('\n');
            result
        }
    }
}

/// Read currently published entries from a project-rules file.
///
/// Returns the raw markdown between the markers (without markers).
///
/// # Errors
/// Returns `MemoryError::Io` or `MemoryError::InvalidPath` on failure.
pub fn read_published<P: AsRef<Path>>(
    project_rules_path: P,
) -> Result<Option<String>, MemoryError> {
    let path = project_rules_path.as_ref();
    let path = Utf8Path::from_path(path)
        .ok_or_else(|| MemoryError::InvalidPath(path.to_string_lossy().to_string()))?;

    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    let start = content.find(START_MARKER);
    let end = content.find(END_MARKER);

    match (start, end) {
        (Some(s), Some(e)) if s < e => {
            let inner = &content[s + START_MARKER.len()..e];
            let trimmed = inner.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        _ => {
            warn!("Markers not found or malformed in {}", path);
            Ok(None)
        }
    }
}

/// Clear the published block (remove everything between markers).
///
/// # Errors
/// Returns `MemoryError::Io` or `MemoryError::InvalidPath` on failure.
pub fn clear_published<P: AsRef<Path>>(project_rules_path: P) -> Result<bool, MemoryError> {
    let path = project_rules_path.as_ref();
    let path = Utf8Path::from_path(path)
        .ok_or_else(|| MemoryError::InvalidPath(path.to_string_lossy().to_string()))?;

    if !path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(path)?;
    let start = content.find(START_MARKER);
    let end = content.find(END_MARKER);

    match (start, end) {
        (Some(s), Some(e)) if s < e => {
            let before = &content[..s];
            let after = &content[e + END_MARKER.len()..];
            let mut result = String::new();
            if !before.is_empty() {
                result.push_str(before.trim_end());
                result.push('\n');
            }
            result.push_str(START_MARKER);
            result.push('\n');
            result.push_str(END_MARKER);
            if !after.is_empty() {
                result.push('\n');
                result.push_str(after.trim_start());
            }
            fs::write(path, result.trim())?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimir_schemas::{MemoryEntry, SourceEvidence};
    use tempfile::TempDir;

    fn make_entry(id: &str, body: &str) -> MemoryEntry {
        MemoryEntry {
            schema_version: 1,
            entry_id: id.to_string(),
            kind: "experience".to_string(),
            body: body.to_string(),
            source_evidence: vec![SourceEvidence {
                kind: "run".to_string(),
                ref_: "run-123".to_string(),
            }],
            confidence: "validated".to_string(),
            promotion_score: Some(0.85),
            promotion_breakdown: None,
            scope: "repo_shared".to_string(),
            safe_to_send: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            last_verified_at: None,
            staleness_policy: None,
            retrieval_tags: vec!["rust".to_string()],
            imported_from: None,
        }
    }

    #[test]
    fn test_publish_new_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project-rules.md");
        let entries = vec![make_entry("e1", "Use anyhow for errors")];
        publish(&entries, &path).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(START_MARKER));
        assert!(content.contains(END_MARKER));
        assert!(content.contains("Use anyhow for errors"));
        assert!(content.contains("validated"));
    }

    #[test]
    fn test_publish_replace_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project-rules.md");
        let initial = format!(
            "# Rules\n\n{}\nOld content\n{}\n\nFooter\n",
            START_MARKER, END_MARKER
        );
        fs::write(&path, initial).unwrap();

        let entries = vec![make_entry("e2", "New rule")];
        publish(&entries, &path).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("New rule"));
        assert!(!content.contains("Old content"));
        assert!(content.contains("Footer"));
    }

    #[test]
    fn test_read_published() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project-rules.md");
        let entries = vec![make_entry("e1", "Rule one")];
        publish(&entries, &path).unwrap();

        let published = read_published(&path).unwrap();
        assert!(published.is_some());
        let text = published.unwrap();
        assert!(text.contains("Rule one"));
    }

    #[test]
    fn test_read_published_no_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.md");
        let published = read_published(&path).unwrap();
        assert!(published.is_none());
    }

    #[test]
    fn test_clear_published() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project-rules.md");
        let entries = vec![make_entry("e1", "Rule one")];
        publish(&entries, &path).unwrap();

        assert!(clear_published(&path).unwrap());
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("Rule one"));
        assert!(content.contains(START_MARKER));
        assert!(content.contains(END_MARKER));
    }
}
