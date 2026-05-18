//! Token policy: cap, reserve, drift.

/// Token policy configuration.
#[derive(Debug, Clone)]
pub struct TokenPolicy {
    /// Hard cap.
    pub cap_tokens: u32,
    /// Reserved for output.
    pub output_reserve_tokens: u32,
    /// Reserved for count drift.
    pub count_drift_reserve_tokens: u32,
}

impl Default for TokenPolicy {
    fn default() -> Self {
        Self {
            cap_tokens: 64000,
            output_reserve_tokens: 4096,
            count_drift_reserve_tokens: 512,
        }
    }
}

impl TokenPolicy {
    /// Available tokens for content.
    pub fn available(&self) -> u32 {
        self.cap_tokens.saturating_sub(self.output_reserve_tokens + self.count_drift_reserve_tokens)
    }

    /// Check if a content token count fits.
    pub fn fits(&self, content_tokens: u32) -> bool {
        let total = content_tokens + self.output_reserve_tokens + self.count_drift_reserve_tokens;
        total <= self.cap_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_available() {
        let p = TokenPolicy::default();
        assert_eq!(p.available(), 64000 - 4096 - 512);
    }

    #[test]
    fn policy_fits() {
        let p = TokenPolicy::default();
        assert!(p.fits(1000));
        assert!(p.fits(p.available()));
        assert!(!p.fits(p.available() + 1));
    }

    #[test]
    fn policy_over_cap() {
        let p = TokenPolicy {
            cap_tokens: 1000,
            output_reserve_tokens: 500,
            count_drift_reserve_tokens: 500,
        };
        assert_eq!(p.available(), 0);
        assert!(!p.fits(1));
    }
}
