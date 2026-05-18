//! Packet hashing: SHA-256 of JCS-canonicalized packet.

use mimir_schemas::ContextPacket;
use sha2::{Digest, Sha256};

/// Compute packet hash, excluding `packet_hash`, `estimated_input_tokens`, `authoritative_input_tokens`.
pub fn hash_packet(packet: &ContextPacket) -> String {
    let mut clone = packet.clone();
    clone.packet_hash.clear();
    clone.estimated_input_tokens = 0;
    clone.authoritative_input_tokens = None;
    let json = serde_json::to_string(&clone).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimir_schemas::ContextPacket;

    #[test]
    fn hash_is_stable() {
        let packet = ContextPacket {
            schema_version: 1,
            packet_id: "pkt-1".to_string(),
            packet_hash: "old".to_string(),
            run_id: "r1".to_string(),
            task_card: "test".to_string(),
            mode: "standard".to_string(),
            cap_tokens: 64000,
            target_tokens: 32000,
            output_reserve_tokens: 4096,
            count_drift_reserve_tokens: 512,
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            capability_snapshot_ref: "anthropic.yaml".to_string(),
            prompt_contract_version: "1.0.0".to_string(),
            included: vec![],
            omitted_candidates: vec![],
            tool_schemas: vec![],
            evidence_cards: vec![],
            memory_entries: vec![],
            budget_ledger_ref: None,
            estimated_input_tokens: 1234,
            count_provenance: "local".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            authoritative_input_tokens: Some(1200),
            recall_guard_flags: vec![],
        };
        let h1 = hash_packet(&packet);
        let h2 = hash_packet(&packet);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn hash_excludes_fields() {
        let mut p1 = ContextPacket {
            schema_version: 1,
            packet_id: "pkt-1".to_string(),
            packet_hash: "a".to_string(),
            run_id: "r1".to_string(),
            task_card: "test".to_string(),
            mode: "standard".to_string(),
            cap_tokens: 64000,
            target_tokens: 32000,
            output_reserve_tokens: 4096,
            count_drift_reserve_tokens: 512,
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            capability_snapshot_ref: "anthropic.yaml".to_string(),
            prompt_contract_version: "1.0.0".to_string(),
            included: vec![],
            omitted_candidates: vec![],
            tool_schemas: vec![],
            evidence_cards: vec![],
            memory_entries: vec![],
            budget_ledger_ref: None,
            estimated_input_tokens: 100,
            count_provenance: "local".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            authoritative_input_tokens: Some(99),
            recall_guard_flags: vec![],
        };
        let mut p2 = p1.clone();
        p2.packet_hash = "b".to_string();
        p2.estimated_input_tokens = 200;
        p2.authoritative_input_tokens = Some(150);
        assert_eq!(hash_packet(&p1), hash_packet(&p2));
    }
}
