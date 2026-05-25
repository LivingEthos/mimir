//! Capability registry.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const ANTHROPIC_YAML: &str = include_str!("../../../providers/anthropic.yaml");
#[cfg(test)]
const PROVIDER_CAPABILITIES_SCHEMA: &str =
    include_str!("../../../schemas/ProviderCapabilities.schema.json");
const MIMIR_CONFIG_PATH: &str = ".mimir/config.yaml";
const PROVIDER_CAPABILITIES_PATH_CONFIG_KEY: &str = "provider_capabilities_path";
const DYNAMIC_OPENAI_COMPATIBLE_PROVIDERS: &[(&str, &str)] = &[
    ("glm", "glm-5.1"),
    ("openai", "gpt-4.1"),
    ("openai-compatible", "gpt-4.1"),
];

/// Environment variable containing a local provider capability YAML path.
pub const LOCAL_PROVIDER_CAPABILITIES_PATH_ENV: &str = "MIMIR_PROVIDER_CAPABILITIES_PATH";

/// Conservative local-count drift reserve used for registry-backed packets.
pub const DEFAULT_COUNT_DRIFT_RESERVE_TOKENS: u32 = 512;

/// Per-model capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Observed P50 drift (null until calibrated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_drift_p50_observed: Option<f64>,
    /// Observed P95 drift (null until calibrated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_drift_p95_observed: Option<f64>,
    /// Observed P99 drift (null until calibrated).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_drift_p99_observed: Option<f64>,
    /// Timestamp of the latest drift calibration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_calibrated_at: Option<String>,
    /// Eval-suite-derived recommended cap for this model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended_cap_tokens: Option<u32>,
}

/// Pricing info.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pricing {
    /// Input cost per million tokens.
    pub input_per_million: f64,
    /// Output cost per million tokens.
    pub output_per_million: f64,
    /// Cache write cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_per_million: Option<f64>,
    /// Cache read cost per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_per_million: Option<f64>,
}

/// Provider capabilities snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilities {
    /// Schema version.
    pub schema_version: u32,
    /// Provider name.
    pub provider: String,
    /// Per-model capabilities.
    pub models: HashMap<String, ModelCapabilities>,
}

/// Provider capability snapshots returned by provider-list APIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCapabilitiesList {
    /// Schema version.
    pub schema_version: u32,
    /// Registry-backed providers plus dynamic OpenAI-compatible descriptors.
    pub providers: Vec<ProviderCapabilities>,
}

/// Provider capabilities resolved for a concrete provider/model pair.
#[derive(Debug, Clone)]
pub struct ResolvedProviderCapabilities {
    /// Capabilities used for gateway validation.
    pub capabilities: ProviderCapabilities,
    /// Snapshot reference embedded into context packets.
    pub snapshot_ref: String,
    /// Whether the snapshot came from the capability registry.
    pub registry_backed: bool,
}

/// Loaded provider capability snapshots keyed by provider name.
#[derive(Debug, Clone, Default)]
pub struct CapabilityRegistry {
    providers: HashMap<String, ProviderCapabilities>,
    snapshot_refs: HashMap<String, String>,
}

impl CapabilityRegistry {
    /// Load Mimir's bundled snapshots plus local YAML when configured.
    pub fn bundled() -> Result<Self, String> {
        let mut registry = Self::bundled_base()?;
        registry.load_local_capabilities_from_env_or_config()?;
        Ok(registry)
    }

    fn bundled_base() -> Result<Self, String> {
        let caps = parse_provider_yaml("providers/anthropic.yaml", ANTHROPIC_YAML)?;
        let mut registry = Self::default();
        registry.insert_with_ref(
            caps,
            snapshot_ref_from_bytes("providers/anthropic.yaml", ANTHROPIC_YAML.as_bytes()),
        )?;
        Ok(registry)
    }

    /// Load bundled snapshots and one explicit local capability YAML file.
    pub fn bundled_with_local_capabilities_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let mut registry = Self::bundled_base()?;
        registry.insert_provider_file(path.as_ref())?;
        Ok(registry)
    }

    /// Load provider capability snapshots from a directory of `*.yaml` files.
    pub fn from_dir(path: impl AsRef<Path>) -> Result<Self, String> {
        let mut registry = Self::default();
        for entry in std::fs::read_dir(path.as_ref())
            .map_err(|error| format!("failed to read provider directory: {error}"))?
        {
            let entry = entry.map_err(|error| format!("failed to read provider entry: {error}"))?;
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
                continue;
            };
            if extension != "yaml" && extension != "yml" {
                continue;
            }

            registry.insert_provider_file(&path)?;
        }
        Ok(registry)
    }

    fn load_local_capabilities_from_env_or_config(&mut self) -> Result<(), String> {
        let Some(path) = local_capabilities_path_from_env_or_config()? else {
            return Ok(());
        };
        self.insert_provider_file(&path)
    }

    fn insert_provider_file(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let source = path.display().to_string();
        let caps = parse_provider_yaml(&source, &text)?;
        self.insert_with_ref(caps, snapshot_ref_from_bytes(&source, text.as_bytes()))
    }

    /// Insert a provider snapshot after validating it.
    pub fn insert_with_ref(
        &mut self,
        caps: ProviderCapabilities,
        snapshot_ref: String,
    ) -> Result<(), String> {
        validate_capabilities(&snapshot_ref, &caps).map_err(|errors| errors.join("; "))?;
        if self.providers.contains_key(&caps.provider) {
            return Err(format!(
                "duplicate provider capabilities for {}",
                caps.provider
            ));
        }
        self.snapshot_refs
            .insert(caps.provider.clone(), snapshot_ref.clone());
        self.providers.insert(caps.provider.clone(), caps);
        Ok(())
    }

    /// Return a provider snapshot.
    pub fn provider(&self, provider: &str) -> Option<&ProviderCapabilities> {
        self.providers.get(provider)
    }

    /// Return a model snapshot.
    pub fn model(&self, provider: &str, model: &str) -> Option<&ModelCapabilities> {
        self.provider(provider)?.models.get(model)
    }

    /// Return the capability snapshot ref for a provider.
    pub fn snapshot_ref(&self, provider: &str) -> Option<&str> {
        self.snapshot_refs.get(provider).map(String::as_str)
    }

    /// Iterate loaded provider snapshots.
    pub fn providers(&self) -> impl Iterator<Item = &ProviderCapabilities> {
        self.providers.values()
    }

    /// Resolve registry-backed capabilities for a provider/model pair.
    pub fn resolve_provider_capabilities(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<Option<ResolvedProviderCapabilities>, String> {
        let Some(capabilities) = self.provider(provider) else {
            return Ok(None);
        };
        if !capabilities.models.contains_key(model) {
            return Err(format!(
                "provider registry has no capabilities for {provider}/{model}"
            ));
        }
        let snapshot_ref = self
            .snapshot_ref(provider)
            .ok_or_else(|| format!("provider registry missing snapshot ref for {provider}"))?
            .to_string();
        Ok(Some(ResolvedProviderCapabilities {
            capabilities: capabilities.clone(),
            snapshot_ref,
            registry_backed: true,
        }))
    }
}

fn local_capabilities_path_from_env_or_config() -> Result<Option<PathBuf>, String> {
    if let Some(path) = std::env::var_os(LOCAL_PROVIDER_CAPABILITIES_PATH_ENV) {
        if path.is_empty() {
            return Err(format!(
                "{LOCAL_PROVIDER_CAPABILITIES_PATH_ENV} must not be empty"
            ));
        }
        return Ok(Some(PathBuf::from(path)));
    }

    local_capabilities_path_from_config()
}

fn local_capabilities_path_from_config() -> Result<Option<PathBuf>, String> {
    let config_path = Path::new(MIMIR_CONFIG_PATH);
    local_capabilities_path_from_config_at(config_path)
}

fn local_capabilities_path_from_config_at(config_path: &Path) -> Result<Option<PathBuf>, String> {
    if !config_path.is_file() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(config_path)
        .map_err(|error| format!("failed to read {MIMIR_CONFIG_PATH}: {error}"))?;
    let value: serde_yaml::Value = serde_yaml::from_str(&text)
        .map_err(|error| format!("{MIMIR_CONFIG_PATH}: invalid config YAML: {error}"))?;
    let Some(mapping) = value.as_mapping() else {
        return Ok(None);
    };
    let key = serde_yaml::Value::String(PROVIDER_CAPABILITIES_PATH_CONFIG_KEY.to_string());
    let Some(raw_path) = mapping.get(&key) else {
        return Ok(None);
    };
    let Some(path) = raw_path.as_str() else {
        return Err(format!(
            "{MIMIR_CONFIG_PATH}: {PROVIDER_CAPABILITIES_PATH_CONFIG_KEY} must be a string path"
        ));
    };
    if path.trim().is_empty() {
        return Err(format!(
            "{MIMIR_CONFIG_PATH}: {PROVIDER_CAPABILITIES_PATH_CONFIG_KEY} must not be empty"
        ));
    }

    let path = PathBuf::from(path);
    if path.is_absolute() {
        Ok(Some(path))
    } else {
        Ok(Some(
            config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(path),
        ))
    }
}

/// Load bundled Anthropic capabilities.
pub fn bundled_anthropic_capabilities() -> ProviderCapabilities {
    CapabilityRegistry::bundled()
        .ok()
        .and_then(|registry| registry.provider("anthropic").cloned())
        .unwrap_or_else(fallback_anthropic_capabilities)
}

/// Resolve capabilities and a snapshot ref for a provider/model pair.
pub fn resolve_provider_capabilities(
    provider: &str,
    model: &str,
) -> Result<ResolvedProviderCapabilities, String> {
    let registry = CapabilityRegistry::bundled()?;
    if let Some(resolved) = registry.resolve_provider_capabilities(provider, model)? {
        return Ok(resolved);
    }

    if !DYNAMIC_OPENAI_COMPATIBLE_PROVIDERS
        .iter()
        .any(|(dynamic_provider, _)| *dynamic_provider == provider)
    {
        return Err(format!(
            "provider {provider} is not registry-backed; add local capability YAML or use openai-compatible"
        ));
    }

    let capabilities = openai_compatible_capabilities(provider, model);
    let snapshot_ref = generated_capability_snapshot_ref(&capabilities);
    Ok(ResolvedProviderCapabilities {
        capabilities,
        snapshot_ref,
        registry_backed: false,
    })
}

/// List registry-backed providers plus dynamic OpenAI-compatible descriptors.
pub fn provider_capabilities_list() -> Result<ProviderCapabilitiesList, String> {
    let registry = CapabilityRegistry::bundled()?;
    Ok(provider_capabilities_list_from_registry(&registry))
}

/// Build a provider-list response from a loaded capability registry.
pub fn provider_capabilities_list_from_registry(
    registry: &CapabilityRegistry,
) -> ProviderCapabilitiesList {
    let mut providers = registry.providers().cloned().collect::<Vec<_>>();
    for (provider, model) in DYNAMIC_OPENAI_COMPATIBLE_PROVIDERS {
        if registry.provider(provider).is_none() {
            providers.push(openai_compatible_capabilities(provider, model));
        }
    }
    providers.sort_by(|left, right| left.provider.cmp(&right.provider));
    ProviderCapabilitiesList {
        schema_version: 1,
        providers,
    }
}

/// Return a generic OpenAI-compatible capability snapshot for a custom model.
pub fn openai_compatible_capabilities(provider: &str, model: &str) -> ProviderCapabilities {
    let mut models = HashMap::new();
    models.insert(
        model.to_string(),
        ModelCapabilities {
            max_context_tokens: 131_072,
            max_input_tokens: 126_464,
            max_output_tokens: 8192,
            output_reserve_tokens: 4096,
            counts_system_tokens: true,
            counts_tool_schemas: true,
            counts_tool_results: true,
            counts_reasoning_tokens: true,
            supports_server_token_count: false,
            supports_prompt_cache: false,
            overflow_behavior: "validation_error".to_string(),
            pricing: Pricing {
                input_per_million: 0.0,
                output_per_million: 0.0,
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
        provider: provider.to_string(),
        models,
    }
}

/// Return a deterministic snapshot ref for runtime-generated capabilities.
pub fn generated_capability_snapshot_ref(caps: &ProviderCapabilities) -> String {
    let source_name = caps
        .models
        .keys()
        .min()
        .filter(|_| caps.models.len() == 1)
        .map(|model| format!("generated:{}/{}", caps.provider, model))
        .unwrap_or_else(|| format!("generated:{}", caps.provider));
    snapshot_ref_from_bytes(&source_name, &stable_capabilities_bytes(caps))
}

/// Return true when two snapshot refs identify the same snapshot hash.
pub fn snapshot_refs_match(current: &str, packet: &str) -> bool {
    match (snapshot_ref_hash(current), snapshot_ref_hash(packet)) {
        (Some(current_hash), Some(packet_hash)) => current_hash.eq_ignore_ascii_case(packet_hash),
        _ => current == packet,
    }
}

/// Extract the sha256 suffix from a snapshot ref.
pub fn snapshot_ref_hash(snapshot_ref: &str) -> Option<&str> {
    let (_, hash) = snapshot_ref.rsplit_once("@sha256:")?;
    if hash.len() == 64 && hash.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit()) {
        Some(hash)
    } else {
        None
    }
}

fn stable_capabilities_bytes(caps: &ProviderCapabilities) -> Vec<u8> {
    #[derive(Serialize)]
    struct StableCapabilities<'a> {
        schema_version: u32,
        provider: &'a str,
        models: BTreeMap<&'a str, &'a ModelCapabilities>,
    }

    let models = caps
        .models
        .iter()
        .map(|(name, model)| (name.as_str(), model))
        .collect();
    serde_json::to_vec(&StableCapabilities {
        schema_version: caps.schema_version,
        provider: &caps.provider,
        models,
    })
    .expect("provider capabilities should serialize")
}

/// Validate schema and semantic invariants for one provider snapshot.
pub fn validate_capabilities(
    source_name: &str,
    caps: &ProviderCapabilities,
) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if caps.schema_version != 1 {
        errors.push(format!(
            "{source_name}: schema_version must be 1, got {}",
            caps.schema_version
        ));
    }
    if caps.provider.trim().is_empty() {
        errors.push(format!("{source_name}: provider must not be empty"));
    }
    if caps.models.is_empty() {
        errors.push(format!("{source_name}: models must not be empty"));
    }

    for (model_name, model) in &caps.models {
        if model.output_reserve_tokens > model.max_output_tokens {
            errors.push(format!(
                "{source_name}: {model_name} output_reserve_tokens {} exceeds max_output_tokens {}",
                model.output_reserve_tokens, model.max_output_tokens
            ));
        }
        if !matches!(
            model.overflow_behavior.as_str(),
            "validation_error" | "stop_reason" | "truncation" | "unknown"
        ) {
            errors.push(format!(
                "{source_name}: {model_name} overflow_behavior {} is not schema-valid",
                model.overflow_behavior
            ));
        }

        let total = model
            .max_input_tokens
            .saturating_add(model.output_reserve_tokens)
            .saturating_add(DEFAULT_COUNT_DRIFT_RESERVE_TOKENS);
        if total > model.max_context_tokens {
            errors.push(format!(
                "{source_name}: {model_name} max_input_tokens + output_reserve_tokens + drift reserve exceeds max_context_tokens"
            ));
        }
        let pricing_values = [
            model.pricing.input_per_million,
            model.pricing.output_per_million,
            model.pricing.cache_write_per_million.unwrap_or_default(),
            model.pricing.cache_read_per_million.unwrap_or_default(),
        ];
        if pricing_values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            errors.push(format!(
                "{source_name}: {model_name} pricing must be finite and non-negative"
            ));
        }
        if let (Some(p50), Some(p95)) = (
            model.count_drift_p50_observed,
            model.count_drift_p95_observed,
        ) {
            if p50 > p95 {
                errors.push(format!(
                    "{source_name}: {model_name} count_drift_p50_observed exceeds p95"
                ));
            }
        }
        if let (Some(p95), Some(p99)) = (
            model.count_drift_p95_observed,
            model.count_drift_p99_observed,
        ) {
            if p95 > p99 {
                errors.push(format!(
                    "{source_name}: {model_name} count_drift_p95_observed exceeds p99"
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn parse_provider_yaml(source_name: &str, text: &str) -> Result<ProviderCapabilities, String> {
    let caps: ProviderCapabilities = serde_yaml::from_str(text)
        .map_err(|error| format!("{source_name}: invalid provider YAML: {error}"))?;
    validate_capabilities(source_name, &caps).map_err(|errors| errors.join("; "))?;
    Ok(caps)
}

fn snapshot_ref_from_bytes(source_name: &str, bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{source_name}@sha256:{:x}", hasher.finalize())
}

fn fallback_anthropic_capabilities() -> ProviderCapabilities {
    let mut models = HashMap::new();
    let sonnet = ModelCapabilities {
        max_context_tokens: 1_000_000,
        max_input_tokens: 935_488,
        max_output_tokens: 64_000,
        output_reserve_tokens: 64_000,
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
        count_drift_p50_observed: None,
        count_drift_p95_observed: None,
        count_drift_p99_observed: None,
        drift_calibrated_at: None,
        recommended_cap_tokens: None,
    };
    models.insert("claude-sonnet-4-6".to_string(), sonnet.clone());
    models.insert("claude-sonnet-4-20250514".to_string(), sonnet);
    models.insert(
        "claude-haiku-4-5".to_string(),
        ModelCapabilities {
            max_context_tokens: 200_000,
            max_input_tokens: 135_488,
            max_output_tokens: 64_000,
            output_reserve_tokens: 64_000,
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
                cache_write_per_million: Some(1.00),
                cache_read_per_million: Some(0.08),
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
        provider: "anthropic".to_string(),
        models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_yaml_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mimir-provider-capabilities-{label}-{}-{nonce}.yaml",
            std::process::id()
        ))
    }

    fn local_capabilities_yaml(provider: &str, model: &str, output_reserve: u32) -> String {
        format!(
            r#"schema_version: 1
provider: {provider}
models:
  {model}:
    max_context_tokens: 32768
    max_input_tokens: 28000
    max_output_tokens: 4096
    output_reserve_tokens: {output_reserve}
    counts_system_tokens: true
    counts_tool_schemas: true
    counts_tool_results: true
    counts_reasoning_tokens: true
    supports_server_token_count: false
    supports_prompt_cache: false
    overflow_behavior: validation_error
    pricing:
      input_per_million: 0.1
      output_per_million: 0.2
"#
        )
    }

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

    #[test]
    fn bundled_provider_yamls_validate_semantic_invariants() {
        let registry = CapabilityRegistry::bundled().unwrap();
        let anthropic = registry.provider("anthropic").unwrap();
        assert!(anthropic.models.contains_key("claude-sonnet-4-20250514"));
        for (model_name, model) in &anthropic.models {
            assert!(
                model.max_input_tokens
                    + model.output_reserve_tokens
                    + DEFAULT_COUNT_DRIFT_RESERVE_TOKENS
                    <= model.max_context_tokens,
                "{model_name} must reserve context for input, output, and drift"
            );
            assert!(model.output_reserve_tokens <= model.max_output_tokens);
        }
        assert!(registry
            .snapshot_ref("anthropic")
            .unwrap()
            .starts_with("providers/anthropic.yaml@sha256:"));
    }

    #[test]
    fn local_capability_yaml_loads_as_registry_backed_provider() {
        let path = temp_yaml_path("valid");
        std::fs::write(
            &path,
            local_capabilities_yaml("local-openai", "local-model", 1024),
        )
        .unwrap();

        let registry = CapabilityRegistry::bundled_with_local_capabilities_file(&path).unwrap();
        let resolved = registry
            .resolve_provider_capabilities("local-openai", "local-model")
            .unwrap()
            .unwrap();
        assert!(resolved.registry_backed);
        assert_eq!(resolved.capabilities.provider, "local-openai");
        assert_eq!(
            resolved
                .capabilities
                .models
                .get("local-model")
                .unwrap()
                .output_reserve_tokens,
            1024
        );
        assert!(resolved
            .snapshot_ref
            .starts_with(&format!("{}@sha256:", path.display())));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn local_capability_yaml_rejects_credentials_and_unknown_fields() {
        let path = temp_yaml_path("credential-field");
        let yaml = format!(
            "{}api_key: sk-test-should-not-appear\n",
            local_capabilities_yaml("local-openai", "local-model", 1024)
        );
        std::fs::write(&path, yaml).unwrap();

        let error = CapabilityRegistry::bundled_with_local_capabilities_file(&path).unwrap_err();
        assert!(error.contains("unknown field"));
        assert!(error.contains("api_key"));
        assert!(!error.contains("sk-test-should-not-appear"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn local_capability_yaml_fails_closed_on_invalid_yaml() {
        let path = temp_yaml_path("invalid");
        std::fs::write(&path, "schema_version: [").unwrap();

        let error = CapabilityRegistry::bundled_with_local_capabilities_file(&path).unwrap_err();
        assert!(error.contains("invalid provider YAML"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn config_path_loader_resolves_relative_capability_path() {
        let dir = std::env::temp_dir().join(format!(
            "mimir-provider-config-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join("config.yaml");
        std::fs::write(
            &config_path,
            "version: 1\nprovider_capabilities_path: local-provider.yaml\n",
        )
        .unwrap();

        let resolved = local_capabilities_path_from_config_at(&config_path)
            .unwrap()
            .unwrap();
        assert_eq!(resolved, dir.join("local-provider.yaml"));

        let _ = std::fs::remove_file(config_path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn provider_list_merges_registry_and_dynamic_openai_compatible_descriptors() {
        let path = temp_yaml_path("list");
        std::fs::write(
            &path,
            local_capabilities_yaml("local-openai", "local-model", 1024),
        )
        .unwrap();

        let registry = CapabilityRegistry::bundled_with_local_capabilities_file(&path).unwrap();
        let list = provider_capabilities_list_from_registry(&registry);
        let names = list
            .providers
            .iter()
            .map(|provider| provider.provider.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"anthropic"));
        assert!(names.contains(&"glm"));
        assert!(names.contains(&"local-openai"));
        assert!(names.contains(&"openai"));
        assert!(names.contains(&"openai-compatible"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn unregistered_custom_provider_fails_closed() {
        let error = resolve_provider_capabilities("typo-compatible", "model").unwrap_err();
        assert!(error.contains("not registry-backed"));
        assert!(error.contains("local capability YAML"));
    }

    #[test]
    fn provider_list_prefers_registry_glm_over_dynamic_descriptor() {
        let path = temp_yaml_path("registry-glm");
        std::fs::write(&path, local_capabilities_yaml("glm", "glm-5.1", 2048)).unwrap();

        let registry = CapabilityRegistry::bundled_with_local_capabilities_file(&path).unwrap();
        let list = provider_capabilities_list_from_registry(&registry);
        let glm_providers = list
            .providers
            .iter()
            .filter(|provider| provider.provider == "glm")
            .collect::<Vec<_>>();
        assert_eq!(glm_providers.len(), 1);
        assert_eq!(
            glm_providers[0]
                .models
                .get("glm-5.1")
                .unwrap()
                .output_reserve_tokens,
            2048
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn generated_snapshot_refs_are_hash_comparable() {
        let caps = openai_compatible_capabilities("glm", "glm-5.1");
        let snapshot_ref = generated_capability_snapshot_ref(&caps);
        assert!(snapshot_ref.starts_with("generated:glm/glm-5.1@sha256:"));

        let hash = snapshot_ref_hash(&snapshot_ref).unwrap();
        let alternate_source = format!("other-source.yaml@sha256:{hash}");
        assert!(snapshot_refs_match(&snapshot_ref, &alternate_source));

        let stale = "generated:glm/glm-5.1@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        assert!(!snapshot_refs_match(&snapshot_ref, stale));
        assert!(!snapshot_refs_match(&snapshot_ref, "generated:glm/glm-5.1"));
    }

    #[test]
    fn validation_matches_schema_pricing_and_overflow_constraints() {
        let mut caps = openai_compatible_capabilities("test", "model");
        let model = caps.models.get_mut("model").unwrap();

        model.overflow_behavior = "surprise".to_string();
        model.pricing.input_per_million = -1.0;

        let errors = validate_capabilities("test", &caps).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("overflow_behavior")));
        assert!(errors.iter().any(|error| error.contains("pricing")));
    }

    #[test]
    fn validation_reserves_drift_headroom() {
        let mut caps = openai_compatible_capabilities("test", "model");
        let model = caps.models.get_mut("model").unwrap();
        model.max_input_tokens = model
            .max_context_tokens
            .saturating_sub(model.output_reserve_tokens);

        let errors = validate_capabilities("test", &caps).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("drift reserve")));
    }

    #[test]
    fn schema_and_rust_reject_unknown_pricing_fields() {
        let value = serde_json::json!({
            "schema_version": 1,
            "provider": "test",
            "models": {
                "model": {
                    "max_context_tokens": 1000,
                    "max_input_tokens": 900,
                    "max_output_tokens": 100,
                    "output_reserve_tokens": 100,
                    "counts_system_tokens": true,
                    "counts_tool_schemas": true,
                    "counts_tool_results": true,
                    "counts_reasoning_tokens": false,
                    "supports_server_token_count": false,
                    "supports_prompt_cache": false,
                    "overflow_behavior": "validation_error",
                    "pricing": {
                        "input_per_million": 1.0,
                        "output_per_million": 2.0,
                        "surprise": 3.0
                    }
                }
            }
        });

        let rust_error = serde_json::from_value::<ProviderCapabilities>(value.clone()).unwrap_err();
        assert!(rust_error.to_string().contains("unknown field"));

        let schema: serde_json::Value = serde_json::from_str(PROVIDER_CAPABILITIES_SCHEMA).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(!validator.is_valid(&value));
    }
}
