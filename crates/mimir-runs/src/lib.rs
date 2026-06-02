//! `mimir-runs` — Run directory layout, event log writer, trace store, and
//! artifact storage.
//!
//! This is the only crate that writes under `.mimir/runs/`.

#![warn(missing_docs)]

use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::error::Error;
use std::fs;
use std::io::{ErrorKind, Write};

const TRACE_SPANS_FILE: &str = "trace.spans.jsonl";

/// Unique run identifier: `YYYYMMDD-HHMMSS-8hex`.
#[derive(Debug, Clone)]
pub struct RunId(pub String);

/// Error returned when a run identifier is not in Mimir's opaque ID format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRunId;

impl std::fmt::Display for InvalidRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid run id")
    }
}

impl Error for InvalidRunId {}

impl RunId {
    /// Generate a new run ID based on the current timestamp.
    pub fn generate() -> Self {
        let now = Utc::now();
        let prefix = now.format("%Y%m%d-%H%M%S").to_string();
        let suffix = format!("{:08x}", fastrand::u32(..));
        Self(format!("{}-{}", prefix, suffix))
    }

    /// Parse and validate a run ID before it is used in a filesystem path.
    pub fn parse(value: impl Into<String>) -> Result<Self, InvalidRunId> {
        let value = value.into();
        if Self::is_valid(value.as_str()) {
            Ok(Self(value))
        } else {
            Err(InvalidRunId)
        }
    }

    /// Return true when `value` matches Mimir's generated run ID format.
    pub fn is_valid(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 24
            && bytes[..8].iter().all(u8::is_ascii_digit)
            && bytes[8] == b'-'
            && bytes[9..15].iter().all(u8::is_ascii_digit)
            && bytes[15] == b'-'
            && bytes[16..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(*byte, b'a'..=b'f'))
    }

    /// Borrow the run ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Layout manager for `.mimir/runs/<run_id>/`.
#[derive(Debug)]
pub struct RunDir {
    root: Utf8PathBuf,
}

impl RunDir {
    /// Create a new run directory under the given `.mimir` root.
    pub fn create(mimir_root: &Utf8PathBuf, run_id: &RunId) -> std::io::Result<Self> {
        validate_run_id(run_id)?;
        fs::create_dir_all(mimir_root)?;
        ensure_regular_dir(mimir_root, "mimir root")?;
        let runs_root = mimir_root.join("runs");
        fs::create_dir_all(&runs_root)?;
        ensure_regular_dir(&runs_root, "runs root")?;
        let root = runs_root.join(run_id.as_str());
        fs::create_dir_all(&root)?;
        ensure_regular_dir(&root, "run directory")?;
        ensure_contained(&runs_root, &root)?;
        Ok(Self { root })
    }

    /// Open an existing run directory without creating filesystem artifacts.
    pub fn open(mimir_root: &Utf8PathBuf, run_id: &RunId) -> std::io::Result<Self> {
        validate_run_id(run_id)?;
        ensure_regular_dir(mimir_root, "mimir root")?;
        let runs_root = mimir_root.join("runs");
        ensure_regular_dir(&runs_root, "runs root")?;
        let root = runs_root.join(run_id.as_str());
        ensure_regular_dir(&root, "run directory")?;
        ensure_contained(&runs_root, &root)?;
        Ok(Self { root })
    }

    /// Path to the run directory.
    pub fn root(&self) -> &Utf8PathBuf {
        &self.root
    }

    /// Path to the context packet file.
    pub fn context_packet_path(&self) -> Utf8PathBuf {
        self.root.join("context_packet.json")
    }

    /// Path to the budget ledger file.
    pub fn budget_ledger_path(&self) -> Utf8PathBuf {
        self.root.join("budget_ledger.json")
    }

    /// Path to the events JSONL file.
    pub fn events_path(&self) -> Utf8PathBuf {
        self.root.join("events.jsonl")
    }

    /// Path to the trace span JSONL file.
    pub fn trace_spans_path(&self) -> Utf8PathBuf {
        self.root.join(TRACE_SPANS_FILE)
    }

    /// Write an event line to `events.jsonl`.
    pub fn append_event(&self, event: &impl Serialize) -> std::io::Result<()> {
        let path = self.events_path();
        ensure_regular_file_if_exists(&path, "events file")?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let line = serde_json::to_string(event)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    /// Write a redacted trace span line to `trace.spans.jsonl`.
    pub fn append_trace_span(&self, span: &mimir_telemetry::TraceSpan) -> std::io::Result<()> {
        let path = self.trace_spans_path();
        ensure_regular_file_if_exists(&path, "trace spans file")?;
        let mut value = serde_json::to_value(span)?;
        mimir_security::redact_json_value(&mut value);
        if let Some(workspace_root) = self.workspace_root() {
            sanitize_trace_paths(&mut value, workspace_root);
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        serde_json::to_writer(&mut file, &value)?;
        writeln!(file)?;
        Ok(())
    }

    /// Read the run event log as UTF-8 after rejecting symlinks and directories.
    pub fn read_events_to_string(&self) -> std::io::Result<String> {
        let path = self.events_path();
        ensure_regular_file(&path, "events file")?;
        fs::read_to_string(path)
    }

    /// Read the run trace span log as UTF-8 after rejecting symlinks and directories.
    pub fn read_trace_spans_to_string(&self) -> std::io::Result<String> {
        let path = self.trace_spans_path();
        ensure_regular_file(&path, "trace spans file")?;
        fs::read_to_string(path)
    }

    fn workspace_root(&self) -> Option<&Utf8Path> {
        self.root.parent()?.parent()?.parent()
    }
}

/// Atomic write: write to temp file, then rename.
pub fn atomic_write(path: &Utf8PathBuf, contents: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    ensure_regular_file_if_exists(&temp, "temporary artifact file")?;
    fs::write(&temp, contents)?;
    fs::rename(&temp, path)?;
    Ok(())
}

fn validate_run_id(run_id: &RunId) -> std::io::Result<()> {
    if RunId::is_valid(run_id.as_str()) {
        Ok(())
    } else {
        Err(std::io::Error::new(ErrorKind::InvalidInput, InvalidRunId))
    }
}

fn ensure_regular_dir(path: &Utf8Path, label: &str) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            std::io::Error::new(ErrorKind::NotFound, format!("{label} not found"))
        } else {
            error
        }
    })?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!("{label} must not be a symlink"),
        ));
    }
    if !file_type.is_dir() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("{label} is not a directory"),
        ));
    }
    Ok(())
}

fn ensure_regular_file_if_exists(path: &Utf8Path, label: &str) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure_regular_file_type(metadata.file_type(), label),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn ensure_regular_file(path: &Utf8Path, label: &str) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            std::io::Error::new(ErrorKind::NotFound, format!("{label} not found"))
        } else {
            error
        }
    })?;
    ensure_regular_file_type(metadata.file_type(), label)
}

fn ensure_regular_file_type(file_type: fs::FileType, label: &str) -> std::io::Result<()> {
    if file_type.is_symlink() {
        return Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            format!("{label} must not be a symlink"),
        ));
    }
    if !file_type.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            format!("{label} is not a regular file"),
        ));
    }
    Ok(())
}

fn ensure_contained(runs_root: &Utf8Path, run_dir: &Utf8Path) -> std::io::Result<()> {
    let canonical_runs = canonical_utf8(runs_root)?;
    let canonical_run = canonical_utf8(run_dir)?;
    if canonical_run.starts_with(&canonical_runs) {
        Ok(())
    } else {
        Err(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "run directory escapes runs root",
        ))
    }
}

fn canonical_utf8(path: &Utf8Path) -> std::io::Result<Utf8PathBuf> {
    Utf8PathBuf::from_path_buf(fs::canonicalize(path)?).map_err(|path| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!("path is not UTF-8: {}", path.display()),
        )
    })
}

fn sanitize_trace_paths(value: &mut Value, workspace_root: &Utf8Path) {
    match value {
        Value::Object(map) => {
            let keys = map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let Some(child) = map.get_mut(&key) else {
                    continue;
                };
                if trace_path_key(&key) {
                    if let Some(path) = child.as_str() {
                        *child = Value::String(trace_display_path(workspace_root, path));
                        continue;
                    }
                }
                sanitize_trace_paths(child, workspace_root);
            }
        }
        Value::Array(items) => {
            for item in items {
                sanitize_trace_paths(item, workspace_root);
            }
        }
        Value::String(text) => {
            let sanitized = sanitize_trace_string(workspace_root, text);
            if sanitized != *text {
                *text = sanitized;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn trace_path_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower == "path"
        || lower == "workspace_root"
        || lower.ends_with("_path")
        || lower.ends_with("_paths")
}

fn trace_display_path(workspace_root: &Utf8Path, value: &str) -> String {
    let redacted = mimir_security::redact_secrets(value);
    let candidate = Utf8Path::new(&redacted);
    if candidate.is_absolute() {
        if let Ok(relative) = candidate.strip_prefix(workspace_root) {
            return if relative.as_str().is_empty() {
                ".".to_string()
            } else {
                relative.as_str().replace('\\', "/")
            };
        }
        return candidate
            .file_name()
            .map(|name| format!("[external]/{name}"))
            .unwrap_or_else(|| "[external path]".to_string());
    }
    sanitize_trace_string(workspace_root, &redacted)
}

fn sanitize_trace_string(workspace_root: &Utf8Path, value: &str) -> String {
    let mut sanitized = mimir_security::redact_secrets(value);
    for prefix in trace_workspace_prefixes(workspace_root) {
        let slash_prefix = format!("{prefix}/");
        let backslash_prefix = format!("{prefix}\\");
        sanitized = sanitized.replace(&slash_prefix, "");
        sanitized = sanitized.replace(&backslash_prefix, "");
        sanitized = sanitized.replace(&prefix, ".");
    }
    sanitized
}

fn trace_workspace_prefixes(workspace_root: &Utf8Path) -> Vec<String> {
    let mut prefixes = Vec::new();
    let raw_workspace_root = workspace_root.as_str();
    if !raw_workspace_root.is_empty() {
        prefixes.push(raw_workspace_root.to_string());
    }
    if let Ok(canonical) = canonical_utf8(workspace_root) {
        let canonical = canonical.as_str().to_string();
        if !prefixes.contains(&canonical) {
            prefixes.push(canonical);
        }
    }
    prefixes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mimir_root(name: &str) -> Utf8PathBuf {
        let root =
            Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string()).join(format!(
                "mimir-test-{name}-{}-{:08x}",
                std::process::id(),
                fastrand::u32(..)
            ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn run_id_format() {
        let id = RunId::generate();
        let re = regex::Regex::new(r"^\d{8}-\d{6}-[0-9a-f]{8}$").unwrap();
        assert!(
            re.is_match(&id.0),
            "RunId {} does not match expected format",
            id.0
        );
    }

    #[test]
    fn run_id_parse_rejects_unsafe_values() {
        assert!(RunId::parse("20260101-120000-abcdef01").is_ok());
        for value in [
            "",
            ".",
            "..",
            "../20260101-120000-abcdef01",
            "20260101-120000-abcdef01/extra",
            "/tmp/20260101-120000-abcdef01",
            "%2e%2e",
            "20260101-120000-ABCDEF01",
            "run-1",
            "20260101-120000-abcdef0g",
        ] {
            assert!(RunId::parse(value).is_err(), "{value} should be rejected");
        }
    }

    #[test]
    fn run_dir_create_and_paths() {
        let tmp = camino::Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string());
        let mimir_root = tmp.join("mimir-test-run");
        let _ = fs::remove_dir_all(&mimir_root);
        let run_id = RunId("20260101-120000-abcdef01".to_string());
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        assert!(run_dir
            .context_packet_path()
            .as_str()
            .contains("context_packet.json"));
        assert!(run_dir
            .budget_ledger_path()
            .as_str()
            .contains("budget_ledger.json"));
        assert!(run_dir.events_path().as_str().contains("events.jsonl"));
        assert!(run_dir
            .trace_spans_path()
            .as_str()
            .contains("trace.spans.jsonl"));
        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[test]
    fn run_dir_rejects_invalid_ids_before_joining_paths() {
        let mimir_root = test_mimir_root("invalid-id");
        fs::create_dir_all(&mimir_root).unwrap();
        for value in [
            "../outside-run",
            "/tmp/outside-run",
            "20260101-120000-abcdef01/extra",
            "%2e%2e",
        ] {
            let error = RunDir::create(&mimir_root, &RunId(value.to_string())).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidInput);
        }
        assert!(!mimir_root.join("outside-run").exists());
        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[cfg(unix)]
    #[test]
    fn run_dir_open_rejects_symlink_escape() {
        let mimir_root = test_mimir_root("symlink-run");
        let runs_root = mimir_root.join("runs");
        let outside = test_mimir_root("symlink-outside");
        fs::create_dir_all(&runs_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let run_id = RunId::parse("20260101-120000-abcdef97").unwrap();
        std::os::unix::fs::symlink(outside.as_std_path(), runs_root.join(run_id.as_str())).unwrap();

        let error = RunDir::open(&mimir_root, &run_id).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);

        let _ = fs::remove_dir_all(&mimir_root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn run_dir_create_rejects_preexisting_symlink() {
        let mimir_root = test_mimir_root("create-symlink-run");
        let runs_root = mimir_root.join("runs");
        let outside = test_mimir_root("create-symlink-outside");
        fs::create_dir_all(&runs_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let run_id = RunId::parse("20260101-120000-abcdef96").unwrap();
        std::os::unix::fs::symlink(outside.as_std_path(), runs_root.join(run_id.as_str())).unwrap();

        let error = RunDir::create(&mimir_root, &run_id).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);

        let _ = fs::remove_dir_all(&mimir_root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn run_dir_rejects_symlinked_mimir_root() {
        let mimir_root = test_mimir_root("symlink-mimir-root");
        let outside = test_mimir_root("symlink-mimir-outside");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(outside.as_std_path(), &mimir_root).unwrap();
        let run_id = RunId::parse("20260101-120000-abcdef95").unwrap();

        let error = RunDir::create(&mimir_root, &run_id).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);

        let _ = fs::remove_file(&mimir_root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn run_dir_open_does_not_create_missing_dir() {
        let tmp = camino::Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string());
        let mimir_root = tmp.join("mimir-test-open-missing");
        let _ = fs::remove_dir_all(&mimir_root);
        let run_id = RunId("20260101-120000-abcdef99".to_string());
        let result = RunDir::open(&mimir_root, &run_id);
        assert!(result.is_err());
        assert!(!mimir_root.join("runs").join(run_id.0).exists());
        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[test]
    fn run_dir_open_existing_dir() {
        let tmp = camino::Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string());
        let mimir_root = tmp.join("mimir-test-open-existing");
        let _ = fs::remove_dir_all(&mimir_root);
        let run_id = RunId("20260101-120000-abcdef98".to_string());
        let created = RunDir::create(&mimir_root, &run_id).unwrap();
        let opened = RunDir::open(&mimir_root, &run_id).unwrap();
        assert_eq!(created.root(), opened.root());
        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[test]
    fn run_dir_append_event() {
        let tmp = camino::Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string());
        let mimir_root = tmp.join("mimir-test-events");
        let _ = fs::remove_dir_all(&mimir_root);
        let run_id = RunId("20260101-120000-abcdef02".to_string());
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        #[derive(Serialize)]
        struct Ev {
            msg: String,
        }
        run_dir
            .append_event(&Ev {
                msg: "hello".to_string(),
            })
            .unwrap();
        let contents = fs::read_to_string(run_dir.events_path()).unwrap();
        assert!(contents.contains("hello"));
        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[cfg(unix)]
    #[test]
    fn run_dir_events_reject_symlink_files() {
        let mimir_root = test_mimir_root("event-symlink");
        let run_id = RunId::parse("20260101-120000-abcdef04").unwrap();
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        let outside = mimir_root.join("outside-events.jsonl");
        fs::write(&outside, "{\"msg\":\"outside\"}\n").unwrap();
        std::os::unix::fs::symlink(outside.as_std_path(), run_dir.events_path()).unwrap();

        #[derive(Serialize)]
        struct Ev {
            msg: String,
        }

        let append_error = run_dir
            .append_event(&Ev {
                msg: "blocked".to_string(),
            })
            .unwrap_err();
        assert_eq!(append_error.kind(), ErrorKind::PermissionDenied);
        let read_error = run_dir.read_events_to_string().unwrap_err();
        assert_eq!(read_error.kind(), ErrorKind::PermissionDenied);
        let outside_contents = fs::read_to_string(outside).unwrap();
        assert!(!outside_contents.contains("blocked"));

        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[test]
    fn run_dir_append_trace_span_redacts_and_scrubs_workspace_paths() {
        let workspace_root = test_mimir_root("trace-span-workspace");
        let mimir_root = workspace_root.join(".mimir");
        let run_id = RunId::parse("20260101-120000-abcdef05").unwrap();
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        let source_path = workspace_root.join("src/main.rs");
        let span = mimir_telemetry::TraceSpan {
            schema_version: 1,
            span_id: "0123456789abcdef".to_string(),
            trace_id: Some("0123456789abcdef0123456789abcdef".to_string()),
            parent_id: None,
            name: "mimir.context.build".to_string(),
            kind: Some(mimir_telemetry::TraceSpanKind::Internal),
            start_us: 1,
            end_us: 2,
            attrs: Some(serde_json::json!({
                "api_key": "sk-123456789012345678901234",
                "source_path": source_path.to_string(),
                "message": format!("read {source_path}")
            })),
            events: None,
            status: Some(mimir_telemetry::TraceSpanStatus {
                code: Some(mimir_telemetry::TraceSpanStatusCode::Ok),
                message: None,
            }),
        };

        run_dir.append_trace_span(&span).unwrap();
        let contents = run_dir.read_trace_spans_to_string().unwrap();
        assert!(!contents.contains("sk-123456789012345678901234"));
        assert!(!contents.contains(workspace_root.as_str()));
        assert!(contents.contains("src/main.rs"));
        let line: Value = serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(line["schema_version"], 1);
        assert_eq!(line["span_id"], "0123456789abcdef");
        assert_eq!(line["attrs"]["source_path"], "src/main.rs");

        let _ = fs::remove_dir_all(&workspace_root);
    }

    #[test]
    fn run_dir_append_trace_span_with_relative_root_preserves_metadata_strings() {
        let mimir_root = Utf8PathBuf::from(format!(
            "mimir-test-relative-trace-{}-{:08x}",
            std::process::id(),
            fastrand::u32(..)
        ));
        let _ = fs::remove_dir_all(&mimir_root);
        let run_id = RunId::parse("20260101-120000-abcdef07").unwrap();
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        let span = mimir_telemetry::TraceSpan {
            schema_version: 1,
            span_id: "0123456789abcdef".to_string(),
            trace_id: Some("0123456789abcdef0123456789abcdef".to_string()),
            parent_id: None,
            name: "mimir.context.build".to_string(),
            kind: Some(mimir_telemetry::TraceSpanKind::Internal),
            start_us: 1,
            end_us: 2,
            attrs: Some(serde_json::json!({
                "run_id": run_id.to_string(),
                "packet_id": "pkt-20260101-120000-abcdef07",
                "provider": "glm",
                "model": "glm-5.1",
                "status": "ok",
            })),
            events: None,
            status: Some(mimir_telemetry::TraceSpanStatus {
                code: Some(mimir_telemetry::TraceSpanStatusCode::Ok),
                message: None,
            }),
        };

        run_dir.append_trace_span(&span).unwrap();
        let contents = run_dir.read_trace_spans_to_string().unwrap();
        assert!(contents.contains("mimir.context.build"));
        assert!(contents.contains("glm-5.1"));
        assert!(!contents.contains(".m.i.m.i.r"));

        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[cfg(unix)]
    #[test]
    fn run_dir_trace_spans_reject_symlink_files() {
        let mimir_root = test_mimir_root("trace-span-symlink");
        let run_id = RunId::parse("20260101-120000-abcdef06").unwrap();
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        let outside = mimir_root.join("outside-trace.spans.jsonl");
        fs::write(&outside, "{\"name\":\"outside\"}\n").unwrap();
        std::os::unix::fs::symlink(outside.as_std_path(), run_dir.trace_spans_path()).unwrap();
        let span = mimir_telemetry::TraceSpan {
            schema_version: 1,
            span_id: "0123456789abcdef".to_string(),
            trace_id: None,
            parent_id: None,
            name: "blocked".to_string(),
            kind: Some(mimir_telemetry::TraceSpanKind::Internal),
            start_us: 1,
            end_us: 2,
            attrs: None,
            events: None,
            status: None,
        };

        let append_error = run_dir.append_trace_span(&span).unwrap_err();
        assert_eq!(append_error.kind(), ErrorKind::PermissionDenied);
        let read_error = run_dir.read_trace_spans_to_string().unwrap_err();
        assert_eq!(read_error.kind(), ErrorKind::PermissionDenied);
        let outside_contents = fs::read_to_string(outside).unwrap();
        assert!(!outside_contents.contains("blocked"));

        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[test]
    fn run_id_display() {
        let id = RunId("20260101-120000-abcdef01".to_string());
        assert_eq!(id.to_string(), "20260101-120000-abcdef01");
    }

    #[test]
    fn run_dir_multiple_events() {
        let tmp = camino::Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string());
        let mimir_root = tmp.join("mimir-test-multi-events");
        let _ = fs::remove_dir_all(&mimir_root);
        let run_id = RunId("20260101-120000-abcdef03".to_string());
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        #[derive(Serialize)]
        struct Ev {
            msg: String,
        }
        run_dir
            .append_event(&Ev {
                msg: "first".to_string(),
            })
            .unwrap();
        run_dir
            .append_event(&Ev {
                msg: "second".to_string(),
            })
            .unwrap();
        let contents = fs::read_to_string(run_dir.events_path()).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("first"));
        assert!(lines[1].contains("second"));
        let _ = fs::remove_dir_all(&mimir_root);
    }

    #[test]
    fn atomic_write_overwrite() {
        let tmp = camino::Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string());
        let path = tmp.join("atomic-overwrite.txt");
        let _ = fs::remove_file(&path);
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "second");
        let _ = fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_rejects_symlinked_temp_path() {
        let root = test_mimir_root("atomic-symlink");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("artifact.json");
        let temp = path.with_extension("tmp");
        let outside = root.join("outside.tmp");
        fs::write(&outside, "outside").unwrap();
        std::os::unix::fs::symlink(outside.as_std_path(), &temp).unwrap();

        let error = atomic_write(&path, b"blocked").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(outside).unwrap(), "outside");

        let _ = fs::remove_dir_all(&root);
    }
}
