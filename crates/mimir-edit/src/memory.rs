//! Verify-before-learn: proposed memory entries recorded but not retrieved.
//!
//! Per 14-LEARNING-LAYER.md: proposed memory entries are *recorded* (not retrieved)
//! under `.mimir/runs/<run_id>/proposed_memory.json`.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A proposed memory entry awaiting human verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedMemoryEntry {
    /// Entry type: pattern, fix, lesson, etc.
    pub entry_type: String,
    /// Human-readable description.
    pub description: String,
    /// Related file paths.
    pub related_paths: Vec<String>,
    /// Confidence score (0.0-1.0).
    pub confidence: f64,
    /// Source run ID.
    pub source_run_id: String,
    /// Timestamp.
    pub created_at: String,
    /// Proposed tags for categorization.
    pub tags: Vec<String>,
}

/// Collection of proposed memory entries for a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProposedMemory {
    /// Run ID.
    pub run_id: String,
    /// Proposed entries.
    pub entries: Vec<ProposedMemoryEntry>,
    /// Whether any entry has been verified.
    pub verified: bool,
}

impl ProposedMemory {
    /// Create a new proposed memory collection.
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            entries: Vec::new(),
            verified: false,
        }
    }

    /// Add a proposed entry.
    pub fn propose(&mut self, entry: ProposedMemoryEntry) {
        self.entries.push(entry);
    }

    /// Save to `.mimir/runs/<run_id>/proposed_memory.json`.
    pub fn save(&self, runs_dir: &Path) -> std::io::Result<()> {
        let path = runs_dir.join(&self.run_id).join("proposed_memory.json");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Load from `.mimir/runs/<run_id>/proposed_memory.json`.
    pub fn load(runs_dir: &Path, run_id: &str) -> std::io::Result<Self> {
        let path = runs_dir.join(run_id).join("proposed_memory.json");
        let content = fs::read_to_string(path)?;
        let parsed: Self = serde_json::from_str(&content)?;
        Ok(parsed)
    }

    /// Mark all entries as verified (called after human review).
    pub fn verify_all(&mut self) {
        self.verified = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_propose_and_save_load() {
        let dir = TempDir::new().unwrap();
        let mut mem = ProposedMemory::new("run-123");
        mem.propose(ProposedMemoryEntry {
            entry_type: "pattern".into(),
            description: "Use Result instead of unwrap".into(),
            related_paths: vec!["src/main.rs".into()],
            confidence: 0.9,
            source_run_id: "run-123".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
            tags: vec!["error-handling".into()],
        });

        mem.save(dir.path()).unwrap();
        let loaded = ProposedMemory::load(dir.path(), "run-123").unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].entry_type, "pattern");
        assert!(!loaded.verified);

        let mut loaded = loaded;
        loaded.verify_all();
        assert!(loaded.verified);
    }
}
