//! Override audit log persistence.

use std::fs;
use std::path::Path;

use crate::override_req::OverrideAuditEntry;

/// Persist audit log to disk.
pub fn save_audit_log(entries: &[OverrideAuditEntry], path: &Path) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(entries)?;
    fs::write(path, json)
}

/// Load audit log from disk.
pub fn load_audit_log(path: &Path) -> std::io::Result<Vec<OverrideAuditEntry>> {
    let content = fs::read_to_string(path)?;
    let entries: Vec<OverrideAuditEntry> = serde_json::from_str(&content)?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    #[test]
    fn test_save_load_audit_log() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("audit.json");
        let entries = vec![OverrideAuditEntry {
            request_id: "ovr-1".into(),
            target: "large-context".into(),
            requester: "model".into(),
            requested_at: Utc::now(),
            approved: true,
            auto_granted: false,
            reason: "needed".into(),
        }];
        save_audit_log(&entries, &path).unwrap();
        let loaded = load_audit_log(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].request_id, "ovr-1");
    }
}
