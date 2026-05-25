//! Subagent packet lineage: parent run_id references.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Lineage tracker for subagent runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LineageTracker {
    /// Parent -> children mapping.
    children: HashMap<String, Vec<String>>,
    /// Child -> parent mapping.
    parents: HashMap<String, String>,
    /// Run metadata.
    runs: HashMap<String, RunMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunMeta {
    pub run_id: String,
    pub subagent: String,
    pub started_at: String,
    pub tokens_consumed: u32,
}

impl LineageTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new subagent run with its parent.
    pub fn spawn(&mut self, parent_run_id: &str, child_run_id: &str, subagent: &str) {
        self.children
            .entry(parent_run_id.to_string())
            .or_default()
            .push(child_run_id.to_string());
        self.parents
            .insert(child_run_id.to_string(), parent_run_id.to_string());
        self.runs.insert(
            child_run_id.to_string(),
            RunMeta {
                run_id: child_run_id.to_string(),
                subagent: subagent.to_string(),
                started_at: chrono::Utc::now().to_rfc3339(),
                tokens_consumed: 0,
            },
        );
    }

    /// Get all children of a run.
    pub fn children(&self, run_id: &str) -> Vec<&str> {
        self.children
            .get(run_id)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get parent of a run.
    pub fn parent(&self, run_id: &str) -> Option<&str> {
        self.parents.get(run_id).map(|s| s.as_str())
    }

    /// Get the full ancestry chain (oldest first).
    pub fn ancestry(&self, run_id: &str) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current = run_id;
        while let Some(parent) = self.parent(current) {
            chain.push(parent.to_string());
            current = parent;
        }
        chain.reverse();
        chain
    }

    /// Get all descendants of a run (BFS).
    pub fn descendants(&self, run_id: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut queue = vec![run_id.to_string()];
        while let Some(current) = queue.pop() {
            for child in self.children(&current) {
                result.push(child.to_string());
                queue.push(child.to_string());
            }
        }
        result
    }

    /// Update token consumption for a run.
    pub fn record_tokens(&mut self, run_id: &str, tokens: u32) {
        if let Some(meta) = self.runs.get_mut(run_id) {
            meta.tokens_consumed += tokens;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lineage_spawn() {
        let mut lt = LineageTracker::new();
        lt.spawn("run-1", "run-1a", "search");
        lt.spawn("run-1", "run-1b", "reviewer");
        lt.spawn("run-1a", "run-1a1", "file-analyst");

        assert_eq!(lt.children("run-1").len(), 2);
        assert_eq!(lt.parent("run-1a"), Some("run-1"));
        assert_eq!(lt.parent("run-1a1"), Some("run-1a"));
    }

    #[test]
    fn test_ancestry() {
        let mut lt = LineageTracker::new();
        lt.spawn("run-1", "run-1a", "search");
        lt.spawn("run-1a", "run-1a1", "file-analyst");

        let ancestry = lt.ancestry("run-1a1");
        assert_eq!(ancestry, vec!["run-1", "run-1a"]);
    }

    #[test]
    fn test_descendants() {
        let mut lt = LineageTracker::new();
        lt.spawn("run-1", "run-1a", "search");
        lt.spawn("run-1a", "run-1a1", "file-analyst");

        let desc = lt.descendants("run-1");
        assert!(desc.contains(&"run-1a".to_string()));
        assert!(desc.contains(&"run-1a1".to_string()));
    }
}
