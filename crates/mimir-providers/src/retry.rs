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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_policy_default() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.max_attempts, 3);
        assert_eq!(policy.initial_backoff_ms, 500);
        assert_eq!(policy.max_backoff_ms, 8000);
    }

    #[test]
    fn test_backoff_increases_with_attempt() {
        let policy = RetryPolicy {
            jitter: 0.0,
            ..Default::default()
        };
        let b0 = backoff(&policy, 0);
        let b1 = backoff(&policy, 1);
        let b2 = backoff(&policy, 2);
        assert!(b1 > b0);
        assert!(b2 > b1);
    }

    #[test]
    fn test_backoff_respects_max() {
        let policy = RetryPolicy {
            max_backoff_ms: 100,
            jitter: 0.0,
            ..Default::default()
        };
        let b = backoff(&policy, 10);
        assert_eq!(b, Duration::from_millis(100));
    }

    #[test]
    fn test_backoff_with_jitter() {
        let policy = RetryPolicy {
            jitter: 0.5,
            ..Default::default()
        };
        let b = backoff(&policy, 0);
        // With jitter 0.5, backoff should be between 500 and 750
        assert!(b >= Duration::from_millis(500));
        assert!(b <= Duration::from_millis(750));
    }
}
