//! `mimir-server` — JSON-RPC server for Mimir context governance and session management.
//!
//! Exposes custom JSON-RPC methods for:
//! - Building context packets (`workspace/contextGovernor`)
//! - Listing provider capabilities (`workspace/providers`)
//! - Creating and managing sessions (`session/*`)

#![warn(missing_docs)]

pub mod lsp;
pub mod rpc;
pub mod session;
pub mod ui;

pub use lsp::MimirLspBackend;
pub use rpc::MimirServer;
pub use session::{Session, SessionId, SessionStore};
pub use ui::{run_ui_server, UiServerConfig, UiServerInfo};

/// Server configuration.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// TCP bind address (e.g. "127.0.0.1:8080").
    pub tcp_bind: Option<String>,
    /// Whether to use stdio transport.
    pub stdio: bool,
    /// UI HTTP/WebSocket bind address. Must be loopback.
    pub ui_bind: Option<String>,
    /// Optional UI auth token. If absent, a random token is generated.
    pub ui_token: Option<String>,
    /// Workspace root for UI APIs.
    pub workspace_root: Option<camino::Utf8PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            tcp_bind: None,
            stdio: true,
            ui_bind: None,
            ui_token: None,
            workspace_root: None,
        }
    }
}

/// Run the Mimir server with the given configuration.
///
/// If `tcp_bind` is set, listens on TCP. Otherwise uses stdio.
pub async fn run_server(config: ServerConfig) -> anyhow::Result<()> {
    if let Some(bind_addr) = config.ui_bind {
        tracing::info!("Starting Mimir UI server on HTTP/WebSocket: {}", bind_addr);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
        let workspace_root =
            match config.workspace_root {
                Some(root) => root,
                None => camino::Utf8PathBuf::from_path_buf(std::env::current_dir()?).map_err(
                    |path| anyhow::anyhow!("workspace root is not UTF-8: {}", path.display()),
                )?,
            };
        let ui_config = UiServerConfig::new(workspace_root, config.ui_token);
        let info = ui_config.info_for_listener(&listener)?;
        println!("Mimir Studio listening on {}", info.launch_url());
        println!("Mimir Studio API bound to {}", info.base_url);
        run_ui_server(listener, ui_config).await?;
        return Ok(());
    }

    let sessions = SessionStore::new();

    if let Some(bind_addr) = config.tcp_bind {
        tracing::info!("Starting Mimir server on TCP: {}", bind_addr);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
        run_tcp_server(listener, sessions).await?;
    } else {
        tracing::info!("Starting Mimir server on stdio");
        let (service, socket) =
            tower_lsp::LspService::new(|client| MimirLspBackend::new(client, sessions.clone()));
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        tower_lsp::Server::new(stdin, stdout, socket)
            .serve(service)
            .await;
    }

    Ok(())
}

/// Serve LSP clients from an already-bound TCP listener.
///
/// This is public so integration tests and embedders can bind port `0`, inspect
/// the assigned socket address, and then hand the listener to Mimir.
pub async fn run_tcp_server(
    listener: tokio::net::TcpListener,
    sessions: SessionStore,
) -> anyhow::Result<()> {
    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::info!("Client connected from: {:?}", addr);
        let sessions = sessions.clone();

        tokio::spawn(async move {
            let (service, socket) = tower_lsp::LspService::new(move |client| {
                MimirLspBackend::new(client, sessions.clone())
            });
            let (read, write) = tokio::io::split(stream);
            tower_lsp::Server::new(read, write, socket)
                .serve(service)
                .await;
        });
    }
}
