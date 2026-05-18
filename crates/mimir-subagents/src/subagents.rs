//! Read-only LLM subagent stubs.
//!
//! search, file-analyst-llm, reviewer, test-summarizer

use crate::{EvidenceSummary, Result, SubagentDef, SubagentError};

/// Registry of available subagents.
pub struct SubagentRegistry {
    agents: Vec<SubagentDef>,
}

impl Default for SubagentRegistry {
    fn default() -> Self {
        Self {
            agents: vec![
                SubagentDef {
                    name: "file-analyst".into(),
                    description: "Deterministic file analysis (no LLM)".into(),
                    uses_llm: false,
                    cost_tier: crate::CostTier::Free,
                    token_cap: 0,
                    allowed_tools: vec![],
                    read_only: true,
                },
                SubagentDef {
                    name: "search".into(),
                    description: "Semantic and lexical search across codebase".into(),
                    uses_llm: false,
                    cost_tier: crate::CostTier::Free,
                    token_cap: 1000,
                    allowed_tools: vec!["grep".into(), "find".into()],
                    read_only: true,
                },
                SubagentDef {
                    name: "file-analyst-llm".into(),
                    description: "LLM-powered file analysis for complex patterns".into(),
                    uses_llm: true,
                    cost_tier: crate::CostTier::Cheap,
                    token_cap: 8000,
                    allowed_tools: vec!["read_file".into()],
                    read_only: true,
                },
                SubagentDef {
                    name: "reviewer".into(),
                    description: "Code review subagent".into(),
                    uses_llm: true,
                    cost_tier: crate::CostTier::Standard,
                    token_cap: 16000,
                    allowed_tools: vec!["read_file".into(), "diff".into()],
                    read_only: true,
                },
                SubagentDef {
                    name: "test-summarizer".into(),
                    description: "Summarize test output into TestCard".into(),
                    uses_llm: true,
                    cost_tier: crate::CostTier::Cheap,
                    token_cap: 4000,
                    allowed_tools: vec!["read_file".into()],
                    read_only: true,
                },
            ],
        }
    }
}

impl SubagentRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn get(&self, name: &str) -> Option<&SubagentDef> {
        self.agents.iter().find(|a| a.name == name)
    }

    pub fn list(&self) -> &[SubagentDef] {
        &self.agents
    }

    pub fn by_tier(&self, tier: crate::CostTier) -> Vec<&SubagentDef> {
        self.agents.iter().filter(|a| a.cost_tier == tier).collect()
    }
}

/// Execute a subagent by name (stub — returns simulated evidence).
pub fn execute_stub(name: &str, query: &str, parent_run_id: Option<&str>) -> Result<EvidenceSummary> {
    let registry = SubagentRegistry::new();
    let def = registry.get(name)
        .ok_or_else(|| SubagentError::UnknownSubagent(name.to_string()))?;

    if def.read_only {
        // In a real implementation, this would call the provider gateway
        // with the subagent's token cap enforced.
    }

    // Simulated evidence for testing
    Ok(EvidenceSummary {
        subagent: name.to_string(),
        query: query.to_string(),
        findings: vec![format!("Stub result for '{}' on query '{}'", name, query)],
        relevant_paths: vec!["src/lib.rs".into()],
        confidence: 0.8,
        tokens_consumed: 100,
        cost_usd: if def.uses_llm { 0.01 } else { 0.0 },
        parent_run_id: parent_run_id.map(|s| s.to_string()),
        run_id: format!("{}-{}", name, uuid::Uuid::new_v4()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_lookup() {
        let reg = SubagentRegistry::new();
        assert!(reg.get("file-analyst").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_by_tier() {
        let reg = SubagentRegistry::new();
        let free = reg.by_tier(crate::CostTier::Free);
        assert_eq!(free.len(), 2); // file-analyst, search
    }

    #[test]
    fn test_execute_stub() {
        let result = execute_stub("file-analyst", "find panics", Some("run-1")).unwrap();
        assert_eq!(result.subagent, "file-analyst");
        assert_eq!(result.cost_usd, 0.0);
        assert_eq!(result.parent_run_id, Some("run-1".into()));
    }

    #[test]
    fn test_execute_unknown_subagent() {
        let err = execute_stub("unknown", "q", None).unwrap_err();
        assert!(matches!(err, SubagentError::UnknownSubagent(_)));
    }
}
