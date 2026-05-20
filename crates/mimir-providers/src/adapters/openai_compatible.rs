//! OpenAI-compatible chat completions adapter.
//!
//! This supports protocol-compatible providers such as Z.AI GLM Coding Plan
//! and MiniMax OpenAI mode without baking vendor-specific logic into the CLI.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;

use crate::adapters::ProviderAdapter;
use crate::capabilities::ProviderCapabilities;
use crate::count;
use crate::error::{map_http_status, ProviderError, Result};
use crate::retry::{backoff, RetryPolicy};
use crate::types::{ProviderRequest, ProviderResponse, ResponseBlock, TokenUsage};

/// Configuration for an OpenAI-compatible provider endpoint.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleConfig {
    /// Provider display name stored in capability snapshots and packets.
    pub provider: String,
    /// API base URL without the `/chat/completions` suffix.
    pub base_url: String,
    /// Default model name.
    pub model: String,
    /// Bearer token.
    pub api_key: SecretString,
}

/// Adapter for OpenAI-compatible `/chat/completions` APIs.
pub struct OpenAiCompatibleAdapter {
    client: reqwest::Client,
    capabilities: ProviderCapabilities,
    config: OpenAiCompatibleConfig,
    retry_policy: RetryPolicy,
}

impl OpenAiCompatibleAdapter {
    /// Create an adapter from explicit configuration.
    pub fn from_config(config: OpenAiCompatibleConfig) -> Result<Self> {
        let capabilities = default_capabilities(&config.provider, &config.model)?;
        Ok(Self {
            client: reqwest::Client::new(),
            capabilities,
            config,
            retry_policy: RetryPolicy::default(),
        })
    }

    /// Create an adapter for Z.AI GLM Coding Plan using GLM/ZAI environment variables.
    pub fn glm_from_env() -> Result<Self> {
        let model = std::env::var("GLM_MODEL").unwrap_or_else(|_| "glm-5.1".to_string());
        Self::glm_from_env_with_model(model)
    }

    /// Create a GLM adapter using the caller-selected model and environment credentials.
    pub fn glm_from_env_with_model(model: impl Into<String>) -> Result<Self> {
        let api_key = std::env::var("GLM_API_KEY")
            .or_else(|_| std::env::var("ZAI_API_KEY"))
            .map(|s| SecretString::new(s.into_boxed_str()))
            .map_err(|_| {
                ProviderError::new(
                    "provider_unauthorized",
                    "GLM_API_KEY or ZAI_API_KEY not set",
                )
            })?;
        let base_url = std::env::var("GLM_BASE_URL")
            .unwrap_or_else(|_| "https://api.z.ai/api/coding/paas/v4".to_string());
        Self::from_config(OpenAiCompatibleConfig {
            provider: "glm".to_string(),
            base_url,
            model: model.into(),
            api_key,
        })
    }

    /// Create an adapter for a generic OpenAI-compatible endpoint from environment variables.
    pub fn generic_from_env(provider: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map(|s| SecretString::new(s.into_boxed_str()))
            .map_err(|_| ProviderError::new("provider_unauthorized", "OPENAI_API_KEY not set"))?;
        let base_url = std::env::var("OPENAI_BASE_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        Self::from_config(OpenAiCompatibleConfig {
            provider: provider.into(),
            base_url,
            model: model.into(),
            api_key,
        })
    }

    /// Override the retry policy.
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set a custom base URL.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.config.base_url = base_url.into();
        self
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let auth = format!("Bearer {}", self.config.api_key.expose_secret());
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth).expect("valid authorization header"),
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers
    }

    fn redact(&self, text: &str) -> String {
        mimir_security::redact_secrets(text)
            .replace(self.config.api_key.expose_secret(), "[REDACTED_API_KEY]")
    }

    fn build_body(&self, request: &ProviderRequest) -> serde_json::Value {
        let mut messages = Vec::new();
        if let Some(system) = &request.system {
            messages.push(json!({"role": "system", "content": system}));
        }
        messages.extend(
            request
                .messages
                .iter()
                .map(|m| json!({"role": m.role, "content": m.content})),
        );

        let mut body = json!({
            "model": request.model,
            "messages": messages,
        });

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
            body["stop"] = json!(stop);
        }
        if let Some(tools) = &request.tools {
            let tools_json: Vec<serde_json::Value> = tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = json!(tools_json);
        }
        if let Some(extra) = &request.extra {
            for (key, value) in extra {
                body[key] = value.clone();
            }
        }

        body
    }

    async fn request_with_retry(&self, body: serde_json::Value) -> Result<reqwest::Response> {
        let base = self.config.base_url.trim_end_matches('/');
        let url = format!("{base}/chat/completions");
        let headers = self.headers();

        for attempt in 0..self.retry_policy.max_attempts {
            let response = self
                .client
                .post(&url)
                .headers(headers.clone())
                .timeout(Duration::from_secs(120))
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let should_retry = matches!(status, 408 | 429 | 500..=599);
                    if !should_retry || attempt == self.retry_policy.max_attempts - 1 {
                        return Ok(resp);
                    }
                }
                Err(error) => {
                    if attempt == self.retry_policy.max_attempts - 1 {
                        return Err(ProviderError::new(
                            "provider_connection_reset",
                            format!(
                                "request failed after {} attempts: {}",
                                self.retry_policy.max_attempts,
                                self.redact(&error.to_string())
                            ),
                        )
                        .retryable());
                    }
                }
            }

            tokio::time::sleep(backoff(&self.retry_policy, attempt)).await;
        }

        Err(ProviderError::new(
            "provider_internal_error",
            "retry loop exhausted",
        ))
    }

    async fn parse_error(&self, resp: reqwest::Response) -> ProviderError {
        let status = resp.status().as_u16();
        let body_text = resp.text().await.unwrap_or_default();
        let redacted = self.redact(&body_text);

        #[derive(Deserialize)]
        struct ErrorBody {
            error: Option<ErrorDetail>,
        }

        #[derive(Deserialize)]
        struct ErrorDetail {
            message: Option<String>,
        }

        if let Ok(parsed) = serde_json::from_str::<ErrorBody>(&body_text) {
            if let Some(message) = parsed.error.and_then(|error| error.message) {
                return map_http_status(status, &self.redact(&message));
            }
        }

        map_http_status(status, &redacted)
    }
}

impl ProviderAdapter for OpenAiCompatibleAdapter {
    fn name(&self) -> &str {
        &self.config.provider
    }

    fn capabilities(&self) -> &ProviderCapabilities {
        &self.capabilities
    }

    fn count_local(&self, request: &ProviderRequest) -> Result<u32> {
        let mut messages = Vec::new();
        for message in &request.messages {
            messages.push((message.role.clone(), message.content.clone()));
        }
        Ok(count::count_request_local(
            request.system.as_deref(),
            &messages,
        ))
    }

    async fn count_server(&self, request: &ProviderRequest) -> Result<u32> {
        self.count_local(request)
    }
}

impl OpenAiCompatibleAdapter {
    pub(crate) async fn dispatch_validated(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse> {
        let body = self.build_body(&request);
        let resp = self.request_with_retry(body).await?;

        if !resp.status().is_success() {
            return Err(self.parse_error(resp).await);
        }

        let parsed: ChatCompletionResponse = resp.json().await.map_err(|error| {
            ProviderError::new(
                "provider_malformed_response",
                self.redact(&error.to_string()),
            )
        })?;

        let Some(choice) = parsed.choices.into_iter().next() else {
            return Err(ProviderError::new(
                "provider_malformed_response",
                "missing choices in OpenAI-compatible response",
            ));
        };

        if choice.finish_reason.as_deref() == Some("length") {
            return Err(ProviderError::new(
                "provider_truncated",
                "response truncated due to max_tokens limit",
            )
            .with_status(200));
        }

        let mut content = Vec::new();
        if let Some(text) = choice.message.content_text() {
            if !text.is_empty() {
                content.push(ResponseBlock::Text { text });
            }
        }
        for tool_call in choice.message.tool_calls.unwrap_or_default() {
            if tool_call.call_type == "function" {
                let input = serde_json::from_str(&tool_call.function.arguments)
                    .unwrap_or_else(|_| json!({"arguments": tool_call.function.arguments}));
                content.push(ResponseBlock::ToolUse {
                    id: tool_call.id,
                    name: tool_call.function.name,
                    input,
                });
            }
        }

        Ok(ProviderResponse {
            content,
            usage: TokenUsage {
                input_tokens: parsed.usage.as_ref().map_or(0, |u| u.prompt_tokens),
                output_tokens: parsed.usage.as_ref().map_or(0, |u| u.completion_tokens),
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
            model: parsed.model,
            stop_reason: choice.finish_reason,
            raw: None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    model: String,
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: Option<serde_json::Value>,
    tool_calls: Option<Vec<ToolCall>>,
}

impl Message {
    fn content_text(&self) -> Option<String> {
        match self.content.as_ref()? {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Array(parts) => {
                let text = parts
                    .iter()
                    .filter_map(|part| {
                        part.get("text")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
                    .collect::<Vec<_>>()
                    .join("");
                Some(text)
            }
            other => Some(other.to_string()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: FunctionCall,
}

#[derive(Debug, Deserialize)]
struct FunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

fn default_capabilities(provider: &str, model: &str) -> Result<ProviderCapabilities> {
    crate::capabilities::resolve_provider_capabilities(provider, model)
        .map(|resolved| resolved.capabilities)
        .map_err(|message| ProviderError::new("provider_config_error", message))
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;
    use crate::retry::RetryPolicy;
    use crate::types::ProviderMessage;

    fn adapter(base_url: String) -> OpenAiCompatibleAdapter {
        OpenAiCompatibleAdapter::from_config(OpenAiCompatibleConfig {
            provider: "glm".to_string(),
            base_url,
            model: "glm-5.1".to_string(),
            api_key: SecretString::new("test-key".to_string().into_boxed_str()),
        })
        .unwrap()
        .with_retry_policy(RetryPolicy {
            max_attempts: 1,
            initial_backoff_ms: 1,
            max_backoff_ms: 1,
            jitter: 0.0,
        })
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            model: "glm-5.1".to_string(),
            system: Some("You are terse.".to_string()),
            messages: vec![ProviderMessage {
                role: "user".to_string(),
                content: "Reply OK".to_string(),
            }],
            tools: None,
            max_tokens: Some(16),
            temperature: Some(0.0),
            stream: Some(false),
            stop_sequences: None,
            extra: None,
        }
    }

    #[test]
    fn runtime_capabilities_are_gateway_consistent() {
        let capabilities = default_capabilities("glm", "glm-5.1").unwrap();
        assert_eq!(capabilities.provider, "glm");
        let model = capabilities.models.get("glm-5.1").unwrap();
        assert!(model.output_reserve_tokens <= model.max_output_tokens);
        assert!(
            model.max_input_tokens
                + model.output_reserve_tokens
                + crate::capabilities::DEFAULT_COUNT_DRIFT_RESERVE_TOKENS
                <= model.max_context_tokens
        );
        assert_eq!(model.overflow_behavior, "validation_error");
    }

    #[test]
    fn count_local_is_positive() {
        let adapter = adapter("http://127.0.0.1".to_string());
        assert!(adapter.count_local(&request()).unwrap() > 0);
    }

    #[tokio::test]
    async fn parses_chat_completion_response() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "glm-5.1",
                "choices": [{
                    "message": {"role": "assistant", "content": "OK."},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 7, "completion_tokens": 2}
            })))
            .mount(&server)
            .await;

        let adapter = adapter(server.uri());
        let request = request();
        let gateway =
            crate::gateway::ProviderGateway::new(default_capabilities("glm", "glm-5.1").unwrap());
        let validated = gateway
            .prepare_request(
                crate::gateway::ValidatedPacket {
                    provider: "glm".to_string(),
                    model: "glm-5.1".to_string(),
                    capability_snapshot_ref: "test-snapshot".to_string(),
                    estimated_input_tokens: adapter.count_local(&request).unwrap(),
                    output_reserve_tokens: 16,
                    count_drift_reserve_tokens: 0,
                },
                request,
            )
            .unwrap();
        let response = gateway
            .dispatch(
                crate::gateway::ProviderDispatchAdapter::from(&adapter),
                validated,
            )
            .await
            .unwrap();
        assert_eq!(response.model, "glm-5.1");
        assert_eq!(response.usage.input_tokens, 7);
        assert_eq!(response.usage.output_tokens, 2);
        assert!(matches!(
            response.content.first(),
            Some(ResponseBlock::Text { text }) if text == "OK."
        ));
    }
}
