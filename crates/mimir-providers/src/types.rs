//! Provider-neutral request/response DTOs.
//!
//! Adapters translate between these types and provider-native API shapes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single message in a provider-neutral conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMessage {
    /// Role: user, assistant, system, tool.
    pub role: String,
    /// Text content.
    pub content: String,
}

/// Tool schema in a provider-neutral format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Tool name.
    pub name: String,
    /// JSON Schema for the tool's input parameters.
    pub parameters: serde_json::Value,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Provider-neutral request DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    /// Model identifier.
    pub model: String,
    /// System prompt / instructions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// Conversation messages.
    pub messages: Vec<ProviderMessage>,
    /// Available tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolSchema>>,
    /// Max tokens to generate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Sampling temperature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Whether to stream the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Stop sequences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    /// Extra provider-specific parameters (forwarded as-is by the adapter).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<HashMap<String, serde_json::Value>>,
}

/// A block of content in a provider response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseBlock {
    /// Plain text block.
    Text {
        /// Text content.
        text: String,
    },
    /// Tool use block.
    ToolUse {
        /// Tool use ID.
        id: String,
        /// Tool name.
        name: String,
        /// Parsed input JSON.
        input: serde_json::Value,
    },
}

/// Token usage reported by the provider.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    /// Input tokens consumed.
    pub input_tokens: u32,
    /// Output tokens consumed.
    pub output_tokens: u32,
    /// Cache creation input tokens (if supported).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u32>,
    /// Cache read input tokens (if supported).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u32>,
}

/// Provider-neutral response DTO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    /// Response content blocks.
    pub content: Vec<ResponseBlock>,
    /// Token usage.
    pub usage: TokenUsage,
    /// Model that produced the response.
    pub model: String,
    /// Stop reason (e.g. "end_turn", "max_tokens", "tool_use").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Raw response JSON for debugging.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<serde_json::Value>,
}
