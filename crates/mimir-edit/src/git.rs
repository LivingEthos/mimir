//! Git worktree status checking.

use std::process::Command;

use camino::Utf8Path;

use crate::{EditError, Result};

/// Git worktree status for a repository.
#[derive(Debug, Clone, Default)]
pub struct WorktreeStatus {
    /// Whether the repo has uncommitted changes.
    pub is_dirty: bool,
    /// List of modified files.
    pub modified_files: Vec<String>,
    /// Whether this is a git repo at all.
    pub is_git_repo: bool,
}

impl WorktreeStatus {
    /// Check worktree status at the given path.
    pub fn check(base: &Utf8Path) -> Self {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(base.as_std_path())
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let modified: Vec<String> = stdout
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|l| {
                        // Format: XY filename or XY filename -> filename
                        l.split_whitespace().last().unwrap_or(l).to_string()
                    })
                    .collect();
                Self {
                    is_dirty: !modified.is_empty(),
                    modified_files: modified,
                    is_git_repo: true,
                }
            }
            _ => Self {
                is_git_repo: false,
                is_dirty: false,
                modified_files: vec![],
            },
        }
    }

    /// Check if a specific file has uncommitted changes.
    pub fn is_file_dirty(&self, path: &str) -> bool {
        self.modified_files.iter().any(|f| f == path)
    }
}
