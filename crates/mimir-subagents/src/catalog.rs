//! Tool schema compiler with deferred tool catalog.
//!
//! Tool schemas are compiled into a catalog. Deferred tools are not loaded
//! into context until explicitly invoked, reducing token usage.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A tool schema definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Tool name.
    pub name: String,
    /// Description for the model.
    pub description: String,
    /// JSON schema for parameters.
    pub parameters: serde_json::Value,
    /// Whether this tool is deferred (not loaded into context by default).
    pub deferred: bool,
    /// Estimated token cost of this tool's schema.
    pub schema_tokens: u32,
}

/// A compiled tool catalog.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCatalog {
    /// All registered tools.
    tools: HashMap<String, ToolSchema>,
    /// Deferred tools not currently loaded.
    deferred: Vec<String>,
    /// Token budget for tool schemas.
    schema_token_budget: u32,
    /// Current schema token usage.
    schema_tokens_used: u32,
}

impl ToolCatalog {
    pub fn new(schema_token_budget: u32) -> Self {
        Self {
            tools: HashMap::new(),
            deferred: Vec::new(),
            schema_token_budget,
            schema_tokens_used: 0,
        }
    }

    /// Register a tool. If deferred, it doesn't count against the budget.
    pub fn register(&mut self, tool: ToolSchema) {
        if tool.deferred {
            self.deferred.push(tool.name.clone());
        } else {
            self.schema_tokens_used += tool.schema_tokens;
        }
        self.tools.insert(tool.name.clone(), tool);
    }

    /// Get a tool by name (loads deferred tools on demand).
    pub fn get(&mut self, name: &str) -> Option<&ToolSchema> {
        if let Some(tool) = self.tools.get(name) {
            if tool.deferred && self.deferred.contains(&name.to_string()) {
                // Load deferred tool on first use
                self.deferred.retain(|n| n != name);
                self.schema_tokens_used += tool.schema_tokens;
            }
            self.tools.get(name)
        } else {
            None
        }
    }

    /// Get all active (non-deferred) tools.
    pub fn active_tools(&self) -> Vec<&ToolSchema> {
        self.tools
            .values()
            .filter(|t| !self.deferred.contains(&t.name))
            .collect()
    }

    /// Check if schema token budget is exceeded.
    pub fn budget_exceeded(&self) -> bool {
        self.schema_tokens_used > self.schema_token_budget
    }

    /// Defer a tool by name (move from active to deferred).
    pub fn defer(&mut self, name: &str) {
        if let Some(tool) = self.tools.get(name) {
            if !tool.deferred && !self.deferred.contains(&name.to_string()) {
                self.deferred.push(name.to_string());
                self.schema_tokens_used =
                    self.schema_tokens_used.saturating_sub(tool.schema_tokens);
            }
        }
    }

    /// Token usage summary.
    pub fn token_summary(&self) -> TokenSummary {
        TokenSummary {
            budget: self.schema_token_budget,
            used: self.schema_tokens_used,
            active: self.tools.len() - self.deferred.len(),
            deferred: self.deferred.len(),
        }
    }
}

/// Token usage summary for the tool catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSummary {
    pub budget: u32,
    pub used: u32,
    pub active: usize,
    pub deferred: usize,
}

/// Predefined tool schemas for Mimir.
pub fn default_catalog() -> ToolCatalog {
    let mut catalog = ToolCatalog::new(2000);

    catalog.register(ToolSchema {
        name: "read_file".into(),
        description: "Read a file's contents".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        }),
        deferred: false,
        schema_tokens: 50,
    });

    catalog.register(ToolSchema {
        name: "search_code".into(),
        description: "Search for patterns in code".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "path": { "type": "string" }
            },
            "required": ["query"]
        }),
        deferred: false,
        schema_tokens: 60,
    });

    catalog.register(ToolSchema {
        name: "run_tests".into(),
        description: "Run the test suite".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "framework": { "type": "string" }
            }
        }),
        deferred: true,
        schema_tokens: 80,
    });

    catalog.register(ToolSchema {
        name: "edit_file".into(),
        description: "Apply a patch to a file".into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "patch": { "type": "string" }
            },
            "required": ["path", "patch"]
        }),
        deferred: true,
        schema_tokens: 100,
    });

    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catalog_budget() {
        let mut catalog = ToolCatalog::new(200);
        catalog.register(ToolSchema {
            name: "tool1".into(),
            description: "d".into(),
            parameters: serde_json::json!({}),
            deferred: false,
            schema_tokens: 100,
        });
        catalog.register(ToolSchema {
            name: "tool2".into(),
            description: "d".into(),
            parameters: serde_json::json!({}),
            deferred: true,
            schema_tokens: 150,
        });

        assert_eq!(catalog.schema_tokens_used, 100);
        assert!(!catalog.budget_exceeded());

        // Load deferred tool
        catalog.get("tool2");
        assert_eq!(catalog.schema_tokens_used, 250);
        assert!(catalog.budget_exceeded());
    }

    #[test]
    fn test_defer_tool() {
        let mut catalog = ToolCatalog::new(1000);
        catalog.register(ToolSchema {
            name: "tool1".into(),
            description: "d".into(),
            parameters: serde_json::json!({}),
            deferred: false,
            schema_tokens: 100,
        });
        assert_eq!(catalog.active_tools().len(), 1);

        catalog.defer("tool1");
        assert_eq!(catalog.active_tools().len(), 0);
        assert_eq!(catalog.schema_tokens_used, 0);
    }

    #[test]
    fn test_default_catalog() {
        let catalog = default_catalog();
        let summary = catalog.token_summary();
        assert_eq!(summary.active, 2); // read_file, search_code
        assert_eq!(summary.deferred, 2); // run_tests, edit_file
    }
}
