//! Cost-tier mapping: subagents on Haiku, supervisor on Sonnet.

use serde::{Deserialize, Serialize};

use crate::CostTier;

/// Cost model for a provider/model combination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostModel {
    /// Model name.
    pub model: String,
    /// Provider.
    pub provider: String,
    /// Cost per 1K input tokens.
    pub input_cost_per_1k: f64,
    /// Cost per 1K output tokens.
    pub output_cost_per_1k: f64,
    /// Cost tier.
    pub tier: CostTier,
}

/// Cost ledger tracking spend across subagents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostLedger {
    /// Entries by subagent name.
    entries: Vec<LedgerEntry>,
    /// Total spend.
    total_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub subagent: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost_usd: f64,
}

impl CostLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, subagent: &str, model: &str, input: u32, output: u32, cost: f64) {
        self.entries.push(LedgerEntry {
            subagent: subagent.to_string(),
            model: model.to_string(),
            input_tokens: input,
            output_tokens: output,
            cost_usd: cost,
        });
        self.total_usd += cost;
    }

    pub fn total(&self) -> f64 {
        self.total_usd
    }

    pub fn by_subagent(&self, name: &str) -> Vec<&LedgerEntry> {
        self.entries.iter().filter(|e| e.subagent == name).collect()
    }

    pub fn by_tier(&self, tier: CostTier, models: &[CostModel]) -> f64 {
        self.entries
            .iter()
            .filter(|e| models.iter().any(|m| m.model == e.model && m.tier == tier))
            .map(|e| e.cost_usd)
            .sum()
    }

    pub fn print_summary(&self) {
        println!("┌─────────────────────────────────────────────────────────────┐");
        println!("│ Cost Ledger                                                 │");
        println!("├──────────────────┬────────────────┬───────────┬─────────────┤");
        println!("│ Subagent         │ Model          │ Tokens    │ Cost        │");
        println!("├──────────────────┼────────────────┼───────────┼─────────────┤");
        for entry in &self.entries {
            println!(
                "│ {:16} │ {:14} │ {:4}i/{:4}o │ ${:10.4} │",
                &entry.subagent[..entry.subagent.len().min(16)],
                &entry.model[..entry.model.len().min(14)],
                entry.input_tokens,
                entry.output_tokens,
                entry.cost_usd
            );
        }
        println!("├──────────────────┴────────────────┴───────────┼─────────────┤");
        println!(
            "│ Total                                         │ ${:10.4} │",
            self.total_usd
        );
        println!("└───────────────────────────────────────────────┴─────────────┘");
    }
}

/// Default cost models for Anthropic.
pub fn anthropic_cost_models() -> Vec<CostModel> {
    vec![
        CostModel {
            model: "claude-haiku-4-5".into(),
            provider: "anthropic".into(),
            input_cost_per_1k: 0.00025,
            output_cost_per_1k: 0.00125,
            tier: CostTier::Cheap,
        },
        CostModel {
            model: "claude-sonnet-4-6".into(),
            provider: "anthropic".into(),
            input_cost_per_1k: 0.003,
            output_cost_per_1k: 0.015,
            tier: CostTier::Standard,
        },
        CostModel {
            model: "claude-opus-4".into(),
            provider: "anthropic".into(),
            input_cost_per_1k: 0.015,
            output_cost_per_1k: 0.075,
            tier: CostTier::Expensive,
        },
    ]
}

/// Map a cost tier to the cheapest available model.
pub fn tier_to_model(tier: CostTier, models: &[CostModel]) -> Option<&CostModel> {
    models.iter().filter(|m| m.tier == tier).min_by(|a, b| {
        let a_total = a.input_cost_per_1k + a.output_cost_per_1k;
        let b_total = b.input_cost_per_1k + b.output_cost_per_1k;
        a_total
            .partial_cmp(&b_total)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ledger_record() {
        let mut ledger = CostLedger::new();
        ledger.record("search", "claude-haiku", 1000, 500, 0.001);
        ledger.record("reviewer", "claude-sonnet", 2000, 1000, 0.01);
        assert_eq!(ledger.total(), 0.011);
        assert_eq!(ledger.by_subagent("search").len(), 1);
    }

    #[test]
    fn test_tier_to_model() {
        let models = anthropic_cost_models();
        let cheap = tier_to_model(CostTier::Cheap, &models);
        assert!(cheap.is_some());
        assert_eq!(cheap.unwrap().model, "claude-haiku-4-5");
    }

    #[test]
    fn test_by_tier() {
        let mut ledger = CostLedger::new();
        let models = anthropic_cost_models();
        ledger.record("search", "claude-haiku-4-5", 1000, 500, 0.001);
        ledger.record("reviewer", "claude-sonnet-4-6", 2000, 1000, 0.01);
        let cheap_spend = ledger.by_tier(CostTier::Cheap, &models);
        assert_eq!(cheap_spend, 0.001);
    }
}
