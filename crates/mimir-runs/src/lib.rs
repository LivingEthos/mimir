//! `mimir-runs` — Run directory layout, event log writer, trace store, and
//! artifact storage.
//!
//! This is the only crate that writes under `.mimir/runs/`.

#![warn(missing_docs)]

use camino::Utf8PathBuf;
use chrono::Utc;
use serde::Serialize;
use std::fs;
use std::io::Write;

/// Unique run identifier: `YYYYMMDD-HHMMSS-8hex`.
#[derive(Debug, Clone)]
pub struct RunId(pub String);

impl RunId {
    /// Generate a new run ID based on the current timestamp.
    pub fn generate() -> Self {
        let now = Utc::now();
        let prefix = now.format("%Y%m%d-%H%M%S").to_string();
        let suffix = format!("{:08x}", fastrand::u32(..));
        Self(format!("{}-{}", prefix, suffix))
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Layout manager for `.mimir/runs/<run_id>/`.
pub struct RunDir {
    root: Utf8PathBuf,
}

impl RunDir {
    /// Create a new run directory under the given `.mimir` root.
    pub fn create(mimir_root: &Utf8PathBuf, run_id: &RunId) -> std::io::Result<Self> {
        let root = mimir_root.join("runs").join(&run_id.0);
        fs::create_dir_all(&root)?;
        Ok(Self { root })
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

    /// Write an event line to `events.jsonl`.
    pub fn append_event(&self, event: &impl Serialize) -> std::io::Result<()> {
        let path = self.events_path();
        let mut file = fs::OpenOptions::new().create(true).append(true).open(path)?;
        let line = serde_json::to_string(event)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }
}

/// Atomic write: write to temp file, then rename.
pub fn atomic_write(path: &Utf8PathBuf, contents: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, contents)?;
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_format() {
        let id = RunId::generate();
        let re = regex::Regex::new(r"^\d{8}-\d{6}-[0-9a-f]{8}$").unwrap();
        assert!(re.is_match(&id.0), "RunId {} does not match expected format", id.0);
    }

    #[test]
    fn run_dir_create_and_paths() {
        let tmp = camino::Utf8PathBuf::from(std::env::temp_dir().to_string_lossy().to_string());
        let mimir_root = tmp.join("mimir-test-run");
        let _ = fs::remove_dir_all(&mimir_root);
        let run_id = RunId("20260101-120000-abcdef01".to_string());
        let run_dir = RunDir::create(&mimir_root, &run_id).unwrap();
        assert!(run_dir.context_packet_path().as_str().contains("context_packet.json"));
        assert!(run_dir.budget_ledger_path().as_str().contains("budget_ledger.json"));
        assert!(run_dir.events_path().as_str().contains("events.jsonl"));
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
        struct Ev { msg: String }
        run_dir.append_event(&Ev { msg: "hello".to_string() }).unwrap();
        let contents = fs::read_to_string(run_dir.events_path()).unwrap();
        assert!(contents.contains("hello"));
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
        struct Ev { msg: String }
        run_dir.append_event(&Ev { msg: "first".to_string() }).unwrap();
        run_dir.append_event(&Ev { msg: "second".to_string() }).unwrap();
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
}
