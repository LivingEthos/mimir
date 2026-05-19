//! Memory Decision Engine: computes promotion scores for memory entries.
//!
//! The engine scores entries based on five dimensions:
//! - severity: how bad was the failure / how important is the lesson
//! - recurrence: how often has this pattern appeared
//! - success_rate: empirical success after applying the lesson
//! - task_relevance: how relevant to current task domain
//! - token_savings: estimated tokens saved by including this memory

use mimir_schemas::{MemoryEntry, PromotionBreakdown};

/// Weights for the scoring formula.
///
/// Defaults tuned for general-purpose coding agents.
#[derive(Debug, Clone, Copy)]
pub struct ScoreWeights {
    /// Weight for severity (0.0–1.0).
    pub severity: f64,
    /// Weight for recurrence (0.0–1.0).
    pub recurrence: f64,
    /// Weight for success rate (0.0–1.0).
    pub success_rate: f64,
    /// Weight for task relevance (0.0–1.0).
    pub task_relevance: f64,
    /// Weight for token savings (0.0–1.0).
    pub token_savings: f64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            severity: 0.30,
            recurrence: 0.25,
            success_rate: 0.20,
            task_relevance: 0.15,
            token_savings: 0.10,
        }
    }
}

/// Input signals for scoring a memory entry.
#[derive(Debug, Clone)]
pub struct ScoreSignals {
    /// Severity of the underlying issue (0.0–1.0).
    pub severity: f64,
    /// Recurrence count (number of times observed).
    pub recurrence_count: u32,
    /// Success rate after applying the lesson (0.0–1.0).
    pub success_rate: f64,
    /// Task relevance score (0.0–1.0).
    pub task_relevance: f64,
    /// Estimated token savings per inclusion.
    pub token_savings: u32,
}

impl Default for ScoreSignals {
    fn default() -> Self {
        Self {
            severity: 0.5,
            recurrence_count: 1,
            success_rate: 0.5,
            task_relevance: 0.5,
            token_savings: 0,
        }
    }
}

/// Decision engine that scores memory entries.
#[derive(Debug, Clone)]
pub struct MemoryDecisionEngine {
    weights: ScoreWeights,
}

impl MemoryDecisionEngine {
    /// Create a new engine with default weights.
    #[must_use]
    pub fn new() -> Self {
        Self {
            weights: ScoreWeights::default(),
        }
    }

    /// Create a new engine with custom weights.
    #[must_use]
    pub fn with_weights(weights: ScoreWeights) -> Self {
        Self { weights }
    }

    /// Compute the promotion score and breakdown for a memory entry.
    ///
    /// The formula is a weighted sum of normalized signals:
    ///
    /// ```text
    /// score = w_sev * severity
    ///       + w_rec * min(recurrence_count / 5, 1.0)
    ///       + w_suc * success_rate
    ///       + w_rel * task_relevance
    ///       + w_tok * min(token_savings / 500, 1.0)
    /// ```
    ///
    /// # Panics
    /// Panics if any weight is negative or if the total weight is zero.
    #[must_use]
    pub fn score(&self, signals: &ScoreSignals) -> PromotionBreakdown {
        assert!(
            self.weights.severity >= 0.0
                && self.weights.recurrence >= 0.0
                && self.weights.success_rate >= 0.0
                && self.weights.task_relevance >= 0.0
                && self.weights.token_savings >= 0.0,
            "weights must be non-negative"
        );
        let total_weight = self.weights.severity
            + self.weights.recurrence
            + self.weights.success_rate
            + self.weights.task_relevance
            + self.weights.token_savings;
        assert!(total_weight > 0.0, "total weight must be > 0");

        let recurrence_norm = (f64::from(signals.recurrence_count) / 5.0).min(1.0);
        let token_savings_norm = (f64::from(signals.token_savings) / 500.0).min(1.0);

        let severity_score = self.weights.severity * signals.severity;
        let recurrence_score = self.weights.recurrence * recurrence_norm;
        let success_rate_score = self.weights.success_rate * signals.success_rate;
        let task_relevance_score = self.weights.task_relevance * signals.task_relevance;
        let token_savings_score = self.weights.token_savings * token_savings_norm;

        let total = (severity_score
            + recurrence_score
            + success_rate_score
            + task_relevance_score
            + token_savings_score)
            / total_weight;

        PromotionBreakdown {
            severity_score,
            recurrence_score,
            success_rate_score,
            task_relevance_score,
            token_savings_score,
            total,
        }
    }

    /// Score an entry in-place, updating its `promotion_score` and `promotion_breakdown`.
    pub fn score_entry(&self, entry: &mut MemoryEntry, signals: &ScoreSignals) {
        let breakdown = self.score(signals);
        entry.promotion_score = Some(breakdown.total);
        entry.promotion_breakdown = Some(breakdown);
    }

    /// Return the default promotion threshold (entries above this are promoted).
    #[must_use]
    pub const fn promotion_threshold() -> f64 {
        0.65
    }

    /// Return the default safe-to-send threshold.
    #[must_use]
    pub const fn safe_to_send_threshold() -> f64 {
        0.80
    }

    /// Decide whether an entry should be promoted based on its score.
    #[must_use]
    pub fn should_promote(&self, entry: &MemoryEntry) -> bool {
        entry
            .promotion_score
            .map_or(false, |s| s >= Self::promotion_threshold())
    }

    /// Decide whether an entry is safe to send to the model.
    #[must_use]
    pub fn is_safe_to_send(&self, entry: &MemoryEntry) -> bool {
        entry.safe_to_send
            && entry
                .promotion_score
                .map_or(false, |s| s >= Self::safe_to_send_threshold())
    }
}

impl Default for MemoryDecisionEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_score() {
        let engine = MemoryDecisionEngine::new();
        let signals = ScoreSignals::default();
        let score = engine.score(&signals);

        // severity=0.5 * 0.30 = 0.15
        // recurrence=1/5 * 0.25 = 0.05
        // success=0.5 * 0.20 = 0.10
        // relevance=0.5 * 0.15 = 0.075
        // tokens=0/500 * 0.10 = 0.0
        // total = (0.15+0.05+0.10+0.075+0.0) / 1.0 = 0.375
        assert!((score.severity_score - 0.15).abs() < 0.001);
        assert!((score.recurrence_score - 0.05).abs() < 0.001);
        assert!((score.total - 0.375).abs() < 0.001);
    }

    #[test]
    fn test_high_score() {
        let engine = MemoryDecisionEngine::new();
        let signals = ScoreSignals {
            severity: 1.0,
            recurrence_count: 10,
            success_rate: 1.0,
            task_relevance: 1.0,
            token_savings: 1000,
        };
        let score = engine.score(&signals);
        assert!(score.total >= 0.99);
    }

    #[test]
    fn test_custom_weights() {
        let engine = MemoryDecisionEngine::with_weights(ScoreWeights {
            severity: 0.0,
            recurrence: 0.0,
            success_rate: 1.0,
            task_relevance: 0.0,
            token_savings: 0.0,
        });
        let signals = ScoreSignals {
            severity: 1.0,
            recurrence_count: 100,
            success_rate: 0.5,
            task_relevance: 0.0,
            token_savings: 0,
        };
        let score = engine.score(&signals);
        assert!((score.total - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_should_promote() {
        let engine = MemoryDecisionEngine::new();
        let mut entry = MemoryEntry {
            schema_version: 1,
            entry_id: "e1".to_string(),
            kind: "lesson".to_string(),
            body: "test".to_string(),
            source_evidence: vec![],
            confidence: "proposed".to_string(),
            promotion_score: Some(0.7),
            promotion_breakdown: None,
            scope: "project".to_string(),
            safe_to_send: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            last_verified_at: None,
            staleness_policy: None,
            retrieval_tags: vec![],
            imported_from: None,
        };
        assert!(engine.should_promote(&entry));

        entry.promotion_score = Some(0.5);
        assert!(!engine.should_promote(&entry));
    }

    #[test]
    fn test_is_safe_to_send() {
        let engine = MemoryDecisionEngine::new();
        let mut entry = MemoryEntry {
            schema_version: 1,
            entry_id: "e1".to_string(),
            kind: "lesson".to_string(),
            body: "test".to_string(),
            source_evidence: vec![],
            confidence: "verified".to_string(),
            promotion_score: Some(0.85),
            promotion_breakdown: None,
            scope: "project".to_string(),
            safe_to_send: true,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            last_verified_at: None,
            staleness_policy: None,
            retrieval_tags: vec![],
            imported_from: None,
        };
        assert!(engine.is_safe_to_send(&entry));

        entry.safe_to_send = false;
        assert!(!engine.is_safe_to_send(&entry));

        entry.safe_to_send = true;
        entry.promotion_score = Some(0.7);
        assert!(!engine.is_safe_to_send(&entry));
    }

    #[test]
    fn test_score_entry() {
        let engine = MemoryDecisionEngine::new();
        let mut entry = MemoryEntry {
            schema_version: 1,
            entry_id: "e1".to_string(),
            kind: "lesson".to_string(),
            body: "test".to_string(),
            source_evidence: vec![],
            confidence: "proposed".to_string(),
            promotion_score: None,
            promotion_breakdown: None,
            scope: "project".to_string(),
            safe_to_send: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            last_verified_at: None,
            staleness_policy: None,
            retrieval_tags: vec![],
            imported_from: None,
        };
        let signals = ScoreSignals {
            severity: 1.0,
            recurrence_count: 5,
            success_rate: 1.0,
            task_relevance: 1.0,
            token_savings: 500,
        };
        engine.score_entry(&mut entry, &signals);
        assert!(entry.promotion_score.unwrap() > 0.9);
        assert!(entry.promotion_breakdown.is_some());
    }
}
