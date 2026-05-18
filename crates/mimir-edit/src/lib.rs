//! `mimir-edit` — Patch apply engine, editable set enforcement, and worktree safety.

use std::collections::HashSet;

use thiserror::Error;

pub mod apply;
pub mod backup;
pub mod git;
pub mod test_runner;

pub use apply::PatchEngine;
pub use backup::BackupManager;
pub use git::WorktreeStatus;

/// Errors from the edit engine.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum EditError {
    #[error("file_not_editable: {path} is not in the editable set")]
    FileNotEditable { path: String },
    #[error("file_not_found: {path}")]
    FileNotFound { path: String },
    #[error("invalid_line_range: {path} lines {start}-{end} (file has {file_lines} lines)")]
    InvalidLineRange { path: String, start: usize, end: usize, file_lines: usize },
    #[error("diff_apply_failed: {path} — {reason}")]
    DiffApplyFailed { path: String, reason: String },
    #[error("dirty_worktree: {path} has uncommitted changes")]
    DirtyWorktree { path: String },
    #[error("backup_failed: {path} — {reason}")]
    BackupFailed { path: String, reason: String },
    #[error("io_error: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, EditError>;

/// An editable target set: paths the model is allowed to modify.
#[derive(Debug, Clone, Default)]
pub struct EditableSet {
    paths: HashSet<String>,
}

impl EditableSet {
    pub fn new() -> Self { Self::default() }
    pub fn from_paths(paths: Vec<String>) -> Self {
        Self { paths: paths.into_iter().collect() }
    }
    pub fn add(&mut self, path: impl Into<String>) {
        self.paths.insert(path.into());
    }
    pub fn contains(&self, path: &str) -> bool {
        self.paths.contains(path)
    }
    pub fn paths(&self) -> &HashSet<String> { &self.paths }
}

/// Extract target paths from a patch step.
pub fn patch_step_paths(step: &mimir_schemas::PatchStep) -> Vec<&str> {
    use mimir_schemas::PatchStep;
    match step {
        PatchStep::LineRange { path, .. } => vec![path],
        PatchStep::UnifiedDiff { path, .. } => vec![path],
        PatchStep::WholeFile { path, .. } => vec![path],
        PatchStep::Create { path, .. } => vec![path],
        PatchStep::Delete { path } => vec![path],
        PatchStep::Move { from, to } => vec![from, to],
    }
}

/// Verify all paths in a patch step are in the editable set.
pub fn verify_editable_set(step: &mimir_schemas::PatchStep, editable: &EditableSet) -> Result<()> {
    for path in patch_step_paths(step) {
        if !editable.contains(path) {
            return Err(EditError::FileNotEditable { path: path.to_string() });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editable_set() {
        let mut set = EditableSet::new();
        set.add("src/main.rs");
        set.add("src/lib.rs");
        assert!(set.contains("src/main.rs"));
        assert!(!set.contains("src/other.rs"));
    }

    #[test]
    fn test_verify_editable_passes() {
        let mut set = EditableSet::new();
        set.add("src/main.rs");
        let step = mimir_schemas::PatchStep::LineRange {
            path: "src/main.rs".into(),
            start_line: 1,
            end_line: 5,
            content: "new".into(),
        };
        assert!(verify_editable_set(&step, &set).is_ok());
    }

    #[test]
    fn test_verify_editable_fails() {
        let set = EditableSet::new();
        let step = mimir_schemas::PatchStep::LineRange {
            path: "src/main.rs".into(),
            start_line: 1,
            end_line: 5,
            content: "new".into(),
        };
        let err = verify_editable_set(&step, &set).unwrap_err();
        assert!(matches!(err, EditError::FileNotEditable { .. }));
    }
}

pub mod repair;

pub mod memory;
