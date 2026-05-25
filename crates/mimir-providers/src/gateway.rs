//! Provider gateway: the single path to any provider.

use crate::adapters::{
    anthropic::AnthropicAdapter, openai_compatible::OpenAiCompatibleAdapter, ProviderAdapter,
};
use crate::capabilities::{snapshot_refs_match, ProviderCapabilities};
use crate::error::{ProviderError, Result as ProviderResult};
use crate::types::{ProviderRequest, ProviderResponse};
use serde::{Deserialize, Serialize};

/// A validated packet ready for provider dispatch.
#[derive(Debug, Clone)]
pub struct ValidatedPacket {
    /// Provider name.
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Capability snapshot ref embedded in the packet.
    pub capability_snapshot_ref: String,
    /// Estimated input tokens.
    pub estimated_input_tokens: u32,
    /// Output reserve.
    pub output_reserve_tokens: u32,
    /// Count drift reserve.
    pub count_drift_reserve_tokens: u32,
}

/// A provider request that has passed gateway validation.
#[derive(Debug)]
pub struct ValidatedProviderRequest {
    packet: ValidatedPacket,
    request: ProviderRequest,
}

/// Provider adapter handle accepted by the gateway dispatcher.
#[derive(Clone, Copy)]
pub enum ProviderDispatchAdapter<'a> {
    /// Anthropic Messages API adapter.
    Anthropic(&'a AnthropicAdapter),
    /// OpenAI-compatible chat completions adapter.
    OpenAiCompatible(&'a OpenAiCompatibleAdapter),
}

impl ProviderDispatchAdapter<'_> {
    fn capabilities(&self) -> &ProviderCapabilities {
        match self {
            ProviderDispatchAdapter::Anthropic(adapter) => adapter.capabilities(),
            ProviderDispatchAdapter::OpenAiCompatible(adapter) => adapter.capabilities(),
        }
    }
}

impl<'a> From<&'a AnthropicAdapter> for ProviderDispatchAdapter<'a> {
    fn from(adapter: &'a AnthropicAdapter) -> Self {
        Self::Anthropic(adapter)
    }
}

impl<'a> From<&'a OpenAiCompatibleAdapter> for ProviderDispatchAdapter<'a> {
    fn from(adapter: &'a OpenAiCompatibleAdapter) -> Self {
        Self::OpenAiCompatible(adapter)
    }
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
    /// Current capability snapshot ref, when known.
    capability_snapshot_ref: Option<String>,
}

impl ProviderGateway {
    /// Create a new gateway with the given capabilities.
    pub fn new(capabilities: ProviderCapabilities) -> Self {
        Self {
            capabilities,
            capability_snapshot_ref: None,
        }
    }

    /// Attach the current capability snapshot ref for stale-packet validation.
    pub fn with_capability_snapshot_ref(mut self, snapshot_ref: impl Into<String>) -> Self {
        self.capability_snapshot_ref = Some(snapshot_ref.into());
        self
    }

    /// Validate and bind a provider request for gateway-owned dispatch.
    pub fn prepare_request(
        &self,
        packet: ValidatedPacket,
        request: ProviderRequest,
    ) -> Result<ValidatedProviderRequest, String> {
        self.validate(&packet)?;
        Ok(ValidatedProviderRequest { packet, request })
    }

    /// Dispatch a gateway-validated request through a provider adapter.
    pub async fn dispatch(
        &self,
        adapter: ProviderDispatchAdapter<'_>,
        validated: ValidatedProviderRequest,
    ) -> ProviderResult<ProviderResponse> {
        self.validate(&validated.packet)
            .map_err(|message| ProviderError::new("gateway_refused", message))?;
        let adapter_capabilities = adapter.capabilities();
        if adapter_capabilities.provider != self.capabilities.provider
            || adapter_capabilities.provider != validated.packet.provider
        {
            return Err(ProviderError::new(
                "gateway_adapter_mismatch",
                format!(
                    "adapter provider {} does not match gateway provider {} and packet provider {}",
                    adapter_capabilities.provider,
                    self.capabilities.provider,
                    validated.packet.provider
                ),
            ));
        }
        if !adapter_capabilities
            .models
            .contains_key(&validated.packet.model)
        {
            return Err(ProviderError::new(
                "gateway_adapter_model_mismatch",
                format!(
                    "adapter provider {} has no capabilities for model {}",
                    adapter_capabilities.provider, validated.packet.model
                ),
            ));
        }

        match adapter {
            ProviderDispatchAdapter::Anthropic(adapter) => {
                adapter.dispatch_validated(validated.request).await
            }
            ProviderDispatchAdapter::OpenAiCompatible(adapter) => {
                adapter.dispatch_validated(validated.request).await
            }
        }
    }

    /// Validate a packet against policy (pure, no network).
    pub fn validate(&self, packet: &ValidatedPacket) -> Result<(), String> {
        if packet.provider != self.capabilities.provider {
            return Err(format!(
                "gateway_provider_mismatch: packet provider {} does not match capability provider {}",
                packet.provider, self.capabilities.provider
            ));
        }

        if let Some(current_snapshot_ref) = &self.capability_snapshot_ref {
            if !snapshot_refs_match(current_snapshot_ref, &packet.capability_snapshot_ref) {
                return Err(format!(
                    "gateway_capability_snapshot_mismatch: packet snapshot {} does not match current {}",
                    packet.capability_snapshot_ref, current_snapshot_ref
                ));
            }
        }

        let Some(model) = self.capabilities.models.get(&packet.model) else {
            return Err(format!(
                "gateway_unknown_model: {}/{}",
                packet.provider, packet.model
            ));
        };

        if packet.output_reserve_tokens > model.max_output_tokens {
            return Err(format!(
                "gateway_output_reserve_over_cap: {} > {}",
                packet.output_reserve_tokens, model.max_output_tokens
            ));
        }

        let total = packet
            .estimated_input_tokens
            .saturating_add(packet.output_reserve_tokens)
            .saturating_add(packet.count_drift_reserve_tokens);
        if packet.estimated_input_tokens > model.max_input_tokens
            || total > model.max_context_tokens
        {
            return Err(format!(
                "gateway_over_cap: input {} + output {} + drift {} > input cap {} or context cap {}",
                packet.estimated_input_tokens,
                packet.output_reserve_tokens,
                packet.count_drift_reserve_tokens,
                model.max_input_tokens,
                model.max_context_tokens
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
                overflow_behavior: "validation_error".to_string(),
                pricing: Pricing {
                    input_per_million: 1.0,
                    output_per_million: 2.0,
                    cache_write_per_million: None,
                    cache_read_per_million: None,
                },
                count_drift_p50_observed: None,
                count_drift_p95_observed: None,
                count_drift_p99_observed: None,
                drift_calibrated_at: None,
                recommended_cap_tokens: None,
            },
        );
        ProviderCapabilities {
            schema_version: 1,
            provider: "test".to_string(),
            models,
        }
    }

    #[tokio::test]
    async fn dispatch_rejects_adapter_provider_mismatch_before_provider_io() {
        let gateway = ProviderGateway::new(crate::capabilities::bundled_anthropic_capabilities());
        let request = ProviderRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            system: None,
            messages: vec![crate::types::ProviderMessage {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            tools: None,
            max_tokens: Some(16),
            temperature: None,
            stream: None,
            stop_sequences: None,
            extra: None,
        };
        let validated = gateway
            .prepare_request(
                ValidatedPacket {
                    provider: "anthropic".to_string(),
                    model: "claude-sonnet-4-20250514".to_string(),
                    capability_snapshot_ref: "test-snapshot".to_string(),
                    estimated_input_tokens: 1,
                    output_reserve_tokens: 16,
                    count_drift_reserve_tokens: 0,
                },
                request,
            )
            .unwrap();
        let adapter = crate::adapters::openai_compatible::OpenAiCompatibleAdapter::from_config(
            crate::adapters::openai_compatible::OpenAiCompatibleConfig {
                provider: "glm".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                model: "glm-5.1".to_string(),
                api_key: secrecy::SecretString::new("test-key".to_string().into_boxed_str()),
            },
        )
        .unwrap();

        let error = gateway
            .dispatch(ProviderDispatchAdapter::from(&adapter), validated)
            .await
            .unwrap_err();

        assert_eq!(error.code, "gateway_adapter_mismatch");
    }

    #[test]
    fn validate_under_cap_passes() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 500,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 0,
        };
        assert!(gw.validate(&packet).is_ok());
    }

    #[test]
    fn validate_over_cap_fails() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 1000,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 0,
        };
        let err = gw.validate(&packet).unwrap_err();
        assert!(err.contains("gateway_over_cap"));
    }

    #[test]
    fn validate_unknown_model_fails() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "unknown".to_string(),
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 60000,
            output_reserve_tokens: 1000,
            count_drift_reserve_tokens: 0,
        };
        let err = gw.validate(&packet).unwrap_err();
        assert!(err.contains("gateway_unknown_model"));
    }

    #[test]
    fn validate_exactly_at_cap_passes() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 900,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 0,
        };
        assert!(gw.validate(&packet).is_ok());
    }

    #[test]
    fn validate_zero_input_passes() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 0,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 0,
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
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 2000,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 0,
        };
        let result = gw.validate(&packet);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("gateway_over_cap"));
    }

    #[test]
    fn validate_provider_mismatch_fails() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "other".to_string(),
            model: "test-model".to_string(),
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 100,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 0,
        };
        let err = gw.validate(&packet).unwrap_err();
        assert!(err.contains("gateway_provider_mismatch"));
    }

    #[test]
    fn validate_stale_capability_snapshot_fails() {
        let current =
            "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let gw = ProviderGateway::new(test_caps()).with_capability_snapshot_ref(current);
        let mut packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            capability_snapshot_ref:
                "different-source.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 100,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 0,
        };

        assert!(gw.validate(&packet).is_ok());

        packet.capability_snapshot_ref =
            "test.yaml@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string();
        let err = gw.validate(&packet).unwrap_err();
        assert!(err.contains("gateway_capability_snapshot_mismatch"));
    }

    #[test]
    fn validate_includes_drift_reserve() {
        let gw = ProviderGateway::new(test_caps());
        let packet = ValidatedPacket {
            provider: "test".to_string(),
            model: "test-model".to_string(),
            capability_snapshot_ref:
                "test.yaml@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            estimated_input_tokens: 850,
            output_reserve_tokens: 100,
            count_drift_reserve_tokens: 51,
        };
        let err = gw.validate(&packet).unwrap_err();
        assert!(err.contains("gateway_over_cap"));
    }
}
