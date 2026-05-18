//! Backup-before-mutation for non-git files.

use std::fs;
use std::path::Path;

use camino::Utf8Path;

use crate::{EditError, Result};

/// Manages backups before file mutations.
pub struct BackupManager {
    backup_dir: String,
}

impl BackupManager {
    /// Create a new backup manager with a backup directory.
    pub fn new(backup_dir: impl Into<String>) -> Self {
        Self {
            backup_dir: backup_dir.into(),
        }
    }

    /// Create a backup of a file before mutation.
    pub fn backup(&self, base: &Utf8Path, path: &str) -> Result<()> {
        let src = base.join(path);
        if !src.exists() {
            // Nothing to backup (e.g. create operation)
            return Ok(());
        }

        let backup_root = base.join(&self.backup_dir);
        fs::create_dir_all(&backup_root)
            .map_err(|e| EditError::BackupFailed {
                path: path.to_string(),
                reason: format!("create backup dir: {}", e),
            })?;

        // Flatten path into backup filename
        let backup_name = path.replace('/', "__");
        let dst = backup_root.join(backup_name);

        fs::copy(&src, &dst).map_err(|e| EditError::BackupFailed {
            path: path.to_string(),
            reason: format!("copy: {}", e),
        })?;

        Ok(())
    }

    /// Restore a file from backup.
    pub fn restore(&self, base: &Utf8Path, path: &str) -> Result<()> {
        let backup_name = path.replace('/', "__");
        let src = base.join(&self.backup_dir).join(&backup_name);
        let dst = base.join(path);

        if !src.exists() {
            return Err(EditError::BackupFailed {
                path: path.to_string(),
                reason: "backup not found".to_string(),
            });
        }

        fs::copy(&src, &dst).map_err(|e| EditError::BackupFailed {
            path: path.to_string(),
            reason: format!("restore copy: {}", e),
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_backup_and_restore() {
        let dir = TempDir::new().unwrap();
        let base = Utf8Path::from_path(dir.path()).unwrap();
        let file_path = dir.path().join("test.txt");
        {
            let mut f = fs::File::create(&file_path).unwrap();
            f.write_all(b"original").unwrap();
        }

        let mgr = BackupManager::new(".mimir/backups");
        mgr.backup(base, "test.txt").unwrap();

        // Mutate
        fs::write(&file_path, "modified").unwrap();
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "modified");

        // Restore
        mgr.restore(base, "test.txt").unwrap();
        assert_eq!(fs::read_to_string(&file_path).unwrap(), "original");
    }
}
