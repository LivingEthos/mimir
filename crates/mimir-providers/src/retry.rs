//! Bounded retry with exponential backoff.

use std::time::Duration;

/// Retry configuration.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Max attempts.
    pub max_attempts: u32,
    /// Initial backoff.
    pub initial_backoff_ms: u64,
    /// Max backoff.
    pub max_backoff_ms: u64,
    /// Jitter factor (0.0-1.0).
    pub jitter: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 500,
            max_backoff_ms: 8000,
            jitter: 0.2,
        }
    }
}

/// Compute backoff duration for attempt N (0-indexed).
pub fn backoff(policy: &RetryPolicy, attempt: u32) -> Duration {
    let base = policy.initial_backoff_ms * 2_u64.pow(attempt);
    let capped = base.min(policy.max_backoff_ms);
    let jitter_ms = (capped as f64 * policy.jitter * fastrand::f64()) as u64;
    Duration::from_millis(capped + jitter_ms)
}
