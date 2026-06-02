//! Minimal LSP client used by the TUI live refresh path.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use mimir_schemas::ContextPacket;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Live server configuration for TUI refreshes.
#[derive(Debug, Clone)]
pub struct LiveServerConfig {
    /// TCP address, for example `127.0.0.1:7788`.
    pub address: String,
    /// Task used to ask the server for a fresh context packet.
    pub task: String,
    /// Optional provider to set on the live session.
    pub provider: Option<String>,
    /// Optional model to set on the live session.
    pub model: Option<String>,
    /// Optional existing server session id.
    pub session_id: Option<String>,
    /// Workspace root to pass in the LSP initialize request.
    pub workspace_root: Option<PathBuf>,
    /// Per-connection and per-request timeout.
    pub timeout: Duration,
    /// Optional automatic refresh interval.
    pub refresh_interval: Option<Duration>,
}

impl LiveServerConfig {
    /// Create live refresh configuration with conservative defaults.
    #[must_use]
    pub fn new(address: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            task: task.into(),
            provider: None,
            model: None,
            session_id: None,
            workspace_root: None,
            timeout: Duration::from_secs(5),
            refresh_interval: None,
        }
    }
}

/// Result of a live refresh.
#[derive(Debug)]
pub struct LivePacket {
    /// Server session id used for this packet.
    pub session_id: String,
    /// Fresh context packet returned by the server.
    pub packet: ContextPacket,
}

struct LspClient {
    stream: TcpStream,
    next_id: u64,
    timeout: Duration,
}

/// Fetch a context packet from a running `mimir-server` TCP listener.
///
/// # Errors
/// Returns an error when the server cannot be reached, LSP framing fails, or
/// the server returns a JSON-RPC error.
pub async fn fetch_live_packet(config: &LiveServerConfig) -> anyhow::Result<LivePacket> {
    let stream = timeout(config.timeout, TcpStream::connect(&config.address))
        .await
        .context("timed out connecting to live server")??;
    let mut client = LspClient {
        stream,
        next_id: 1,
        timeout: config.timeout,
    };

    let root_uri = config
        .workspace_root
        .as_ref()
        .map(|root| file_uri_from_path(root.as_path()));

    client
        .request(
            "initialize",
            serde_json::json!({
                "processId": null,
                "rootUri": root_uri,
                "capabilities": {}
            }),
        )
        .await?;
    client
        .notify("initialized", serde_json::json!({}))
        .await
        .context("failed to send initialized notification")?;

    let session_id = match &config.session_id {
        Some(session_id) => session_id.clone(),
        None => create_live_session(&mut client).await?,
    };

    match build_live_packet(&mut client, config, &session_id).await {
        Ok(packet) => Ok(LivePacket { session_id, packet }),
        Err(error) if config.session_id.is_some() && is_session_not_found_error(&error) => {
            let fresh_session_id = create_live_session(&mut client).await?;
            let packet = build_live_packet(&mut client, config, &fresh_session_id)
                .await
                .context("failed to build live context packet with fresh session")?;
            Ok(LivePacket {
                session_id: fresh_session_id,
                packet,
            })
        }
        Err(error) => Err(error),
    }
}

async fn create_live_session(client: &mut LspClient) -> anyhow::Result<String> {
    let value = client
        .execute_command("session/create", Vec::new())
        .await
        .context("failed to create live server session")?;
    value["session_id"]
        .as_str()
        .ok_or_else(|| anyhow!("session/create response missing session_id"))
        .map(str::to_string)
}

async fn build_live_packet(
    client: &mut LspClient,
    config: &LiveServerConfig,
    session_id: &str,
) -> anyhow::Result<ContextPacket> {
    if let (Some(provider), Some(model)) = (&config.provider, &config.model) {
        client
            .execute_command(
                "session/setProvider",
                vec![serde_json::json!({
                    "session_id": session_id,
                    "provider": provider,
                    "model": model,
                })],
            )
            .await
            .context("failed to set live server provider")?;
    }

    let response = client
        .execute_command(
            "session/buildContext",
            vec![serde_json::json!({
                "session_id": session_id,
                "task": &config.task,
            })],
        )
        .await
        .context("failed to build live context packet")?;
    serde_json::from_value(response["packet"].clone())
        .context("session/buildContext response missing valid packet")
}

fn is_session_not_found_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("session not found")
            || message.contains("session-not-found")
            || message.contains("session_not_found")
    })
}

fn file_uri_from_path(path: &std::path::Path) -> String {
    let path = path.to_string_lossy();
    let mut encoded = String::new();
    for byte in path.as_bytes() {
        let ch = *byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | '~' | ':') {
            encoded.push(ch);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("file://{encoded}")
}

impl LspClient {
    async fn execute_command(
        &mut self,
        command: &str,
        arguments: Vec<Value>,
    ) -> anyhow::Result<Value> {
        self.request(
            "workspace/executeCommand",
            serde_json::json!({
                "command": command,
                "arguments": arguments,
            }),
        )
        .await
    }

    async fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write_message(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;

        loop {
            let message = self.read_message().await?;
            if message["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                bail!("server returned error for {method}: {error}");
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> anyhow::Result<()> {
        self.write_message(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    async fn write_message(&mut self, message: &Value) -> anyhow::Result<()> {
        let body = serde_json::to_string(message)?;
        let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        timeout(self.timeout, self.stream.write_all(frame.as_bytes()))
            .await
            .context("timed out writing LSP frame")??;
        Ok(())
    }

    async fn read_message(&mut self) -> anyhow::Result<Value> {
        let mut header = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            timeout(self.timeout, self.stream.read_exact(&mut byte))
                .await
                .context("timed out reading LSP header")??;
            header.push(byte[0]);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
        }

        let header = std::str::from_utf8(&header)?;
        let content_length = header
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .ok_or_else(|| anyhow!("LSP frame missing Content-Length"))?
            .parse::<usize>()?;
        let mut body = vec![0u8; content_length];
        timeout(self.timeout, self.stream.read_exact(&mut body))
            .await
            .context("timed out reading LSP body")??;
        Ok(serde_json::from_slice(&body)?)
    }
}
