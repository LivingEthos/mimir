//! Packet hashing: SHA-256 of JCS-canonicalized packet.

use mimir_schemas::ContextPacket;
use sha2::{Digest, Sha256};

/// Compute packet hash, excluding `packet_hash`, `estimated_input_tokens`, `authoritative_input_tokens`.
pub fn hash_packet(packet: &ContextPacket) -> String {
    let mut value = serde_json::to_value(packet).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.remove("packet_hash");
        object.remove("estimated_input_tokens");
        object.remove("authoritative_input_tokens");
    }
    let json = canonical_json(&value);
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => serde_json::to_string(value).unwrap_or_default(),
        serde_json::Value::Array(items) => {
            let items = items.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        serde_json::Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let entries = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", entries.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mimir_schemas::{ContextPacket, TaskCard};

    fn sample_packet() -> ContextPacket {
        ContextPacket {
            schema_version: 1,
            packet_id: "pkt-1".to_string(),
            packet_hash: "old".to_string(),
            run_id: "20260101-000000-00000001".to_string(),
            task_card: TaskCard {
                goal: "test".to_string(),
                acceptance_criteria: Vec::new(),
                likely_files: Vec::new(),
                risk_level: None,
                expected_test_command: None,
                unknowns: Vec::new(),
                need_for_large_context: None,
                complexity: "tiny".to_string(),
            },
            mode: "ask".to_string(),
            cap_tokens: 64000,
            target_tokens: 32000,
            output_reserve_tokens: 4096,
            count_drift_reserve_tokens: 512,
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            capability_snapshot_ref: "anthropic.yaml".to_string(),
            prompt_contract_version: 1,
            included: vec![],
            omitted_candidates: vec![],
            tool_schemas: vec![],
            evidence_cards: vec![],
            memory_entries: vec![],
            budget_ledger_ref: ".mimir/runs/20260101-000000-00000001/budget_ledger.json"
                .to_string(),
            estimated_input_tokens: 1234,
            count_provenance: "local_estimate_only".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            authoritative_input_tokens: Some(1200),
            recall_guard_flags: vec![],
        }
    }

    #[test]
    fn hash_is_stable() {
        let packet = sample_packet();
        let h1 = hash_packet(&packet);
        let h2 = hash_packet(&packet);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn hash_excludes_fields() {
        let mut p1 = sample_packet();
        p1.packet_hash = "a".to_string();
        p1.estimated_input_tokens = 100;
        p1.authoritative_input_tokens = Some(99);
        let mut p2 = p1.clone();
        p2.packet_hash = "b".to_string();
        p2.estimated_input_tokens = 200;
        p2.authoritative_input_tokens = Some(150);
        assert_eq!(hash_packet(&p1), hash_packet(&p2));
    }
}
