//! Task router: routes tasks to appropriate subagents based on task type.
//!
//! See 14-LEARNING-LAYER.md for task routing strategy.

use crate::{CostTier, SubagentDef, SubagentError};

/// A task to be routed.
#[derive(Debug, Clone)]
pub struct Task {
    /// Task description.
    pub description: String,
    /// Task category hint.
    pub category: TaskCategory,
    /// Estimated complexity (1-10).
    pub complexity: u8,
    /// Whether the task requires file mutation.
    pub requires_mutation: bool,
}

/// Task categories for routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCategory {
    Search,
    Analysis,
    Review,
    Test,
    Edit,
    Unknown,
}

/// Routes tasks to subagents.
pub struct TaskRouter;

impl TaskRouter {
    /// Route a task to the best subagent.
    pub fn route(task: &Task) -> Option<&'static str> {
        match task.category {
            TaskCategory::Search => Some("search"),
            TaskCategory::Analysis => {
                if task.complexity <= 3 {
                    Some("file-analyst") // deterministic
                } else {
                    Some("file-analyst-llm") // LLM for complex analysis
                }
            }
            TaskCategory::Review => Some("reviewer"),
            TaskCategory::Test => Some("test-summarizer"),
            TaskCategory::Edit => {
                if task.requires_mutation {
                    None // No read-only subagent can edit
                } else {
                    Some("reviewer")
                }
            }
            TaskCategory::Unknown => None,
        }
    }

    /// Route with cost-tier awareness.
    pub fn route_with_tier(task: &Task, budget_remaining: f64) -> Option<&'static str> {
        let agent = Self::route(task)?;

        // If budget is tight, prefer cheaper alternatives
        if budget_remaining < 0.5 {
            match task.category {
                TaskCategory::Analysis => return Some("file-analyst"),
                TaskCategory::Review => return Some("file-analyst"),
                _ => {}
            }
        }

        Some(agent)
    }

    /// Get the cost tier for a routed task.
    pub fn tier_for_task(task: &Task) -> CostTier {
        match task.category {
            TaskCategory::Search => CostTier::Free,
            TaskCategory::Analysis => {
                if task.complexity <= 3 {
                    CostTier::Free
                } else {
                    CostTier::Cheap
                }
            }
            TaskCategory::Review => CostTier::Standard,
            TaskCategory::Test => CostTier::Cheap,
            TaskCategory::Edit => CostTier::Standard,
            TaskCategory::Unknown => CostTier::Standard,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_search() {
        let task = Task {
            description: "Find all usages".into(),
            category: TaskCategory::Search,
            complexity: 2,
            requires_mutation: false,
        };
        assert_eq!(TaskRouter::route(&task), Some("search"));
    }

    #[test]
    fn test_route_simple_analysis() {
        let task = Task {
            description: "Count functions".into(),
            category: TaskCategory::Analysis,
            complexity: 2,
            requires_mutation: false,
        };
        assert_eq!(TaskRouter::route(&task), Some("file-analyst"));
    }

    #[test]
    fn test_route_complex_analysis() {
        let task = Task {
            description: "Analyze architecture".into(),
            category: TaskCategory::Analysis,
            complexity: 8,
            requires_mutation: false,
        };
        assert_eq!(TaskRouter::route(&task), Some("file-analyst-llm"));
    }

    #[test]
    fn test_route_edit_mutation() {
        let task = Task {
            description: "Fix the bug".into(),
            category: TaskCategory::Edit,
            complexity: 5,
            requires_mutation: true,
        };
        assert_eq!(TaskRouter::route(&task), None);
    }

    #[test]
    fn test_route_with_tier_low_budget() {
        let task = Task {
            description: "Analyze".into(),
            category: TaskCategory::Analysis,
            complexity: 8,
            requires_mutation: false,
        };
        // With low budget, complex analysis falls back to deterministic
        assert_eq!(TaskRouter::route_with_tier(&task, 0.1), Some("file-analyst"));
        // With high budget, uses LLM
        assert_eq!(TaskRouter::route_with_tier(&task, 10.0), Some("file-analyst-llm"));
    }
}
