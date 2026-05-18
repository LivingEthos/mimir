//! Capability registry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Per-model capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCapabilities {
    /// Max context window.
    pub max_context_tokens: u32,
    /// Max input tokens.
    pub max_input_tokens: u32,
    /// Max output tokens.
    pub max_output_tokens: u32,
    /// Output reserve.
    pub output_reserve_tokens: u32,
    /// Whether system tokens are counted.
    pub counts_system_tokens: bool,
    /// Whether tool schemas are counted.
    pub counts_tool_schemas: bool,
    /// Whether tool results are counted.
    pub counts_tool_results: bool,
    /// Whether reasoning tokens are counted.
    pub counts_reasoning_tokens: bool,
    /// Whether server-side token count is available.
    pub supports_server_token_count: bool,
    /// Whether prompt caching is supported.
    pub supports_prompt_cache: bool,
    /// Overflow behavior.
    pub overflow_behavior: String,
    /// Pricing.
    pub pricing: Pricing,
    /// Observed P95 drift (null until calibrated).
    pub count_drift_p95_observed: Option<f64>,
}

/// Pricing info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pricing {
    /// Input cost per million tokens.
    pub input_per_million: f64,
    /// Output cost per million tokens.
    pub output_per_million: f64,
    /// Cache write cost per million tokens.
    pub cache_write_per_million: Option<f64>,
    /// Cache read cost per million tokens.
    pub cache_read_per_million: Option<f64>,
}

/// Provider capabilities snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    /// Schema version.
    pub schema_version: u32,
    /// Provider name.
    pub provider: String,
    /// Per-model capabilities.
    pub models: HashMap<String, ModelCapabilities>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_roundtrip() {
        let mut models = HashMap::new();
        models.insert(
            "test-model".to_string(),
            ModelCapabilities {
                max_context_tokens: 1000,
                max_input_tokens: 1000,
                max_output_tokens: 500,
                output_reserve_tokens: 100,
                counts_system_tokens: true,
                counts_tool_schemas: true,
                counts_tool_results: true,
                counts_reasoning_tokens: false,
                supports_server_token_count: false,
                supports_prompt_cache: false,
                overflow_behavior: "error".to_string(),
                pricing: Pricing {
                    input_per_million: 1.0,
                    output_per_million: 2.0,
                    cache_write_per_million: None,
                    cache_read_per_million: None,
                },
                count_drift_p95_observed: None,
            },
        );
        let caps = ProviderCapabilities {
            schema_version: 1,
            provider: "test".to_string(),
            models,
        };
        let json = serde_json::to_string(&caps).unwrap();
        let decoded: ProviderCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.provider, "test");
        assert!(decoded.models.contains_key("test-model"));
    }
}
