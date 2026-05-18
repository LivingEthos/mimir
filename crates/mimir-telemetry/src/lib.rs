//! `mimir-telemetry` — OpenTelemetry-shaped spans and audit events.
//!
//! Emits structured telemetry for every command, gateway call, and retrieval run.
//! Never exports over the network by default.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

/// A trace span emitted by the telemetry system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSpan {
    /// Span name (e.g., `mimir.gateway.call`).
    pub name: String,
    /// Timestamp in RFC 3339.
    pub started_at: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Span attributes.
    pub attributes: serde_json::Value,
}

/// An audit event emitted for security-relevant operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Event kind (e.g., `override_granted`, `provider_call`).
    pub kind: String,
    /// Timestamp in RFC 3339.
    pub timestamp: String,
    /// Event payload.
    pub payload: serde_json::Value,
}
