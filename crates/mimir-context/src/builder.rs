//! Context packet builder.

use mimir_schemas::ContextPacket;
use mimir_runs::RunId;

/// Builder for ContextPacket.
#[derive(Debug, Default)]
pub struct ContextBuilder {
    task_card: Option<String>,
    mode: Option<String>,
    provider: Option<String>,
    model: Option<String>,
}

impl ContextBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set task card.
    pub fn task_card(mut self, card: impl Into<String>) -> Self {
        self.task_card = Some(card.into());
        self
    }

    /// Set mode.
    pub fn mode(mut self, mode: impl Into<String>) -> Self {
        self.mode = Some(mode.into());
        self
    }

    /// Set provider.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Set model.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Build the packet (stub).
    pub fn build(self) -> Result<ContextPacket, anyhow::Error> {
        let run_id = RunId::generate();
        let packet = ContextPacket {
            schema_version: 1,
            packet_id: format!("pkt-{}", run_id),
            packet_hash: "".to_string(),
            run_id: run_id.to_string(),
            task_card: self.task_card.unwrap_or_default(),
            mode: self.mode.unwrap_or_else(|| "standard".to_string()),
            cap_tokens: 64000,
            target_tokens: 32000,
            output_reserve_tokens: 4096,
            count_drift_reserve_tokens: 512,
            provider: self.provider.unwrap_or_else(|| "anthropic".to_string()),
            model: self.model.unwrap_or_else(|| "claude-sonnet-4-20250514".to_string()),
            capability_snapshot_ref: "anthropic.yaml".to_string(),
            prompt_contract_version: "1.0.0".to_string(),
            included: vec![],
            omitted_candidates: vec![],
            tool_schemas: vec![],
            evidence_cards: vec![],
            memory_entries: vec![],
            budget_ledger_ref: None,
            estimated_input_tokens: 0,
            count_provenance: "local".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            authoritative_input_tokens: None,
        };
        Ok(packet)
    }
}
