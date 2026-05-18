//! Anthropic provider adapter.

use secrecy::SecretString;

/// Anthropic adapter configuration.
pub struct AnthropicAdapter {
    client: reqwest::Client,
    base_url: String,
    api_key: SecretString,
    api_version: String,
}

impl AnthropicAdapter {
    /// Create a new Anthropic adapter.
    pub fn new(api_key: SecretString) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: "https://api.anthropic.com".to_string(),
            api_key,
            api_version: "2023-06-01".to_string(),
        }
    }
}
