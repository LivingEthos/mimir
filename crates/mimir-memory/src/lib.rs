//! Memory store: durable, versioned, retrievable.
//!
//! Provides SQLite-backed storage for memory entries with FTS5 full-text
//! search, a Memory Decision Engine for scoring, marker-block publishing,
//! and session importers.

#![warn(missing_docs)]

pub mod engine;
pub mod importers;
pub mod publish;
pub mod store;

pub use engine::{MemoryDecisionEngine, ScoreSignals, ScoreWeights};
pub use importers::{
    discover_sessions, importer_for, AiderImporter, ClaudeCodeImporter, CodexImporter,
    DiscoveredSession, DiscoveryRoots, OpenCodeImporter, SessionImporter,
};
pub use publish::{clear_published, publish, read_published};
pub use store::{AuditRecord, MemoryStore, StrategyRecord};

use thiserror::Error;

/// Errors that can occur in the memory crate.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// Database operation failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    /// I/O operation failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid path.
    #[error("invalid path: {0}")]
    InvalidPath(String),

    /// Serialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
