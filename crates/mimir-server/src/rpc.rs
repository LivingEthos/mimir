//! JSON-RPC handlers for the Mimir server.
//!
//! Implements custom methods for the Mimir server backend.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::session::{SessionId, SessionStore};
use mimir_providers::capabilities::ProviderCapabilitiesList;
use mimir_schemas::ContextPacket;
use mimir_session::{
    build_context_packet as build_session_context_packet, ContextPacketBuildRequest,
};

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
    workspace_root: Arc<RwLock<Option<PathBuf>>>,
}

impl MimirServer {
    /// Create a new server instance.
    pub fn new(sessions: SessionStore) -> Self {
        Self {
            sessions,
            workspace_root: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the workspace root used for retrieval-backed context packets.
    pub fn set_workspace_root(&self, root: Option<PathBuf>) {
        if let Ok(mut workspace_root) = self.workspace_root.write() {
            *workspace_root = root;
        }
    }

    fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root
            .read()
            .ok()
            .and_then(|root| root.as_ref().cloned())
    }

    fn workspace_root_utf8(&self) -> anyhow::Result<Option<Utf8PathBuf>> {
        self.workspace_root()
            .map(|root| {
                Utf8PathBuf::from_path_buf(root).map_err(|path| {
                    anyhow::anyhow!("workspace root is not UTF-8: {}", path.display())
                })
            })
            .transpose()
    }

    fn build_context_packet(
        &self,
        task: String,
        provider: Option<String>,
        model: Option<String>,
    ) -> anyhow::Result<ContextPacket> {
        let workspace_root = self.workspace_root_utf8()?;
        let persist = workspace_root.is_some();
        let built = build_session_context_packet(ContextPacketBuildRequest {
            task,
            mode: "ask".to_string(),
            provider,
            model,
            workspace_root,
            mimir_root: None,
            edit_targets: Vec::new(),
            persist,
        })?;
        Ok(built.packet)
    }

    /// Handle `workspace/contextGovernor`.
    pub fn handle_context_governor(
        &self,
        params: ContextGovernorRequest,
    ) -> anyhow::Result<ContextGovernorResponse> {
        let packet = self.build_context_packet(params.task, params.provider, params.model)?;
        Ok(ContextGovernorResponse { packet })
    }

    /// Handle `workspace/providers`.
    pub fn handle_providers(&self) -> anyhow::Result<ProviderCapabilitiesList> {
        mimir_providers::capabilities::provider_capabilities_list().map_err(anyhow::Error::msg)
    }

    /// Handle `session/create`.
    pub fn handle_session_create(&self) -> String {
        let id = self.sessions.create();
        id.to_string()
    }

    /// Handle `session/get`.
    pub fn handle_session_get(&self, session_id: String) -> anyhow::Result<SessionGetResponse> {
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
    pub fn handle_session_set_provider(&self, params: SetProviderRequest) -> anyhow::Result<Value> {
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
        let packet = self.build_context_packet(
            params.task,
            session.provider.clone(),
            session.model.clone(),
        )?;
        session.context_packet = Some(packet.clone());
        session.touch();
        self.sessions.update(session);
        Ok(SessionBuildContextResponse {
            session_id: params.session_id,
            packet,
        })
    }
}
