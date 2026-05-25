//! Prompt caching support for provider adapters.
//!
//! Provides cache control headers and hit/miss tracking for providers
//! that support prompt caching (e.g. Anthropic Claude 3.5+).

use serde::{Deserialize, Serialize};

/// Cache control policy for a message or content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CacheControl {
    /// Content should be cached (ephemeral, refreshed on each request).
    Ephemeral,
}

/// Cache status reported by the provider in the response.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CacheStatus {
    /// Whether the prompt was read from cache.
    pub cache_hit: bool,
    /// Whether new cache entries were created.
    pub cache_write: bool,
    /// Number of tokens read from cache.
    pub cache_read_tokens: u32,
    /// Number of tokens written to cache.
    pub cache_write_tokens: u32,
}

/// Extension to ProviderRequest for cache-aware requests.
#[derive(Debug, Clone, Default)]
pub struct CacheConfig {
    /// Whether to enable prompt caching for this request.
    pub enabled: bool,
    /// System prompt cache control (if enabled, system is cached).
    pub cache_system: bool,
    /// Number of recent messages to mark as ephemeral (for multi-turn caching).
    pub cache_last_n_messages: usize,
}

impl CacheConfig {
    /// Create a disabled cache config.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            cache_system: false,
            cache_last_n_messages: 0,
        }
    }

    /// Create a cache config that caches the system prompt.
    pub fn cache_system() -> Self {
        Self {
            enabled: true,
            cache_system: true,
            cache_last_n_messages: 0,
        }
    }

    /// Create a cache config that caches system + last N messages.
    pub fn cache_conversation(last_n: usize) -> Self {
        Self {
            enabled: true,
            cache_system: true,
            cache_last_n_messages: last_n,
        }
    }
}

/// Apply cache control to Anthropic request body JSON.
pub fn apply_anthropic_cache_control(body: &mut serde_json::Value, config: &CacheConfig) {
    if !config.enabled {
        return;
    }

    // Cache system prompt by wrapping in array with cache_control
    if config.cache_system {
        if let Some(system) = body.get("system").cloned() {
            if let Some(sys_str) = system.as_str() {
                body["system"] = serde_json::json!([
                    {
                        "type": "text",
                        "text": sys_str,
                        "cache_control": { "type": "ephemeral" }
                    }
                ]);
            }
        }
    }

    // Cache last N messages
    if config.cache_last_n_messages > 0 {
        if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            let total = messages.len();
            let start = total.saturating_sub(config.cache_last_n_messages);
            for msg in messages.iter_mut().skip(start) {
                if let Some(content) = msg.get("content").cloned() {
                    if content.is_string() {
                        let text = content.as_str().unwrap_or("").to_string();
                        msg["content"] = serde_json::json!([
                            {
                                "type": "text",
                                "text": text,
                                "cache_control": { "type": "ephemeral" }
                            }
                        ]);
                    }
                }
            }
        }
    }
}

/// Extract cache status from Anthropic response usage.
pub fn extract_anthropic_cache_status(usage: &serde_json::Value) -> CacheStatus {
    CacheStatus {
        cache_hit: usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 0,
        cache_write: usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 0,
        cache_read_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        cache_write_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_cache_system_prompt() {
        let mut body = json!({
            "model": "claude-sonnet-4-6",
            "system": "You are a helpful assistant.",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let config = CacheConfig::cache_system();
        apply_anthropic_cache_control(&mut body, &config);

        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "You are a helpful assistant.");
        assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn test_cache_last_n_messages() {
        let mut body = json!({
            "model": "claude-sonnet-4-6",
            "messages": [
                {"role": "user", "content": "First"},
                {"role": "assistant", "content": "Response 1"},
                {"role": "user", "content": "Second"},
            ]
        });

        let config = CacheConfig::cache_conversation(2);
        apply_anthropic_cache_control(&mut body, &config);

        let messages = body["messages"].as_array().unwrap();
        // First message unchanged
        assert_eq!(messages[0]["content"], "First");
        // Last 2 messages wrapped with cache_control
        assert!(messages[1]["content"].is_array());
        assert!(messages[2]["content"].is_array());
        assert_eq!(
            messages[1]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
        assert_eq!(
            messages[2]["content"][0]["cache_control"]["type"],
            "ephemeral"
        );
    }

    #[test]
    fn test_disabled_config_no_op() {
        let mut body = json!({
            "model": "claude-sonnet-4-6",
            "system": "You are helpful.",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let original = body.clone();
        let config = CacheConfig::disabled();
        apply_anthropic_cache_control(&mut body, &config);

        assert_eq!(body, original);
    }

    #[test]
    fn test_extract_cache_status() {
        let usage = json!({
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_creation_input_tokens": 80,
            "cache_read_input_tokens": 20
        });

        let status = extract_anthropic_cache_status(&usage);
        assert!(status.cache_hit);
        assert!(status.cache_write);
        assert_eq!(status.cache_read_tokens, 20);
        assert_eq!(status.cache_write_tokens, 80);
    }

    #[test]
    fn test_extract_cache_status_empty() {
        let usage = json!({
            "input_tokens": 100,
            "output_tokens": 50
        });

        let status = extract_anthropic_cache_status(&usage);
        assert!(!status.cache_hit);
        assert!(!status.cache_write);
        assert_eq!(status.cache_read_tokens, 0);
        assert_eq!(status.cache_write_tokens, 0);
    }
}
