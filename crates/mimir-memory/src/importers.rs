//! Session importers for external AI coding tools.
//!
//! Provides a `SessionImporter` trait and implementations for:
//! - Aider (`.aider.chat.history.md`)
//! - Claude Code (`conversation.json`)
//! - Codex (`.jsonl` log files)
//! - OpenCode (`.json` log files)

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::{env, fs};

use chrono::Utc;
use serde::Deserialize;

use mimir_schemas::{ImportedFrom, MemoryEntry, SourceEvidence};

use crate::MemoryError;

/// Trait for importing sessions from external tools.
pub trait SessionImporter {
    /// Import memory entries from a session file at `path`.
    ///
    /// # Errors
    /// Returns `MemoryError` if the file cannot be read or parsed.
    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError>;

    /// Discover default session files for this importer.
    ///
    /// # Errors
    /// Returns `MemoryError` if a configured discovery root cannot be read.
    fn discover(&self, roots: &DiscoveryRoots) -> Result<Vec<PathBuf>, MemoryError> {
        let _ = roots;
        Ok(Vec::new())
    }
}

/// Roots used for safe default session discovery.
#[derive(Debug, Clone)]
pub struct DiscoveryRoots {
    /// Current project directory.
    pub cwd: PathBuf,
    /// User home directory.
    pub home: Option<PathBuf>,
    /// XDG data home directory.
    pub xdg_data_home: Option<PathBuf>,
    /// Codex home directory.
    pub codex_home: Option<PathBuf>,
    /// Maximum files to return per discovery run.
    pub max_files: usize,
    /// Include archived locations when a tool supports them.
    pub include_archived: bool,
}

impl DiscoveryRoots {
    /// Build discovery roots from the current process environment.
    #[must_use]
    pub fn from_env() -> Self {
        let home = env::var_os("HOME").map(PathBuf::from);
        let xdg_data_home = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
        let codex_home = env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|home| home.join(".codex")));

        Self {
            cwd: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            home,
            xdg_data_home,
            codex_home,
            max_files: 128,
            include_archived: false,
        }
    }
}

/// A discovered session path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSession {
    /// Source tool.
    pub tool: String,
    /// Session path.
    pub path: PathBuf,
}

/// Discover default session files for a supported tool.
///
/// # Errors
/// Returns `MemoryError` if a configured discovery root cannot be read.
pub fn discover_sessions(
    tool: &str,
    roots: &DiscoveryRoots,
) -> Result<Vec<DiscoveredSession>, MemoryError> {
    let importer = importer_for(tool).ok_or_else(|| MemoryError::InvalidPath(tool.to_string()))?;
    let canonical_tool = canonical_tool_name(tool)
        .ok_or_else(|| MemoryError::InvalidPath(tool.to_string()))?
        .to_string();
    importer.discover(roots).map(|paths| {
        paths
            .into_iter()
            .map(|path| DiscoveredSession {
                tool: canonical_tool.clone(),
                path,
            })
            .collect()
    })
}

/// Return an importer for the given tool name, or `None` if unsupported.
#[must_use]
pub fn importer_for(tool: &str) -> Option<Box<dyn SessionImporter>> {
    match canonical_tool_name(tool)? {
        "aider" => Some(Box::new(AiderImporter)),
        "claude-code" => Some(Box::new(ClaudeCodeImporter)),
        "codex" => Some(Box::new(CodexImporter)),
        "opencode" => Some(Box::new(OpenCodeImporter)),
        _ => None,
    }
}

fn canonical_tool_name(tool: &str) -> Option<&'static str> {
    match tool.to_lowercase().as_str() {
        "aider" => Some("aider"),
        "claude-code" | "claude_code" | "claude" => Some("claude-code"),
        "codex" => Some("codex"),
        "opencode" | "open_code" => Some("opencode"),
        _ => None,
    }
}

/// Aider session importer.
///
/// Parses `.aider.chat.history.md` files which contain markdown sections
/// headed by `## USER` and `## ASSISTANT`.
pub struct AiderImporter;

impl SessionImporter for AiderImporter {
    fn discover(&self, roots: &DiscoveryRoots) -> Result<Vec<PathBuf>, MemoryError> {
        let path = roots.cwd.join(".aider.chat.history.md");
        Ok(path.exists().then_some(path).into_iter().collect())
    }

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
    fn discover(&self, roots: &DiscoveryRoots) -> Result<Vec<PathBuf>, MemoryError> {
        let Some(home) = &roots.home else {
            return Ok(Vec::new());
        };
        let projects = home.join(".claude/projects");
        discover_files(&[projects], roots.max_files, |path| {
            path.extension().is_some_and(|ext| ext == "jsonl")
                && path.file_name().is_some_and(|name| name != "history.jsonl")
        })
    }

    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        let content = fs::read_to_string(path)?;
        let session_id = session_id_from_path(path);

        if content.trim_start().starts_with('{') {
            if let Ok(conversation) = serde_json::from_str::<ClaudeConversation>(&content) {
                return Ok(claude_conversation_entries(conversation, &session_id));
            }
        }

        let mut entries = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value = serde_json::from_str(line)?;
            if let Some((role, body)) = claude_message_from_value(&value) {
                if !body.trim().is_empty() {
                    entries.push(build_entry(&session_id, "claude-code", &role, &body));
                }
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
    fn discover(&self, roots: &DiscoveryRoots) -> Result<Vec<PathBuf>, MemoryError> {
        let Some(codex_home) = &roots.codex_home else {
            return Ok(Vec::new());
        };
        let mut roots_to_scan = vec![codex_home.join("sessions")];
        if roots.include_archived {
            roots_to_scan.push(codex_home.join("archived_sessions"));
        }

        discover_files(&roots_to_scan, roots.max_files, |path| {
            path.extension().is_some_and(|ext| ext == "jsonl")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("rollout-") || name.ends_with(".jsonl"))
        })
    }

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
            let value: serde_json::Value = serde_json::from_str(&line)?;
            if let Some((role, content)) = codex_message_from_value(&value) {
                if !content.trim().is_empty() {
                    entries.push(build_entry(&session_id, "codex", &role, &content));
                }
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
    fn discover(&self, roots: &DiscoveryRoots) -> Result<Vec<PathBuf>, MemoryError> {
        let Some(xdg_data_home) = &roots.xdg_data_home else {
            return Ok(Vec::new());
        };
        let opencode = xdg_data_home.join("opencode");
        let db = opencode.join("opencode.db");
        let mut found = db.exists().then_some(db).into_iter().collect::<Vec<_>>();
        let remaining = roots.max_files.saturating_sub(found.len());
        if remaining > 0 {
            found.extend(discover_files(
                &[opencode.join("storage")],
                remaining,
                |path| {
                    path.extension().is_some_and(|ext| ext == "json")
                        || path.extension().is_some_and(|ext| ext == "jsonl")
                },
            )?);
        }
        Ok(found)
    }

    fn import(&self, path: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
        if Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "opencode.db")
        {
            return import_opencode_db(path);
        }

        let content = fs::read_to_string(path)?;
        let session_id = session_id_from_path(path);

        let mut entries = Vec::new();
        for msg in opencode_messages_from_json(&content)? {
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
    let trimmed = body.trim();
    let evidence_hash = stable_hash(&format!("{tool}\n{session_id}\n{role}\n{trimmed}"));
    let entry_id = format!("session-{tool}-{session_id}-{}", &evidence_hash[..16]);
    MemoryEntry {
        schema_version: 1,
        entry_id,
        kind: "experience".to_string(),
        body: format!("[{}]\n{}", role, trimmed),
        source_evidence: vec![SourceEvidence {
            kind: "file_hash".to_string(),
            ref_: format!("session:{tool}:{session_id}:sha256:{evidence_hash}"),
        }],
        confidence: "provisional".to_string(),
        promotion_score: None,
        promotion_breakdown: None,
        scope: "private".to_string(),
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

fn stable_hash(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes());
    digest.to_hex().to_string()
}

fn discover_files<F>(
    roots: &[PathBuf],
    max_files: usize,
    mut include: F,
) -> Result<Vec<PathBuf>, MemoryError>
where
    F: FnMut(&Path) -> bool,
{
    let mut found = Vec::new();
    let mut stack = roots
        .iter()
        .filter(|root| root.exists())
        .cloned()
        .collect::<Vec<_>>();
    stack.sort();

    while let Some(path) = stack.pop() {
        if found.len() >= max_files {
            break;
        }

        if path.is_file() {
            if include(&path) {
                found.push(path);
            }
            continue;
        }

        if path.is_dir() {
            let mut entries = fs::read_dir(&path)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<Result<Vec<_>, _>>()?;
            entries.sort();
            for entry in entries.into_iter().rev() {
                stack.push(entry);
            }
        }
    }

    found.sort();
    found.truncate(max_files);
    Ok(found)
}

fn claude_conversation_entries(
    conversation: ClaudeConversation,
    session_id: &str,
) -> Vec<MemoryEntry> {
    let mut entries = Vec::new();
    for msg in conversation.messages {
        let body = msg
            .content
            .into_iter()
            .filter_map(|c| if c.kind == "text" { c.text } else { None })
            .collect::<Vec<_>>()
            .join("\n");

        if !body.trim().is_empty() {
            entries.push(build_entry(session_id, "claude-code", &msg.role, &body));
        }
    }
    entries
}

fn claude_message_from_value(value: &serde_json::Value) -> Option<(String, String)> {
    let message = value.get("message").unwrap_or(value);
    let role = message.get("role")?.as_str()?.to_string();
    let body = value_text(message.get("content")?)?;
    Some((role, body))
}

fn codex_message_from_value(value: &serde_json::Value) -> Option<(String, String)> {
    if let Ok(line) = serde_json::from_value::<CodexLine>(value.clone()) {
        return Some((line.role, line.content));
    }

    let payload = value
        .get("payload")
        .or_else(|| value.get("item"))
        .unwrap_or(value);
    let message_type = payload.get("type").and_then(|value| value.as_str());
    if !matches!(
        message_type,
        Some("message") | Some("input_text") | Some("output_text")
    ) {
        return None;
    }

    let role = payload
        .get("role")
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| {
            if message_type == Some("input_text") {
                "user"
            } else {
                "assistant"
            }
        })
        .to_string();
    let body = value_text(payload.get("content").or_else(|| payload.get("text"))?)?;
    Some((role, body))
}

fn value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(items) => {
            let parts = items
                .iter()
                .filter_map(|item| {
                    if let Some(text) = item.as_str() {
                        return Some(text.to_string());
                    }
                    for key in ["text", "input_text", "output_text"] {
                        if let Some(text) = item.get(key).and_then(|value| value.as_str()) {
                            return Some(text.to_string());
                        }
                    }
                    None
                })
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        serde_json::Value::Object(_) => ["text", "input_text", "output_text"]
            .iter()
            .find_map(|key| value.get(key).and_then(|value| value.as_str()))
            .map(ToString::to_string),
        _ => None,
    }
}

fn opencode_messages_from_json(content: &str) -> Result<Vec<OpenCodeMessage>, MemoryError> {
    if let Ok(messages) = serde_json::from_str::<Vec<OpenCodeMessage>>(content) {
        return Ok(messages);
    }

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(messages) = value.get("messages").and_then(|value| value.as_array()) {
            return Ok(messages
                .iter()
                .filter_map(opencode_message_from_value)
                .collect());
        }
    }

    let mut messages = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line)?;
        if let Some(message) = opencode_message_from_value(&value) {
            messages.push(message);
        }
    }
    Ok(messages)
}

fn opencode_message_from_value(value: &serde_json::Value) -> Option<OpenCodeMessage> {
    let role = value.get("role")?.as_str()?.to_string();
    let content = value_text(value.get("content").or_else(|| value.get("text"))?)?;
    Some(OpenCodeMessage { role, content })
}

fn import_opencode_db(path: &str) -> Result<Vec<MemoryEntry>, MemoryError> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let session_id = session_id_from_path(path);
    let mut entries = Vec::new();

    let queries = [
        "SELECT message.role, part.text FROM message JOIN part ON part.message_id = message.id WHERE part.text IS NOT NULL ORDER BY message.id, part.id",
        "SELECT messages.role, parts.text FROM messages JOIN parts ON parts.message_id = messages.id WHERE parts.text IS NOT NULL ORDER BY messages.id, parts.id",
    ];

    for query in queries {
        let mut statement = match conn.prepare(query) {
            Ok(statement) => statement,
            Err(_) => continue,
        };
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (role, body) = row?;
            if !body.trim().is_empty() {
                entries.push(build_entry(&session_id, "opencode", &role, &body));
            }
        }
        return Ok(entries);
    }

    Err(MemoryError::InvalidPath(
        "unsupported opencode.db schema".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn assert_schema_valid(entry: &MemoryEntry) {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/MemoryEntry.schema.json")).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let value = serde_json::to_value(entry).unwrap();
        let errors = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "memory entry schema errors: {errors:#?}");
    }

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
        assert_schema_valid(&entries[0]);
        assert_eq!(entries[0].kind, "experience");
        assert_eq!(entries[0].confidence, "provisional");
        assert_eq!(entries[0].scope, "private");
        assert!(!entries[0].safe_to_send);
        assert_eq!(entries[0].source_evidence.len(), 1);
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
        assert_schema_valid(&entries[0]);
        assert_eq!(entries[0].kind, "experience");
        assert!(entries[0].body.contains("[user]"));
        assert!(entries[0].body.contains("Add a login feature"));
        assert!(entries[1].body.contains("[assistant]"));
        assert!(entries[1].body.contains("I'll create the auth module."));
        assert_eq!(entries[0].retrieval_tags, vec!["claude-code"]);
        let im = entries[0].imported_from.as_ref().unwrap();
        assert_eq!(im.tool, "claude-code");
    }

    #[test]
    fn test_claude_code_native_jsonl_import() {
        let jsonl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Explain the router"}]}}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"The router lives in src/router.rs."}]}}
"#;
        let (_dir, path) = make_temp_file("session.jsonl", jsonl);
        let importer = ClaudeCodeImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_schema_valid(&entries[0]);
        assert!(entries[0].body.contains("Explain the router"));
        assert!(entries[1].body.contains("src/router.rs"));
    }

    #[test]
    fn test_codex_import() {
        let jsonl = "{\"role\":\"user\",\"content\":\"Write a test for the parser\"}\n{\"role\":\"assistant\",\"content\":\"Here's a unit test.\"}\n";
        let (_dir, path) = make_temp_file("session.jsonl", jsonl);
        let importer = CodexImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_schema_valid(&entries[0]);
        assert_eq!(entries[0].kind, "experience");
        assert!(entries[0].body.contains("[user]"));
        assert!(entries[0].body.contains("Write a test for the parser"));
        assert!(entries[1].body.contains("[assistant]"));
        assert!(entries[1].body.contains("Here's a unit test."));
        assert_eq!(entries[0].retrieval_tags, vec!["codex"]);
        let im = entries[0].imported_from.as_ref().unwrap();
        assert_eq!(im.tool, "codex");
    }

    #[test]
    fn test_codex_native_rollout_jsonl_import() {
        let jsonl = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Find the failing test"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"The failing test is parser_roundtrip."}]}}
"#;
        let (_dir, path) = make_temp_file("rollout-2026-05-20.jsonl", jsonl);
        let importer = CodexImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_schema_valid(&entries[0]);
        assert!(entries[0].body.contains("Find the failing test"));
        assert!(entries[1].body.contains("parser_roundtrip"));
    }

    /// R-14 memory-pollution guard.
    ///
    /// Imported sessions must NEVER be eligible to reach a provider. This test
    /// drives synthetic sessions through real importers (Aider markdown and
    /// Codex JSONL — the same fixtures the importer tests use) and asserts:
    ///
    /// 1. EVERY imported entry is quarantined: `safe_to_send == false`,
    ///    `scope == "private"`, `confidence == "provisional"`. If any importer
    ///    ever produced `safe_to_send == true`, this fails.
    /// 2. The provider-bound selection path (`MemoryDecisionEngine::is_safe_to_send`,
    ///    the public eligibility check) FILTERS OUT every imported entry, mirroring
    ///    the CLI's `entry.safe_to_send` filter in `mimir-cli/src/main.rs`. Even if
    ///    an imported entry were scored above the threshold, the `safe_to_send`
    ///    gate keeps it out — so imported content cannot reach a provider.
    #[test]
    fn test_imported_entries_quarantined_from_provider() {
        use crate::engine::{MemoryDecisionEngine, ScoreSignals};

        // Synthetic sessions via real importers — no network, no real secrets.
        let aider_md =
            "## USER\n\nLeaked api key sk-synthetic-not-a-real-secret\n\n## ASSISTANT\n\nNoted.\n";
        let (_aider_dir, aider_path) = make_temp_file("history.md", aider_md);
        let codex_jsonl = "{\"role\":\"user\",\"content\":\"internal note: do not exfiltrate\"}\n{\"role\":\"assistant\",\"content\":\"understood\"}\n";
        let (_codex_dir, codex_path) = make_temp_file("session.jsonl", codex_jsonl);

        let mut imported = AiderImporter.import(&aider_path).unwrap();
        imported.extend(CodexImporter.import(&codex_path).unwrap());

        // Sanity: we actually imported something to assert over.
        assert_eq!(imported.len(), 4, "expected 2 aider + 2 codex entries");

        // (1) Every imported entry is quarantined.
        for entry in &imported {
            assert!(
                !entry.safe_to_send,
                "imported entry {} must be safe_to_send=false (R-14)",
                entry.entry_id
            );
            assert_eq!(
                entry.scope, "private",
                "imported entry {} must be scope=private",
                entry.entry_id
            );
            assert_eq!(
                entry.confidence, "provisional",
                "imported entry {} must be confidence=provisional",
                entry.entry_id
            );
            assert!(
                entry.imported_from.is_some(),
                "imported entry {} must record imported_from",
                entry.entry_id
            );
        }

        // (2) The provider-bound selection path excludes every imported entry.
        let engine = MemoryDecisionEngine::new();
        for entry in &imported {
            assert!(
                !engine.is_safe_to_send(entry),
                "imported entry {} must NOT be eligible for the provider",
                entry.entry_id
            );
        }
        let eligible: Vec<_> = imported
            .iter()
            .filter(|entry| engine.is_safe_to_send(entry))
            .collect();
        assert!(
            eligible.is_empty(),
            "no imported entry may pass provider-bound selection, found {}",
            eligible.len()
        );

        // Mirror the CLI publish filter (`mimir-cli/src/main.rs` ~4867): the raw
        // `safe_to_send` flag the CLI relies on must reject all imports too.
        let cli_safe: Vec<_> = imported.iter().filter(|entry| entry.safe_to_send).collect();
        assert!(
            cli_safe.is_empty(),
            "CLI safe_to_send filter must reject all imported entries"
        );

        // Defense in depth: even a maximal score cannot flip the gate, because
        // `is_safe_to_send` AND-gates the `safe_to_send` flag (which stays false).
        let mut scored = imported[0].clone();
        engine.score_entry(
            &mut scored,
            &ScoreSignals {
                severity: 1.0,
                recurrence_count: 100,
                success_rate: 1.0,
                task_relevance: 1.0,
                token_savings: 10_000,
            },
        );
        assert!(
            scored.promotion_score.unwrap() >= MemoryDecisionEngine::safe_to_send_threshold(),
            "test setup: scored entry should clear the score threshold"
        );
        assert!(
            !engine.is_safe_to_send(&scored),
            "a high score must not make an imported entry sendable while safe_to_send=false"
        );
    }

    #[test]
    fn test_opencode_import() {
        let json = r#"[{"role":"user","content":"Fix the memory leak"},{"role":"assistant","content":"Use Arc instead of Rc."}]"#;
        let (_dir, path) = make_temp_file("log.json", json);
        let importer = OpenCodeImporter;
        let entries = importer.import(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_schema_valid(&entries[0]);
        assert_eq!(entries[0].kind, "experience");
        assert!(entries[0].body.contains("[user]"));
        assert!(entries[0].body.contains("Fix the memory leak"));
        assert!(entries[1].body.contains("[assistant]"));
        assert!(entries[1].body.contains("Use Arc instead of Rc."));
        assert_eq!(entries[0].retrieval_tags, vec!["opencode"]);
        let im = entries[0].imported_from.as_ref().unwrap();
        assert_eq!(im.tool, "opencode");
    }

    #[test]
    fn test_opencode_db_import() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("opencode.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE message (id TEXT PRIMARY KEY, role TEXT)", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, text TEXT)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO message (id, role) VALUES ('m1', 'user')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO part (id, message_id, text) VALUES ('p1', 'm1', 'Open the native DB')",
            [],
        )
        .unwrap();

        let importer = OpenCodeImporter;
        let entries = importer.import(&path.to_string_lossy()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_schema_valid(&entries[0]);
        assert!(entries[0].body.contains("Open the native DB"));
    }

    #[test]
    fn test_default_session_discovery_uses_synthetic_roots() {
        let cwd = tempfile::tempdir().unwrap();
        fs::write(cwd.path().join(".aider.chat.history.md"), "## USER\nhello").unwrap();

        let home = tempfile::tempdir().unwrap();
        let claude_project = home.path().join(".claude/projects/project-a");
        fs::create_dir_all(&claude_project).unwrap();
        fs::write(claude_project.join("session.jsonl"), "{}").unwrap();
        fs::write(claude_project.join("history.jsonl"), "{}").unwrap();

        let codex_home = tempfile::tempdir().unwrap();
        let codex_day = codex_home.path().join("sessions/2026/05/20");
        fs::create_dir_all(&codex_day).unwrap();
        fs::write(codex_day.join("rollout-synthetic.jsonl"), "{}").unwrap();

        let xdg = tempfile::tempdir().unwrap();
        let opencode_dir = xdg.path().join("opencode");
        fs::create_dir_all(&opencode_dir).unwrap();
        fs::write(opencode_dir.join("opencode.db"), "").unwrap();

        let roots = DiscoveryRoots {
            cwd: cwd.path().to_path_buf(),
            home: Some(home.path().to_path_buf()),
            xdg_data_home: Some(xdg.path().to_path_buf()),
            codex_home: Some(codex_home.path().to_path_buf()),
            max_files: 16,
            include_archived: false,
        };

        let aider = discover_sessions("aider", &roots).unwrap();
        assert_eq!(aider.len(), 1);
        assert!(aider[0].path.ends_with(".aider.chat.history.md"));

        let claude = discover_sessions("claude-code", &roots).unwrap();
        assert_eq!(claude.len(), 1);
        assert!(claude[0].path.ends_with("session.jsonl"));

        let codex = discover_sessions("codex", &roots).unwrap();
        assert_eq!(codex.len(), 1);
        assert!(codex[0].path.ends_with("rollout-synthetic.jsonl"));

        let opencode = discover_sessions("opencode", &roots).unwrap();
        assert_eq!(opencode.len(), 1);
        assert!(opencode[0].path.ends_with("opencode.db"));
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
