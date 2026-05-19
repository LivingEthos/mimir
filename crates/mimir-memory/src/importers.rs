//! Session importers for external AI coding tools.
//!
//! Provides a `SessionImporter` trait and implementations for:
//! - Aider (`.aider.chat.history.md`)
//! - Claude Code (`conversation.json`)
//! - Codex (`.jsonl` log files)
//! - OpenCode (`.json` log files)

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use chrono::Utc;
use serde::Deserialize;

use mimir_schemas::{ImportedFrom, MemoryEntry};

use crate::MemoryError;

/// Trait for importing sessions from external tools.
pub trait SessionImporter {
    /// Import memory entries from a session file at `path`.
    ///
    /// # Errors
    /// Returns `MemoryError` if the file cannot be read or parsed.
    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError>;
}

/// Return an importer for the given tool name, or `None` if unsupported.
#[must_use]
pub fn importer_for(tool: &str) -> Option<Box<dyn SessionImporter>> {
    match tool.to_lowercase().as_str() {
        "aider" => Some(Box::new(AiderImporter)),
        "claude-code" | "claude_code" | "claude" => Some(Box::new(ClaudeCodeImporter)),
        "codex" => Some(Box::new(CodexImporter)),
        "opencode" | "open_code" => Some(Box::new(OpenCodeImporter)),
        _ => None,
    }
}

/// Aider session importer.
///
/// Parses `.aider.chat.history.md` files which contain markdown sections
/// headed by `## USER` and `## ASSISTANT`.
pub struct AiderImporter;

impl SessionImporter for AiderImporter {
    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        let content = fs::read_to_string(path)?;
        let session_id = session_id_from_path(path);

        let mut entries = Vec::new();
        let mut current_role = String::new();
        let mut current_body = String::new();

        for line in content.lines() {
            if line.starts_with("## USER") || line.starts_with("## user") {
                if !current_body.is_empty() {
                    entries.push(build_entry(
                        &session_id,
                        "aider",
                        &current_role,
                        &current_body,
                    ));
                    current_body.clear();
                }
                current_role = "user".to_string();
            } else if line.starts_with("## ASSISTANT") || line.starts_with("## assistant") {
                if !current_body.is_empty() {
                    entries.push(build_entry(
                        &session_id,
                        "aider",
                        &current_role,
                        &current_body,
                    ));
                    current_body.clear();
                }
                current_role = "assistant".to_string();
            } else {
                current_body.push_str(line);
                current_body.push('\n');
            }
        }

        if !current_body.is_empty() && !current_role.is_empty() {
            entries.push(build_entry(
                &session_id,
                "aider",
                &current_role,
                &current_body,
            ));
        }

        Ok(entries)
    }
}

/// Claude Code session importer.
///
/// Reads `conversation.json` files produced by Claude Code.
#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    role: String,
    content: Vec<ClaudeContent>,
}

#[derive(Debug, Deserialize)]
struct ClaudeContent {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeConversation {
    messages: Vec<ClaudeMessage>,
}

/// Claude Code session importer.
pub struct ClaudeCodeImporter;

impl SessionImporter for ClaudeCodeImporter {
    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        let content = fs::read_to_string(path)?;
        let conversation: ClaudeConversation = serde_json::from_str(&content)?;
        let session_id = session_id_from_path(path);

        let mut entries = Vec::new();
        for msg in conversation.messages {
            let body = msg
                .content
                .into_iter()
                .filter_map(|c| if c.kind == "text" { c.text } else { None })
                .collect::<Vec<_>>()
                .join("\n");

            if !body.trim().is_empty() {
                entries.push(build_entry(&session_id, "claude-code", &msg.role, &body));
            }
        }

        Ok(entries)
    }
}

/// Codex session importer.
///
/// Reads `.jsonl` log files where each line is a JSON object with `role` and `content`.
#[derive(Debug, Deserialize)]
struct CodexLine {
    role: String,
    content: String,
}

/// Codex session importer.
pub struct CodexImporter;

impl SessionImporter for CodexImporter {
    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);
        let session_id = session_id_from_path(path);

        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record: CodexLine = serde_json::from_str(&line)?;
            if !record.content.trim().is_empty() {
                entries.push(build_entry(
                    &session_id,
                    "codex",
                    &record.role,
                    &record.content,
                ));
            }
        }

        Ok(entries)
    }
}

/// OpenCode session importer.
///
/// Reads `.json` log files containing an array of message objects.
#[derive(Debug, Deserialize)]
struct OpenCodeMessage {
    role: String,
    content: String,
}

/// OpenCode session importer.
pub struct OpenCodeImporter;

impl SessionImporter for OpenCodeImporter {
    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        let content = fs::read_to_string(path)?;
        let messages: Vec<OpenCodeMessage> = serde_json::from_str(&content)?;
        let session_id = session_id_from_path(path);

        let mut entries = Vec::new();
        for msg in messages {
            if !msg.content.trim().is_empty() {
                entries.push(build_entry(
                    &session_id,
                    "opencode",
                    &msg.role,
                    &msg.content,
                ));
            }
        }

        Ok(entries)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn session_id_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn build_entry(session_id: &str, tool: &str, role: &str, body: &str) -> MemoryEntry {
    let entry_id = format!("{}-{}-{}", tool, session_id, uuid::Uuid::new_v4());
    let trimmed = body.trim();
    MemoryEntry {
        schema_version: 1,
        entry_id,
        kind: "session_import".to_string(),
        body: format!("[{}]\n{}", role, trimmed),
        source_evidence: Vec::new(),
        confidence: "proposed".to_string(),
        promotion_score: None,
        promotion_breakdown: None,
        scope: "project".to_string(),
        safe_to_send: false,
        created_at: Utc::now().to_rfc3339(),
        last_verified_at: None,
        staleness_policy: None,
        retrieval_tags: vec![tool.to_string()],
        imported_from: Some(ImportedFrom {
            tool: tool.to_string(),
            session_id: session_id.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_temp_file(name: &str, contents: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        (dir, path.to_string_lossy().to_string())
    }

    #[test]
    fn test_aider_import() {
        let md = "## USER\n\nHello, can you help me refactor this?\n\n## ASSISTANT\n\nSure, let's start by extracting a function.\n\n## USER\n\nLooks good.\n";
        let (_dir, path) = make_temp_file("history.md", md);
        let importer = AiderImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].kind, "session_import");
        assert_eq!(entries[0].confidence, "proposed");
        assert_eq!(entries[0].scope, "project");
        assert!(entries[0].body.contains("[user]"));
        assert!(entries[0]
            .body
            .contains("Hello, can you help me refactor this?"));
        assert!(entries[1].body.contains("[assistant]"));
        assert!(entries[1]
            .body
            .contains("Sure, let's start by extracting a function."));
        assert_eq!(entries[0].retrieval_tags, vec!["aider"]);
        assert!(entries[0].imported_from.is_some());
        let im = entries[0].imported_from.as_ref().unwrap();
        assert_eq!(im.tool, "aider");
    }

    #[test]
    fn test_claude_code_import() {
        let json = r#"{"messages":[{"role":"user","content":[{"type":"text","text":"Add a login feature"}]},{"role":"assistant","content":[{"type":"text","text":"I'll create the auth module."}]}]}"#;
        let (_dir, path) = make_temp_file("conversation.json", json);
        let importer = ClaudeCodeImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, "session_import");
        assert!(entries[0].body.contains("[user]"));
        assert!(entries[0].body.contains("Add a login feature"));
        assert!(entries[1].body.contains("[assistant]"));
        assert!(entries[1].body.contains("I'll create the auth module."));
        assert_eq!(entries[0].retrieval_tags, vec!["claude-code"]);
        let im = entries[0].imported_from.as_ref().unwrap();
        assert_eq!(im.tool, "claude-code");
    }

    #[test]
    fn test_codex_import() {
        let jsonl = "{\"role\":\"user\",\"content\":\"Write a test for the parser\"}\n{\"role\":\"assistant\",\"content\":\"Here's a unit test.\"}\n";
        let (_dir, path) = make_temp_file("session.jsonl", jsonl);
        let importer = CodexImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, "session_import");
        assert!(entries[0].body.contains("[user]"));
        assert!(entries[0].body.contains("Write a test for the parser"));
        assert!(entries[1].body.contains("[assistant]"));
        assert!(entries[1].body.contains("Here's a unit test."));
        assert_eq!(entries[0].retrieval_tags, vec!["codex"]);
        let im = entries[0].imported_from.as_ref().unwrap();
        assert_eq!(im.tool, "codex");
    }

    #[test]
    fn test_opencode_import() {
        let json = r#"[{"role":"user","content":"Fix the memory leak"},{"role":"assistant","content":"Use Arc instead of Rc."}]"#;
        let (_dir, path) = make_temp_file("log.json", json);
        let importer = OpenCodeImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, "session_import");
        assert!(entries[0].body.contains("[user]"));
        assert!(entries[0].body.contains("Fix the memory leak"));
        assert!(entries[1].body.contains("[assistant]"));
        assert!(entries[1].body.contains("Use Arc instead of Rc."));
        assert_eq!(entries[0].retrieval_tags, vec!["opencode"]);
        let im = entries[0].imported_from.as_ref().unwrap();
        assert_eq!(im.tool, "opencode");
    }

    #[test]
    fn test_importer_factory() {
        assert!(importer_for("aider").is_some());
        assert!(importer_for("claude-code").is_some());
        assert!(importer_for("claude_code").is_some());
        assert!(importer_for("claude").is_some());
        assert!(importer_for("codex").is_some());
        assert!(importer_for("opencode").is_some());
        assert!(importer_for("open_code").is_some());
        assert!(importer_for("unknown").is_none());
    }

    #[test]
    fn test_empty_aider_file() {
        let (_dir, path) = make_temp_file("empty.md", "");
        let importer = AiderImporter;
        let entries = importer.import(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_empty_codex_file() {
        let (_dir, path) = make_temp_file("empty.jsonl", "");
        let importer = CodexImporter;
        let entries = importer.import(&path).unwrap();
        assert!(entries.is_empty());
    }
}
