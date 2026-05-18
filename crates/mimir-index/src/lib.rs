//! `mimir-index` — Repo index: files, imports, exports.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single file entry in the repo index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative path from repo root.
    pub path: String,
    /// Language (e.g., "rust", "typescript").
    pub language: String,
    /// Estimated token count.
    pub token_count: u32,
    /// Exported symbols.
    pub exports: Vec<String>,
    /// Imported symbols.
    pub imports: Vec<String>,
}

/// The repo index.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct RepoIndex {
    /// Files by path.
    pub files: HashMap<String, FileEntry>,
    /// Import graph: path -> imported paths.
    pub import_graph: HashMap<String, Vec<String>>,
}

impl RepoIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file entry.
    pub fn add(&mut self, entry: FileEntry) {
        self.files.insert(entry.path.clone(), entry);
    }

    /// Get a file by path.
    pub fn get(&self, path: &str) -> Option<&FileEntry> {
        self.files.get(path)
    }

    /// Total tokens across all files.
    pub fn total_tokens(&self) -> u32 {
        self.files.values().map(|f| f.token_count).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_get() {
        let mut index = RepoIndex::new();
        let entry = FileEntry {
            path: "src/main.rs".to_string(),
            language: "rust".to_string(),
            token_count: 100,
            exports: vec!["main".to_string()],
            imports: vec![],
        };
        index.add(entry);
        assert!(index.get("src/main.rs").is_some());
        assert_eq!(index.get("src/main.rs").unwrap().token_count, 100);
    }

    #[test]
    fn total_tokens_sum() {
        let mut index = RepoIndex::new();
        index.add(FileEntry {
            path: "a.rs".to_string(),
            language: "rust".to_string(),
            token_count: 100,
            exports: vec![],
            imports: vec![],
        });
        index.add(FileEntry {
            path: "b.rs".to_string(),
            language: "rust".to_string(),
            token_count: 200,
            exports: vec![],
            imports: vec![],
        });
        assert_eq!(index.total_tokens(), 300);
    }
}
