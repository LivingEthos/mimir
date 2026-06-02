//! `mimir-telemetry` — OpenTelemetry-shaped spans and audit events.
//!
//! Emits structured telemetry for every command, gateway call, and retrieval run.
//! Never exports over the network by default.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

/// Current trace span schema version.
pub const TRACE_SPAN_SCHEMA_VERSION: u32 = 1;

/// A trace span emitted by the telemetry system.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSpan {
    /// Trace span schema version.
    pub schema_version: u32,
    /// Hex-encoded span identifier.
    pub span_id: String,
    /// Hex-encoded trace identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Parent span identifier, when this span has a parent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Span name, for example `mimir.gateway.call`.
    pub name: String,
    /// OpenTelemetry span kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<TraceSpanKind>,
    /// Unix microseconds when the span started.
    pub start_us: u64,
    /// Unix microseconds when the span ended.
    pub end_us: u64,
    /// Span attributes. Callers must avoid secrets; run writers redact again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<serde_json::Value>,
    /// Point-in-time events attached to this span.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<TraceSpanEvent>>,
    /// Span completion status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TraceSpanStatus>,
}

/// OpenTelemetry span kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceSpanKind {
    /// Internal operation span.
    Internal,
    /// Outbound client span.
    Client,
    /// Inbound server span.
    Server,
    /// Producer span.
    Producer,
    /// Consumer span.
    Consumer,
}

/// Event attached to a trace span.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSpanEvent {
    /// Unix microseconds when the event occurred.
    pub at_us: u64,
    /// Event name.
    pub name: String,
    /// Event attributes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<serde_json::Value>,
}

/// Completion status attached to a trace span.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceSpanStatus {
    /// Status code.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<TraceSpanStatusCode>,
    /// Optional redacted status message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Trace span status code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceSpanStatusCode {
    /// Status has not been set.
    Unset,
    /// Span completed successfully.
    Ok,
    /// Span completed with an error.
    Error,
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
            schema_version: TRACE_SPAN_SCHEMA_VERSION,
            span_id: "0123456789abcdef".to_string(),
            trace_id: Some("0123456789abcdef0123456789abcdef".to_string()),
            parent_id: None,
            name: "test.span".to_string(),
            kind: Some(TraceSpanKind::Internal),
            start_us: 1_704_067_200_000_000,
            end_us: 1_704_067_200_000_100,
            attrs: Some(serde_json::json!({"key": "value"})),
            events: Some(vec![TraceSpanEvent {
                at_us: 1_704_067_200_000_050,
                name: "checkpoint".to_string(),
                attrs: None,
            }]),
            status: Some(TraceSpanStatus {
                code: Some(TraceSpanStatusCode::Ok),
                message: None,
            }),
        };
        let json = serde_json::to_string(&span).unwrap();
        assert!(json.contains("test.span"));
        assert!(json.contains("start_us"));
        assert!(json.contains("checkpoint"));
        let value = serde_json::to_value(&span).unwrap();
        assert_trace_span_schema_valid(&value);
        assert!(value.get("started_at").is_none());
        assert!(value.get("duration_ms").is_none());
        assert!(value.get("attributes").is_none());
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
            schema_version: 1,
            span_id: "fedcba9876543210".to_string(),
            trace_id: Some("fedcba9876543210fedcba9876543210".to_string()),
            parent_id: None,
            name: "mimir.gateway.call".to_string(),
            kind: Some(TraceSpanKind::Client),
            start_us: 1_717_244_800_000_000,
            end_us: 1_717_244_800_000_042,
            attrs: Some(serde_json::json!({"model": "claude"})),
            events: None,
            status: Some(TraceSpanStatus {
                code: Some(TraceSpanStatusCode::Ok),
                message: None,
            }),
        };
        let json = serde_json::to_string(&span).unwrap();
        let decoded: TraceSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, "mimir.gateway.call");
        assert_eq!(decoded.end_us - decoded.start_us, 42);
    }

    fn assert_trace_span_schema_valid(value: &serde_json::Value) {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schemas/TraceSpan.schema.json")).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors = validator
            .iter_errors(value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "schema validation failed: {errors:#?}");
    }
}
