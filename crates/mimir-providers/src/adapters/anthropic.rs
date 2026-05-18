//! Anthropic provider adapter.
//!
//! Implements [`ProviderAdapter`] for the Anthropic Messages API.
//!
//! Endpoints:
//! - `POST /v1/messages` — send a conversation and receive a response.
//! - `POST /v1/messages/count_tokens` — server-side token count.
//!
//! Headers:
//! - `x-api-key: <key>`
//! - `anthropic-version: 2023-06-01`
//! - `content-type: application/json`
//!
//! Retry policy (from `mimir-providers::retry`):
//! - Retry on HTTP 429 / 529 / 5xx / 408.
//! - No retry on 400 / 401 / 403 / 404 / 413.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;

use crate::capabilities::ProviderCapabilities;
use crate::count;
use crate::error::{map_anthropic_error, map_http_status, ProviderError, Result};
use crate::retry::{backoff, RetryPolicy};
use crate::types::{ProviderRequest, ProviderResponse, ResponseBlock, TokenUsage};

/// Anthropic adapter configuration.
pub struct AnthropicAdapter {
    client: reqwest::Client,
    capabilities: ProviderCapabilities,
    base_url: String,
    api_key: SecretString,
    api_version: String,
    retry_policy: RetryPolicy,
}

impl AnthropicAdapter {
    /// Create a new Anthropic adapter.
    ///
    /// The API key is read from the `ANTHROPIC_API_KEY` environment variable.
    pub fn new() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map(|s| SecretString::new(s.into_boxed_str()))
            .map_err(|_| ProviderError::new("provider_unauthorized", "ANTHROPIC_API_KEY not set"))?;

        Ok(Self {
            client: reqwest::Client::new(),
            capabilities: default_capabilities(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key,
            api_version: "2023-06-01".to_string(),
            retry_policy: RetryPolicy::default(),
        })
    }

    /// Create a new Anthropic adapter with an explicit API key.
    pub fn with_key(api_key: SecretString) -> Self {
        Self {
            client: reqwest::Client::new(),
            capabilities: default_capabilities(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key,
            api_version: "2023-06-01".to_string(),
            retry_policy: RetryPolicy::default(),
        }
    }

    /// Set a custom base URL (useful for testing / proxies).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Set a custom retry policy.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Build the standard header set for Anthropic requests.
    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(self.api_key.expose_secret()).expect("valid header value"),
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static("2023-06-01"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers
    }

    /// Redact secrets from a string (e.g. error messages that may contain the request body).
    fn redact(&self, text: &str) -> String {
        // Simple redaction: replace the exposed API key with a placeholder.
        // In production this should use mimir_security::redact_secrets.
        text.replace(self.api_key.expose_secret(), "[REDACTED_API_KEY]")
    }

    /// Translate a provider-neutral request into Anthropic's JSON shape.
    fn build_body(&self, request: &ProviderRequest) -> serde_json::Value {
        let mut body = json!({
            "model": request.model,
            "messages": request.messages.iter().map(|m| json!({"role": &m.role, "content": &m.content})).collect::<Vec<_>>(),
        });

        if let Some(system) = &request.system {
            body["system"] = json!(system);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if let Some(temperature) = request.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(stream) = request.stream {
            body["stream"] = json!(stream);
        }
        if let Some(stop) = &request.stop_sequences {
            body["stop_sequences"] = json!(stop);
        }
        if let Some(tools) = &request.tools {
            let tools_json: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    let mut obj = json!({
                        "name": t.name,
                        "input_schema": t.parameters,
                    });
                    if let Some(desc) = &t.description {
                        obj["description"] = json!(desc);
                    }
                    obj
                })
                .collect();
            body["tools"] = json!(tools_json);
        }
        if let Some(extra) = &request.extra {
            for (k, v) in extra {
                body[k] = v.clone();
            }
        }

        body
    }

    /// Execute an HTTP request with retry logic.
    async fn request_with_retry(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{}", self.base_url, path);
        let headers = self.headers();

        for attempt in 0..self.retry_policy.max_attempts {
            let mut req = self.client.request(method.clone(), &url);
            req = req.headers(headers.clone()).timeout(Duration::from_secs(120));
            if let Some(ref b) = body {
                req = req.json(b);
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    // Retryable statuses: 429, 529, 5xx, 408
                    let should_retry = matches!(status, 429 | 529 | 408 | 500..=599);
                    if !should_retry || attempt == self.retry_policy.max_attempts - 1 {
                        return Ok(resp);
                    }
                    let delay = backoff(&self.retry_policy, attempt);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    let msg = self.redact(&e.to_string());
                    // Network errors are retryable.
                    if attempt == self.retry_policy.max_attempts - 1 {
                        return Err(ProviderError::new(
                            "provider_connection_reset",
                            &format!("request failed after {} attempts: {}", self.retry_policy.max_attempts, msg),
                        )
                        .retryable());
                    }
                    let delay = backoff(&self.retry_policy, attempt);
                    tokio::time::sleep(delay).await;
                }
            }
        }

        // Unreachable — loop always returns inside.
        Err(ProviderError::new("provider_internal_error", "retry loop exhausted"))
    }

    /// Parse an Anthropic error response body into a [`ProviderError`].
    async fn parse_error(&self, resp: reqwest::Response) -> ProviderError {
        let status = resp.status().as_u16();
        let body_text = resp.text().await.unwrap_or_default();
        let redacted = self.redact(&body_text);

        #[derive(Deserialize)]
        struct AnthropicErrorBody {
            #[serde(rename = "type")]
            _type: String,
            error: AnthropicErrorDetail,
        }

        #[derive(Deserialize)]
        struct AnthropicErrorDetail {
            #[serde(rename = "type")]
            error_type: String,
            message: String,
        }

        if let Ok(parsed) = serde_json::from_str::<AnthropicErrorBody>(&body_text) {
            map_anthropic_error(&parsed.error.error_type, &parsed.error.message, status)
        } else {
            map_http_status(status, &redacted)
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderAdapter trait
// ---------------------------------------------------------------------------

/// Trait implemented by all provider adapters.
pub trait ProviderAdapter: Send + Sync {
    /// Adapter name.
    fn name(&self) -> &str;
    /// Current capabilities snapshot.
    fn capabilities(&self) -> &ProviderCapabilities;
    /// Local token count (fast, no network).
    fn count_local(&self, request: &ProviderRequest) -> Result<u32>;
    /// Server-side token count (network I/O).
    async fn count_server(&self, request: &ProviderRequest) -> Result<u32>;
    /// Dispatch a request and return the response.
    async fn call(&self, request: ProviderRequest) -> Result<ProviderResponse>;
}

impl ProviderAdapter for AnthropicAdapter {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    fn count_local(&self, request: &ProviderRequest) -> Result<u32> {
        // Phase 1: rough word-based estimate.
        // Concatenate system + all message contents and apply the local estimator.
        let mut text = String::new();
        if let Some(sys) = &request.system {
            text.push_str(sys);
            text.push(' ');
        }
        for msg in &request.messages {
            text.push_str(&msg.content);
            text.push(' ');
        }
        Ok(count::count_local(&text))
    }

    async fn count_server(&self, request: &ProviderRequest) -> Result<u32> {
        let body = self.build_body(request);
        let resp = self
            .request_with_retry(reqwest::Method::POST, "/v1/messages/count_tokens", Some(body))
            .await?;

        if !resp.status().is_success() {
            return Err(self.parse_error(resp).await);
        }

        #[derive(Deserialize)]
        struct CountTokensResponse {
            input_tokens: u32,
        }

        let parsed: CountTokensResponse = resp
            .json()
            .await
            .map_err(|e| ProviderError::new("provider_malformed_response", &self.redact(&e.to_string())))?;

        Ok(parsed.input_tokens)
    }

    async fn call(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        let body = self.build_body(&request);
        let resp = self
            .request_with_retry(reqwest::Method::POST, "/v1/messages", Some(body))
            .await?;

        if !resp.status().is_success() {
            return Err(self.parse_error(resp).await);
        }

        #[derive(Deserialize)]
        struct AnthropicMessage {
            id: String,
            #[serde(rename = "type")]
            _type: String,
            role: String,
            content: Vec<AnthropicContentBlock>,
            #[serde(rename = "stop_reason")]
            stop_reason: Option<String>,
            usage: AnthropicUsage,
            model: String,
        }

        #[derive(Deserialize)]
        struct AnthropicContentBlock {
            #[serde(rename = "type")]
            block_type: String,
            text: Option<String>,
            id: Option<String>,
            name: Option<String>,
            input: Option<serde_json::Value>,
        }

        #[derive(Deserialize)]
        struct AnthropicUsage {
            input_tokens: u32,
            output_tokens: u32,
            #[serde(default)]
            cache_creation_input_tokens: Option<u32>,
            #[serde(default)]
            cache_read_input_tokens: Option<u32>,
        }

        let msg: AnthropicMessage = resp
            .json()
            .await
            .map_err(|e| ProviderError::new("provider_malformed_response", &self.redact(&e.to_string())))?;

        // If stop_reason == "max_tokens", return a structured truncation error.
        if msg.stop_reason.as_deref() == Some("max_tokens") {
            return Err(ProviderError::new(
                "provider_truncated",
                "response truncated due to max_tokens limit",
            )
            .with_status(200));
        }

        let content: Vec<ResponseBlock> = msg
            .content
            .into_iter()
            .filter_map(|block| match block.block_type.as_str() {
                "text" => block.text.map(|t| ResponseBlock::Text { text: t }),
                "tool_use" => {
                    let id = block.id.unwrap_or_default();
                    let name = block.name.unwrap_or_default();
                    let input = block.input.unwrap_or_default();
                    Some(ResponseBlock::ToolUse { id, name, input })
                }
                _ => None,
            })
            .collect();

        let usage = TokenUsage {
            input_tokens: msg.usage.input_tokens,
            output_tokens: msg.usage.output_tokens,
            cache_creation_input_tokens: msg.usage.cache_creation_input_tokens,
            cache_read_input_tokens: msg.usage.cache_read_input_tokens,
        };

        Ok(ProviderResponse {
            content,
            usage,
            model: msg.model,
            stop_reason: msg.stop_reason,
            raw: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Default capabilities
// ---------------------------------------------------------------------------

fn default_capabilities() -> ProviderCapabilities {
    use crate::capabilities::{ModelCapabilities, Pricing};
    use std::collections::HashMap;

    let mut models = HashMap::new();

    models.insert(
        "claude-sonnet-4-6".to_string(),
        ModelCapabilities {
            max_context_tokens: 1_000_000,
            max_input_tokens: 992_000,
            max_output_tokens: 8192,
            output_reserve_tokens: 8192,
            counts_system_tokens: true,
            counts_tool_schemas: true,
            counts_tool_results: true,
            counts_reasoning_tokens: false,
            supports_server_token_count: true,
            supports_prompt_cache: true,
            overflow_behavior: "validation_error".to_string(),
            pricing: Pricing {
                input_per_million: 3.00,
                output_per_million: 15.00,
                cache_write_per_million: Some(3.75),
                cache_read_per_million: Some(0.30),
            },
            count_drift_p95_observed: None,
        },
    );

    models.insert(
        "claude-haiku-4-5".to_string(),
        ModelCapabilities {
            max_context_tokens: 200_000,
            max_input_tokens: 196_608,
            max_output_tokens: 8192,
            output_reserve_tokens: 8192,
            counts_system_tokens: true,
            counts_tool_schemas: true,
            counts_tool_results: true,
            counts_reasoning_tokens: false,
            supports_server_token_count: true,
            supports_prompt_cache: true,
            overflow_behavior: "validation_error".to_string(),
            pricing: Pricing {
                input_per_million: 1.00,
                output_per_million: 5.00,
                cache_write_per_million: Some(1.25),
                cache_read_per_million: Some(0.10),
            },
            count_drift_p95_observed: None,
        },
    );

    ProviderCapabilities {
        schema_version: 1,
        provider: "anthropic".to_string(),
        models,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::types::{ProviderMessage, ToolSchema};
    use super::*;

    fn secret(s: &str) -> SecretString {
        SecretString::new(s.to_string().into_boxed_str())
    }

    #[test]
    fn test_count_local() {
        let adapter = AnthropicAdapter::with_key(secret("sk-test"));
        let req = ProviderRequest {
            model: "claude-sonnet-4-6".to_string(),
            system: Some("You are a helpful assistant.".to_string()),
            messages: vec![ProviderMessage {
                role: "user".to_string(),
                content: "Hello world".to_string(),
            }],
            tools: None,
            max_tokens: None,
            temperature: None,
            stream: None,
            stop_sequences: None,
            extra: None,
        };
        let count = adapter.count_local(&req).unwrap();
        // "You are a helpful assistant." (5) + "Hello world" (2) = 7 words
        assert_eq!(count, 7);
    }

    #[test]
    fn test_redact_api_key() {
        let adapter = AnthropicAdapter::with_key(secret("sk-secret-123"));
        let text = "Error: sk-secret-123 is invalid";
        assert_eq!(adapter.redact(text), "Error: [REDACTED_API_KEY] is invalid");
    }

    #[test]
    fn test_build_body_basic() {
        let adapter = AnthropicAdapter::with_key(secret("sk-test"));
        let req = ProviderRequest {
            model: "claude-sonnet-4-6".to_string(),
            system: Some("sys".to_string()),
            messages: vec![ProviderMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: None,
            max_tokens: Some(100),
            temperature: Some(0.5),
            stream: Some(false),
            stop_sequences: None,
            extra: None,
        };
        let body = adapter.build_body(&req);
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["system"], "sys");
        assert_eq!(body["max_tokens"], 100);
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["stream"], false);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hi");
    }

    #[test]
    fn test_build_body_with_tools() {
        let adapter = AnthropicAdapter::with_key(secret("sk-test"));
        let req = ProviderRequest {
            model: "claude-sonnet-4-6".to_string(),
            system: None,
            messages: vec![],
            tools: Some(vec![ToolSchema {
                name: "get_weather".to_string(),
                parameters: json!({"type": "object", "properties": {}}),
                description: Some("Get the weather".to_string()),
            }]),
            max_tokens: None,
            temperature: None,
            stream: None,
            stop_sequences: None,
            extra: None,
        };
        let body = adapter.build_body(&req);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "get_weather");
        assert_eq!(tools[0]["description"], "Get the weather");
    }
}
