//! Override request flow with auto-grant after N failures.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{Result, ReviewError};

/// A request to override a check or policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrideRequest {
    /// Unique request ID.
    pub request_id: String,
    /// What is being overridden.
    pub target: String,
    /// Human-readable reason.
    pub reason: String,
    /// Requester (model, user, system).
    pub requester: String,
    /// Timestamp.
    pub requested_at: DateTime<Utc>,
    /// Number of prior failed attempts.
    pub prior_failures: u32,
    /// Auto-grant threshold.
    pub auto_grant_threshold: u32,
    /// Whether this was auto-granted.
    pub auto_granted: bool,
    /// Whether approved.
    pub approved: Option<bool>,
}

/// Override audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverrideAuditEntry {
    pub request_id: String,
    pub target: String,
    pub requester: String,
    pub requested_at: DateTime<Utc>,
    pub approved: bool,
    pub auto_granted: bool,
    pub reason: String,
}

/// Manages override requests and audit logging.
#[derive(Default)]
pub struct OverrideManager {
    /// Pending and resolved requests.
    pub requests: HashMap<String, OverrideRequest>,
    /// Audit log (append-only).
    pub audit_log: Vec<OverrideAuditEntry>,
    /// Default auto-grant threshold.
    pub default_threshold: u32,
}

impl OverrideManager {
    pub fn new() -> Self {
        Self {
            requests: HashMap::new(),
            audit_log: Vec::new(),
            default_threshold: 3,
        }
    }

    /// Submit a request with prior failure count.
    pub fn request_with_failures(
        &mut self,
        target: impl Into<String>,
        reason: impl Into<String>,
        requester: impl Into<String>,
        prior_failures: u32,
    ) -> Result<String> {
        let request_id = format!("ovr-{}", uuid::Uuid::new_v4());
        let mut req = OverrideRequest {
            request_id: request_id.clone(),
            target: target.into(),
            reason: reason.into(),
            requester: requester.into(),
            requested_at: Utc::now(),
            prior_failures,
            auto_grant_threshold: self.default_threshold,
            auto_granted: false,
            approved: None,
        };

        // Auto-grant if failures exceed threshold
        if prior_failures >= self.default_threshold {
            req.auto_granted = true;
            req.approved = Some(true);
            self.audit_log.push(OverrideAuditEntry {
                request_id: req.request_id.clone(),
                target: req.target.clone(),
                requester: req.requester.clone(),
                requested_at: req.requested_at,
                approved: true,
                auto_granted: true,
                reason: req.reason.clone(),
            });
        }

        self.requests.insert(request_id.clone(), req);
        Ok(request_id)
    }

    /// Approve or deny a request.
    pub fn resolve(&mut self, request_id: &str, approved: bool) -> Result<()> {
        let req = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| ReviewError::OverrideDenied {
                request_id: request_id.to_string(),
            })?;
        req.approved = Some(approved);
        self.audit_log.push(OverrideAuditEntry {
            request_id: req.request_id.clone(),
            target: req.target.clone(),
            requester: req.requester.clone(),
            requested_at: req.requested_at,
            approved,
            auto_granted: req.auto_granted,
            reason: req.reason.clone(),
        });
        Ok(())
    }

    /// Get a request by ID.
    pub fn get(&self, request_id: &str) -> Option<&OverrideRequest> {
        self.requests.get(request_id)
    }

    /// Print a structured table of pending requests.
    pub fn print_pending(&self) {
        println!("┌─────────────────────────────────────────────────────────────┐");
        println!("│ Pending Override Requests                                   │");
        println!("├──────────────┬──────────────────────┬──────────┬────────────┤");
        println!("│ Request ID   │ Target               │ Failures │ Auto-Grant │");
        println!("├──────────────┼──────────────────────┼──────────┼────────────┤");
        for req in self.requests.values().filter(|r| r.approved.is_none()) {
            let auto = if req.prior_failures >= req.auto_grant_threshold {
                "YES*"
            } else {
                "no"
            };
            println!(
                "│ {:12} │ {:20} │ {:8} │ {:10} │",
                &req.request_id[..req.request_id.len().min(12)],
                &req.target[..req.target.len().min(20)],
                req.prior_failures,
                auto
            );
        }
        println!("└──────────────┴──────────────────────┴──────────┴────────────┘");
    }

    /// Save audit log to JSON.
    pub fn save_audit(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.audit_log)?;
        std::fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_override_auto_grant() {
        let mut mgr = OverrideManager::new();
        let id = mgr
            .request_with_failures("large-context", "Need 200K tokens for review", "model", 3)
            .unwrap();
        let req = mgr.get(&id).unwrap();
        assert!(req.auto_granted);
        assert_eq!(req.approved, Some(true));
        assert_eq!(mgr.audit_log.len(), 1);
    }

    #[test]
    fn test_override_manual_approval() {
        let mut mgr = OverrideManager::new();
        let id = mgr
            .request_with_failures("large-context", "Need 200K tokens", "model", 1)
            .unwrap();
        let req = mgr.get(&id).unwrap();
        assert!(!req.auto_granted);
        assert_eq!(req.approved, None);

        mgr.resolve(&id, true).unwrap();
        let resolved = mgr.get(&id).unwrap();
        assert_eq!(resolved.approved, Some(true));
        assert_eq!(mgr.audit_log.len(), 1);
    }

    #[test]
    fn test_override_denied() {
        let mut mgr = OverrideManager::new();
        let id = mgr
            .request_with_failures("large-context", "Need 200K tokens", "model", 1)
            .unwrap();
        mgr.resolve(&id, false).unwrap();
        let resolved = mgr.get(&id).unwrap();
        assert_eq!(resolved.approved, Some(false));
    }
}
