//! LSP transport implementation for the Mimir server.
//!
//! Implements the `tower_lsp::LanguageServer` trait to provide:
//! - Standard LSP lifecycle (initialize, initialized, shutdown)
//! - Custom commands via `workspace/executeCommand`

use std::borrow::Cow;

use tower_lsp::jsonrpc::{Error as RpcError, ErrorCode as RpcErrorCode, Result as RpcResult};
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};
use tracing::{info, warn};

use crate::rpc::{
    ContextGovernorRequest, MimirServer, SessionBuildContextRequest, SetProviderRequest,
    SESSION_BUILD_CONTEXT, SESSION_CREATE, SESSION_GET, SESSION_SET_PROVIDER,
    WORKSPACE_CONTEXT_GOVERNOR, WORKSPACE_PROVIDERS,
};
use crate::session::SessionStore;

/// Build an internal error with a message.
fn internal_err(msg: String) -> RpcError {
    RpcError {
        code: RpcErrorCode::InternalError,
        message: Cow::Owned(msg),
        data: None,
    }
}

/// LSP backend that wraps the Mimir JSON-RPC server.
#[derive(Debug, Clone)]
pub struct MimirLspBackend {
    client: Client,
    inner: MimirServer,
}

impl MimirLspBackend {
    /// Create a new LSP backend.
    pub fn new(client: Client, sessions: SessionStore) -> Self {
        Self {
            client,
            inner: MimirServer::new(sessions),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for MimirLspBackend {
    async fn initialize(&self, _: InitializeParams) -> RpcResult<InitializeResult> {
        info!("LSP initialize");
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        WORKSPACE_CONTEXT_GOVERNOR.to_string(),
                        WORKSPACE_PROVIDERS.to_string(),
                        SESSION_CREATE.to_string(),
                        SESSION_GET.to_string(),
                        SESSION_SET_PROVIDER.to_string(),
                        SESSION_BUILD_CONTEXT.to_string(),
                    ],
                    work_done_progress_options: WorkDoneProgressOptions {
                        work_done_progress: None,
                    },
                }),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "mimir-server".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        info!("LSP initialized");
        self.client
            .log_message(MessageType::INFO, "Mimir server initialized")
            .await;
    }

    async fn shutdown(&self) -> RpcResult<()> {
        info!("LSP shutdown");
        Ok(())
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> RpcResult<Option<serde_json::Value>> {
        info!(command = %params.command, "execute_command");

        match params.command.as_str() {
            WORKSPACE_CONTEXT_GOVERNOR => {
                let args = params
                    .arguments
                    .into_iter()
                    .next()
                    .ok_or_else(|| RpcError::invalid_params("missing request body"))?;
                let req: ContextGovernorRequest = serde_json::from_value(args)
                    .map_err(|e| RpcError::invalid_params(format!("invalid request: {}", e)))?;
                let resp = self
                    .inner
                    .handle_context_governor(req)
                    .map_err(|e| internal_err(e.to_string()))?;
                Ok(Some(
                    serde_json::to_value(resp).map_err(|e| internal_err(e.to_string()))?,
                ))
            }
            WORKSPACE_PROVIDERS => {
                let caps = self
                    .inner
                    .handle_providers()
                    .map_err(|e| internal_err(e.to_string()))?;
                Ok(Some(
                    serde_json::to_value(caps).map_err(|e| internal_err(e.to_string()))?,
                ))
            }
            SESSION_CREATE => {
                let id = self.inner.handle_session_create();
                Ok(Some(serde_json::json!({"session_id": id})))
            }
            SESSION_GET => {
                let args = params
                    .arguments
                    .into_iter()
                    .next()
                    .ok_or_else(|| RpcError::invalid_params("missing session_id"))?;
                let session_id: String = serde_json::from_value(args)
                    .map_err(|e| RpcError::invalid_params(format!("invalid session_id: {}", e)))?;
                let resp = self
                    .inner
                    .handle_session_get(session_id)
                    .map_err(|e| internal_err(e.to_string()))?;
                Ok(Some(
                    serde_json::to_value(resp).map_err(|e| internal_err(e.to_string()))?,
                ))
            }
            SESSION_SET_PROVIDER => {
                let args = params
                    .arguments
                    .into_iter()
                    .next()
                    .ok_or_else(|| RpcError::invalid_params("missing request body"))?;
                let req: SetProviderRequest = serde_json::from_value(args)
                    .map_err(|e| RpcError::invalid_params(format!("invalid request: {}", e)))?;
                let resp = self
                    .inner
                    .handle_session_set_provider(req)
                    .map_err(|e| internal_err(e.to_string()))?;
                Ok(Some(resp))
            }
            SESSION_BUILD_CONTEXT => {
                let args = params
                    .arguments
                    .into_iter()
                    .next()
                    .ok_or_else(|| RpcError::invalid_params("missing request body"))?;
                let req: SessionBuildContextRequest = serde_json::from_value(args)
                    .map_err(|e| RpcError::invalid_params(format!("invalid request: {}", e)))?;
                let resp = self
                    .inner
                    .handle_session_build_context(req)
                    .map_err(|e| internal_err(e.to_string()))?;
                Ok(Some(
                    serde_json::to_value(resp).map_err(|e| internal_err(e.to_string()))?,
                ))
            }
            _ => {
                warn!(command = %params.command, "Unknown command");
                Err(RpcError::method_not_found())
            }
        }
    }
}
