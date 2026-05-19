//! JSON-RPC handlers for the Mimir server.
//!
//! Implements custom methods for the Mimir server backend.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::session::{SessionId, SessionStore};
use mimir_context::ContextBuilder;
use mimir_providers::capabilities::ProviderCapabilities;
use mimir_schemas::ContextPacket;

/// Custom request: build context for a session.
pub const WORKSPACE_CONTEXT_GOVERNOR: &str = "workspace/contextGovernor";
/// Custom request: list provider capabilities.
pub const WORKSPACE_PROVIDERS: &str = "workspace/providers";
/// Custom request: create a new session.
pub const SESSION_CREATE: &str = "session/create";
/// Custom request: get session state.
pub const SESSION_GET: &str = "session/get";
/// Custom request: set provider/model on a session.
pub const SESSION_SET_PROVIDER: &str = "session/setProvider";
/// Custom request: build context packet in a session.
pub const SESSION_BUILD_CONTEXT: &str = "session/buildContext";

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// Request body for `workspace/contextGovernor`.
#[derive(Debug, Deserialize)]
pub struct ContextGovernorRequest {
    /// Task description.
    pub task: String,
    /// Optional provider override.
    #[serde(default)]
    pub provider: Option<String>,
    /// Optional model override.
    #[serde(default)]
    pub model: Option<String>,
}

/// Response for `workspace/contextGovernor`.
#[derive(Debug, Serialize)]
pub struct ContextGovernorResponse {
    /// Built context packet.
    pub packet: ContextPacket,
}

/// Request body for `session/setProvider`.
#[derive(Debug, Deserialize)]
pub struct SetProviderRequest {
    /// Session ID.
    pub session_id: String,
    /// Provider name.
    pub provider: String,
    /// Model name.
    pub model: String,
}

/// Request body for `session/buildContext`.
#[derive(Debug, Deserialize)]
pub struct SessionBuildContextRequest {
    /// Session ID.
    pub session_id: String,
    /// Task description.
    pub task: String,
}

/// Response for `session/buildContext`.
#[derive(Debug, Serialize)]
pub struct SessionBuildContextResponse {
    /// Session ID.
    pub session_id: String,
    /// Built context packet.
    pub packet: ContextPacket,
}

/// Response for `session/get`.
#[derive(Debug, Serialize)]
pub struct SessionGetResponse {
    /// Session ID.
    pub session_id: String,
    /// Provider (if set).
    pub provider: Option<String>,
    /// Model (if set).
    pub model: Option<String>,
    /// Whether a context packet is present.
    pub has_context_packet: bool,
    /// Number of history entries.
    pub history_len: usize,
}

// ---------------------------------------------------------------------------
// Server implementation
// ---------------------------------------------------------------------------

/// Mimir JSON-RPC server backend.
#[derive(Debug, Clone)]
pub struct MimirServer {
    sessions: SessionStore,
}

impl MimirServer {
    /// Create a new server instance.
    pub fn new(sessions: SessionStore) -> Self {
        Self { sessions }
    }

    /// Handle `workspace/contextGovernor`.
    pub fn handle_context_governor(
        &self,
        params: ContextGovernorRequest,
    ) -> anyhow::Result<ContextGovernorResponse> {
        let mut builder = ContextBuilder::new().task_card(&params.task);
        if let Some(p) = params.provider {
            builder = builder.provider(p);
        }
        if let Some(m) = params.model {
            builder = builder.model(m);
        }
        let packet = builder.build()?;
        Ok(ContextGovernorResponse { packet })
    }

    /// Handle `workspace/providers`.
    pub fn handle_providers(&self) -> ProviderCapabilities {
        let mut caps = ProviderCapabilities {
            schema_version: 1,
            provider: "mimir".to_string(),
            models: std::collections::HashMap::new(),
        };
        caps.models.insert(
            "claude-sonnet-4-20250514".to_string(),
            mimir_providers::capabilities::ModelCapabilities {
                max_context_tokens: 1_000_000,
                max_input_tokens: 1_000_000,
                max_output_tokens: 8_192,
                output_reserve_tokens: 1_024,
                counts_system_tokens: true,
                counts_tool_schemas: true,
                counts_tool_results: true,
                counts_reasoning_tokens: false,
                supports_server_token_count: true,
                supports_prompt_cache: true,
                overflow_behavior: "error".to_string(),
                pricing: mimir_providers::capabilities::Pricing {
                    input_per_million: 3.0,
                    output_per_million: 15.0,
                    cache_write_per_million: Some(3.75),
                    cache_read_per_million: Some(0.30),
                },
                count_drift_p95_observed: None,
            },
        );
        caps
    }

    /// Handle `session/create`.
    pub fn handle_session_create(&self) -> String {
        let id = self.sessions.create();
        id.to_string()
    }

    /// Handle `session/get`.
    pub fn handle_session_get(
        &self,
        session_id: String,
    ) -> anyhow::Result<SessionGetResponse> {
        let id = SessionId(session_id.clone());
        let session = self
            .sessions
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id))?;
        Ok(SessionGetResponse {
            session_id: session.id.to_string(),
            provider: session.provider.clone(),
            model: session.model.clone(),
            has_context_packet: session.context_packet.is_some(),
            history_len: session.history.len(),
        })
    }

    /// Handle `session/setProvider`.
    pub fn handle_session_set_provider(
        &self,
        params: SetProviderRequest,
    ) -> anyhow::Result<Value> {
        let id = SessionId(params.session_id.clone());
        let mut session = self
            .sessions
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", params.session_id))?;
        session.provider = Some(params.provider);
        session.model = Some(params.model);
        session.touch();
        self.sessions.update(session);
        Ok(serde_json::json!({"ok": true}))
    }

    /// Handle `session/buildContext`.
    pub fn handle_session_build_context(
        &self,
        params: SessionBuildContextRequest,
    ) -> anyhow::Result<SessionBuildContextResponse> {
        let id = SessionId(params.session_id.clone());
        let mut session = self
            .sessions
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", params.session_id))?;
        let mut builder = ContextBuilder::new().task_card(&params.task);
        if let Some(ref p) = session.provider {
            builder = builder.provider(p.clone());
        }
        if let Some(ref m) = session.model {
            builder = builder.model(m.clone());
        }
        let packet = builder.build()?;
        session.context_packet = Some(packet.clone());
        session.touch();
        self.sessions.update(session);
        Ok(SessionBuildContextResponse {
            session_id: params.session_id,
            packet,
        })
    }
}
