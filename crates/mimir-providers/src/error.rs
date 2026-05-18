//! Provider adapter errors.
//!
//! Maps provider-native errors to structured Mimir error codes.

use std::fmt;

/// A provider adapter error.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderError {
    /// Error code (e.g. `provider_rate_limited`).
    pub code: String,
    /// Human-readable message.
    pub message: String,
    /// HTTP status code, if applicable.
    pub status: Option<u16>,
    /// Whether this error is retryable.
    pub retryable: bool,
}

impl ProviderError {
    /// Create a new provider error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            status: None,
            retryable: false,
        }
    }

    /// Set the HTTP status code.
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Mark as retryable.
    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProviderError {}

/// Convenience type alias for adapter results.
pub type Result<T> = std::result::Result<T, ProviderError>;

/// Map an Anthropic error type + HTTP status to a `ProviderError`.
///
/// Per 26-FIRST-PROVIDER-SPEC.md:
/// | Anthropic error            | Mimir `Error.code`           |
/// |----------------------------|------------------------------|
/// | invalid_request_error      | provider_invalid_request     |
/// | authentication_error       | provider_unauthorized        |
/// | permission_error           | provider_forbidden           |
/// | not_found_error            | provider_not_found           |
/// | request_too_large          | provider_request_too_large   |
/// | rate_limit_error           | provider_rate_limited        |
/// | overloaded_error           | provider_overloaded          |
/// | api_error                  | provider_internal_error      |
pub fn map_anthropic_error(error_type: &str, message: &str, status: u16) -> ProviderError {
    let (code, retryable) = match error_type {
        "invalid_request_error" => ("provider_invalid_request", false),
        "authentication_error" => ("provider_unauthorized", false),
        "permission_error" => ("provider_forbidden", false),
        "not_found_error" => ("provider_not_found", false),
        "request_too_large" => ("provider_request_too_large", false),
        "rate_limit_error" => ("provider_rate_limited", true),
        "overloaded_error" => ("provider_overloaded", true),
        "api_error" => ("provider_internal_error", true),
        _ => {
            // Fallback: decide retryability by HTTP status.
            let retryable = matches!(status, 429 | 529 | 500..=599);
            ("provider_internal_error", retryable)
        }
    };

    ProviderError::new(code, message).with_status(status).retryable_if(retryable)
}

/// Map a raw HTTP status (no parsed error body) to a `ProviderError`.
pub fn map_http_status(status: u16, message: &str) -> ProviderError {
    let (code, retryable) = match status {
        400 => ("provider_invalid_request", false),
        401 => ("provider_unauthorized", false),
        403 => ("provider_forbidden", false),
        404 => ("provider_not_found", false),
        413 => ("provider_request_too_large", false),
        429 => ("provider_rate_limited", true),
        529 => ("provider_overloaded", true),
        408 => ("provider_timeout", true),
        500..=599 => ("provider_internal_error", true),
        _ => ("provider_internal_error", false),
    };

    ProviderError::new(code, message).with_status(status).retryable_if(retryable)
}

impl ProviderError {
    fn retryable_if(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}
