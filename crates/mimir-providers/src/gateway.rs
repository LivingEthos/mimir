//! Provider gateway: the single path to any provider.

use crate::capabilities::ProviderCapabilities;
use serde::{Deserialize, Serialize};

/// A validated packet ready for provider dispatch.
#[derive(Debug, Clone)]
pub struct ValidatedPacket {
    /// Provider name.
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Estimated input tokens.
    pub estimated_input_tokens: u32,
    /// Output reserve.
    pub output_reserve_tokens: u32,
}

/// Outcome of a provider call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallOutcome {
    /// Response text.
    pub response_text: String,
    /// Input tokens used.
    pub tokens_in: u32,
    /// Output tokens used.
    pub tokens_out: u32,
}

/// The provider gateway.
pub struct ProviderGateway {
    /// Registered capabilities.
    pub capabilities: ProviderCapabilities,
}

impl ProviderGateway {
    /// Create a new gateway with the given capabilities.
    pub fn new(capabilities: ProviderCapabilities) -> Self {
        Self { capabilities }
    }

    /// Validate a packet against policy (pure, no network).
    pub fn validate(&self, packet: &ValidatedPacket) -> Result<(), String> {
        let cap = self
            .capabilities
            .models
            .get(&packet.model)
            .map(|m| m.max_context_tokens)
            .unwrap_or(65536);
        let total = packet.estimated_input_tokens + packet.output_reserve_tokens;
        if total > cap {
            return Err(format!(
                "gateway_over_cap: {} + {} > {}",
                packet.estimated_input_tokens, packet.output_reserve_tokens, cap
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{ModelCapabilities, Pricing};
    use std::collections::HashMap;

    fn test_caps() -> ProviderCapabilities {
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
        ProviderCapabilities {
            schema_version: 1,
            provider: "test".to_string(),
            models,
        }
    }

    #[test]
    fn validate_under_cap_passes() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            estimated_input_tokens: 500,
            output_reserve_tokens: 100,
        };
        assert!(gw.validate(&packet).is_ok());
    }

    #[test]
    fn validate_over_cap_fails() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            estimated_input_tokens: 1000,
            output_reserve_tokens: 100,
        };
        let err = gw.validate(&packet).unwrap_err();
        assert!(err.contains("gateway_over_cap"));
    }

    #[test]
    fn validate_unknown_model_uses_default_cap() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "unknown".to_string(),
            estimated_input_tokens: 60000,
            output_reserve_tokens: 1000,
        };
        assert!(gw.validate(&packet).is_ok());
    }

    #[test]
    fn validate_exactly_at_cap_passes() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            estimated_input_tokens: 900,
            output_reserve_tokens: 100,
        };
        assert!(gw.validate(&packet).is_ok());
    }

    #[test]
    fn validate_zero_input_passes() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            estimated_input_tokens: 0,
            output_reserve_tokens: 100,
        };
        assert!(gw.validate(&packet).is_ok());
    }

    #[test]
    fn validate_rejects_before_provider_io() {
        // Cap compliance 100%: packet must be validated before any provider call
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            estimated_input_tokens: 2000,
            output_reserve_tokens: 100,
        };
        let result = gw.validate(&packet);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("gateway_over_cap"));
    }
}
