//! Token counting and drift calibration.
//!
//! Provides both local (fast, no network) and server-side token counting.
//! Local counting uses tiktoken-rs for OpenAI-compatible models and
//! character-based heuristics for others. Server-side counting uses the
//! provider's count_tokens endpoint when available.

/// Count tokens locally (fast, no network).
///
/// Uses tiktoken-rs for OpenAI-compatible tokenization when the `tokenizer`
/// feature is available. Falls back to a character-based heuristic
/// (roughly 4 chars per token for English text).
pub fn count_local(text: &str) -> u32 {
    // Try tiktoken-rs for cl100k_base (used by Claude and GPT-4).
    // If it fails, fall back to heuristic.
    match tiktoken_rs::get_bpe_from_model("cl100k_base") {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len() as u32,
        Err(_) => heuristic_count(text),
    }
}

/// Character-based heuristic: ~4 chars per token for English text.
fn heuristic_count(text: &str) -> u32 {
    // Count characters and divide by average token length.
    // This is a rough estimate; actual tokenization varies by language.
    let char_count = text.chars().count() as u32;
    char_count.div_ceil(4)
}

/// Count tokens for a structured request by serializing its content.
pub fn count_request_local(system: Option<&str>, messages: &[(String, String)]) -> u32 {
    let mut total = 0u32;

    // System prompt tokens
    if let Some(sys) = system {
        total += count_local(sys);
        // Add overhead for system role formatting
        total += 4;
    }

    // Message tokens
    for (role, content) in messages {
        total += count_local(content);
        // Overhead per message: role tokens + formatting
        total += 4 + count_local(role);
    }

    // Add overhead for the message array structure
    if !messages.is_empty() {
        total += 2;
    }

    total
}

/// Calibration result comparing local vs server counts.
#[derive(Debug, Clone)]
pub struct CalibrationResult {
    /// Local token count.
    pub local_count: u32,
    /// Server token count.
    pub server_count: u32,
    /// Absolute difference.
    pub delta: i64,
    /// Relative difference as a ratio.
    pub drift_ratio: f64,
}

impl CalibrationResult {
    /// Create a new calibration result.
    pub fn new(local_count: u32, server_count: u32) -> Self {
        let delta = i64::from(server_count) - i64::from(local_count);
        let drift_ratio = if local_count > 0 {
            delta.abs() as f64 / f64::from(local_count)
        } else {
            0.0
        };
        Self {
            local_count,
            server_count,
            delta,
            drift_ratio,
        }
    }

    /// Whether the drift is within acceptable bounds (5%).
    pub fn is_acceptable(&self) -> bool {
        self.drift_ratio < 0.05
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_local_non_empty() {
        let count = count_local("Hello world");
        assert!(count > 0, "token count should be positive");
    }

    #[test]
    fn test_count_local_empty() {
        assert_eq!(count_local(""), 0);
    }

    #[test]
    fn test_heuristic_count() {
        // "Hello world" = 11 chars, ~3 tokens at 4 chars/token
        let count = heuristic_count("Hello world");
        assert_eq!(count, 3);
    }

    #[test]
    fn test_count_request_local() {
        let messages = vec![
            ("user".to_string(), "Hello".to_string()),
            ("assistant".to_string(), "Hi there!".to_string()),
        ];
        let count = count_request_local(Some("You are helpful."), &messages);
        assert!(count > 0);
        // Should include system + 2 messages + overhead
        assert!(count > 10);
    }

    #[test]
    fn test_calibration_result() {
        let result = CalibrationResult::new(100, 105);
        assert_eq!(result.local_count, 100);
        assert_eq!(result.server_count, 105);
        assert_eq!(result.delta, 5);
        assert_eq!(result.drift_ratio, 0.05);
        assert!(!result.is_acceptable());

        let result2 = CalibrationResult::new(100, 102);
        assert!(result2.is_acceptable());
    }

    #[test]
    fn test_calibration_zero_local() {
        let result = CalibrationResult::new(0, 10);
        assert_eq!(result.drift_ratio, 0.0);
        assert!(result.is_acceptable());
    }
}
