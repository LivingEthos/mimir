//! Memory store: durable, versioned, retrievable.

use mimir_schemas::MemoryEntry;
use std::collections::HashMap;

/// In-memory store (stub).
#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: HashMap<String, MemoryEntry>,
}

impl MemoryStore {
    /// Create a new store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an entry.
    pub fn insert(&mut self, entry: MemoryEntry) {
        self.entries.insert(entry.entry_id.clone(), entry);
    }

    /// Get an entry by ID.
    pub fn get(&self, id: &str) -> Option<&MemoryEntry> {
        self.entries.get(id)
    }

    /// List all entries.
    pub fn list(&self) -> Vec<&MemoryEntry> {
        self.entries.values().collect()
    }
}
