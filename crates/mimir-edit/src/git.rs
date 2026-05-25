//! Git worktree status checking.

use std::process::Command;

use camino::Utf8Path;

/// Git worktree status for a repository.
#[derive(Debug, Clone, Default)]
pub struct WorktreeStatus {
    /// Whether the repo has uncommitted changes.
    pub is_dirty: bool,
    /// List of modified files.
    pub modified_files: Vec<String>,
    /// Whether this is a git repo at all.
    pub is_git_repo: bool,
    /// Git status error when inside a repo but status could not be read.
    pub status_error: Option<String>,
}

impl WorktreeStatus {
    /// Check worktree status at the given path.
    pub fn check(base: &Utf8Path) -> Self {
        let is_repo = Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(base.as_std_path())
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        if !is_repo {
            return Self {
                is_git_repo: false,
                is_dirty: false,
                modified_files: vec![],
                status_error: None,
            };
        }

        let output = Command::new("git")
            .args(["status", "--porcelain=v1", "--untracked-files=all", "-z"])
            .current_dir(base.as_std_path())
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let modified = parse_porcelain_z(&out.stdout);
                Self {
                    is_dirty: !modified.is_empty(),
                    modified_files: modified,
                    is_git_repo: true,
                    status_error: None,
                }
            }
            Ok(out) => Self {
                is_git_repo: true,
                is_dirty: true,
                modified_files: vec![],
                status_error: Some(String::from_utf8_lossy(&out.stderr).trim().to_string()),
            },
            Err(error) => Self {
                is_git_repo: true,
                is_dirty: true,
                modified_files: vec![],
                status_error: Some(error.to_string()),
            },
        }
    }

    /// Check if a specific file has uncommitted changes.
    pub fn is_file_dirty(&self, path: &str) -> bool {
        self.modified_files.iter().any(|dirty| {
            dirty == path
                || dirty.strip_suffix('/').is_some_and(|dir| {
                    path.strip_prefix(dir)
                        .is_some_and(|rest| rest.starts_with('/'))
                })
        })
    }
}

fn parse_porcelain_z(output: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());

    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        let status = &record[..2];
        if let Ok(path) = std::str::from_utf8(&record[3..]) {
            paths.push(path.to_string());
        }

        if status.contains(&b'R') || status.contains(&b'C') {
            if let Some(original) = records.next() {
                if let Ok(path) = std::str::from_utf8(original) {
                    paths.push(path.to_string());
                }
            }
        }
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::parse_porcelain_z;

    #[test]
    fn parses_paths_with_spaces_from_porcelain_z() {
        let paths = parse_porcelain_z(b" M src/foo bar.rs\0?? new file.txt\0");
        assert_eq!(paths, vec!["src/foo bar.rs", "new file.txt"]);
    }

    #[test]
    fn parses_rename_records_from_porcelain_z() {
        let paths = parse_porcelain_z(b"R  new name.rs\0old name.rs\0");
        assert_eq!(paths, vec!["new name.rs", "old name.rs"]);
    }

    #[test]
    fn dirty_directory_marks_descendant_dirty() {
        let status = super::WorktreeStatus {
            is_dirty: true,
            modified_files: vec!["src/generated/".to_string()],
            is_git_repo: true,
            status_error: None,
        };
        assert!(status.is_file_dirty("src/generated/file.rs"));
        assert!(!status.is_file_dirty("src/generated-other/file.rs"));
    }
}
