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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_span_serialize() {
        let span = TraceSpan {
            name: "test.span".to_string(),
            started_at: "2024-01-01T00:00:00Z".to_string(),
            duration_ms: 100,
            attributes: serde_json::json!({"key": "value"}),
        };
        let json = serde_json::to_string(&span).unwrap();
        assert!(json.contains("test.span"));
        assert!(json.contains("100"));
    }

    #[test]
    fn test_audit_event_serialize() {
        let event = AuditEvent {
            kind: "override_granted".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            payload: serde_json::json!({"reason": "test"}),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("override_granted"));
        assert!(json.contains("test"));
    }

    #[test]
    fn test_trace_span_roundtrip() {
        let span = TraceSpan {
            name: "mimir.gateway.call".to_string(),
            started_at: "2024-06-01T12:00:00Z".to_string(),
            duration_ms: 42,
            attributes: serde_json::json!({"model": "claude"}),
        };
        let json = serde_json::to_string(&span).unwrap();
        let decoded: TraceSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, "mimir.gateway.call");
        assert_eq!(decoded.duration_ms, 42);
    }
}
