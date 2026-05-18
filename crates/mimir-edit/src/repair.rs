//! Bounded repair loop for the edit engine.
//!
//! Runs a test-verify-retry cycle with a configurable turn limit and cost cap.

use serde::{Deserialize, Serialize};

use crate::test_runner::{TestRunResult, TestFramework};
use crate::{EditError, Result};

/// Configuration for the repair loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairConfig {
    /// Maximum number of repair turns.
    pub max_repair_turns: u32,
    /// Maximum cost in dollars for the repair loop.
    pub cost_cap_dollars: f64,
    /// Whether to stop on first success.
    pub stop_on_success: bool,
}

impl Default for RepairConfig {
    fn default() -> Self {
        Self {
            max_repair_turns: 3,
            cost_cap_dollars: 5.0,
            stop_on_success: true,
        }
    }
}

/// A single turn in the repair loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairTurn {
    /// Turn number (1-based).
    pub turn: u32,
    /// Test result from this turn.
    pub test_result: TestRunResult,
    /// Cost of this turn (estimated).
    pub estimated_cost: f64,
    /// Patch steps applied in this turn.
    pub patch_steps: Vec<mimir_schemas::PatchStep>,
}

/// Result of the full repair loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairResult {
    /// Whether the repair converged on success.
    pub converged: bool,
    /// Number of turns executed.
    pub turns_executed: u32,
    /// Total estimated cost.
    pub total_cost: f64,
    /// All turns executed.
    pub turns: Vec<RepairTurn>,
    /// Final test result.
    pub final_test: Option<TestRunResult>,
    /// Stop reason.
    pub stop_reason: String,
}

/// Execute a bounded repair loop.
///
/// The `run_tests` closure is called each turn to get the current test result.
/// The `apply_patches` closure is called when tests fail to apply repair patches.
pub fn run_repair_loop<F, G>(
    config: &RepairConfig,
    mut run_tests: F,
    mut apply_patches: G,
) -> RepairResult
where
    F: FnMut() -> Result<TestRunResult>,
    G: FnMut(&TestRunResult) -> Vec<mimir_schemas::PatchStep>,
{
    let mut turns = Vec::new();
    let mut total_cost = 0.0;

    for turn_num in 1..=config.max_repair_turns {
        let test_result = match run_tests() {
            Ok(r) => r,
            Err(e) => {
                return RepairResult {
                    converged: false,
                    turns_executed: turn_num - 1,
                    total_cost,
                    turns: turns.clone(),
                    final_test: None,
                    stop_reason: format!("test_runner_error: {}", e),
                };
            }
        };

        if test_result.passed && config.stop_on_success {
            return RepairResult {
                converged: true,
                turns_executed: turn_num,
                total_cost,
                turns: turns.clone(),
                final_test: Some(test_result),
                stop_reason: "tests_passed".to_string(),
            };
        }

        let steps = apply_patches(&test_result);
        let estimated_cost = 0.50; // Placeholder: $0.50 per turn
        total_cost += estimated_cost;

        if total_cost > config.cost_cap_dollars {
            return RepairResult {
                converged: false,
                turns_executed: turn_num,
                total_cost,
                turns: turns.clone(),
                final_test: Some(test_result),
                stop_reason: "cost_cap_exceeded".to_string(),
            };
        }

        turns.push(RepairTurn {
            turn: turn_num,
            test_result,
            estimated_cost,
            patch_steps: steps,
        });
    }

    RepairResult {
        converged: false,
        turns_executed: config.max_repair_turns,
        total_cost,
        turns: turns.clone(),
        final_test: turns.last().map(|t| t.test_result.clone()),
        stop_reason: "max_repair_turns_reached".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repair_loop_converges_on_first_success() {
        let config = RepairConfig {
            max_repair_turns: 3,
            cost_cap_dollars: 10.0,
            stop_on_success: true,
        };

        let mut call_count = 0;
        let result = run_repair_loop(
            &config,
            || {
                call_count += 1;
                Ok(TestRunResult {
                    framework: TestFramework::CargoTest,
                    command: "cargo test".into(),
                    exit_code: 0,
                    stdout: "ok".into(),
                    stderr: "".into(),
                    passed: true,
                    tests_run: Some(1),
                    tests_failed: Some(0),
                })
            },
            |_test| vec![],
        );

        assert!(result.converged);
        assert_eq!(result.turns_executed, 1);
        assert_eq!(call_count, 1);
        assert_eq!(result.stop_reason, "tests_passed");
    }

    #[test]
    fn test_repair_loop_hits_max_turns() {
        let config = RepairConfig {
            max_repair_turns: 2,
            cost_cap_dollars: 10.0,
            stop_on_success: true,
        };

        let result = run_repair_loop(
            &config,
            || {
                Ok(TestRunResult {
                    framework: TestFramework::CargoTest,
                    command: "cargo test".into(),
                    exit_code: 1,
                    stdout: "failed".into(),
                    stderr: "".into(),
                    passed: false,
                    tests_run: Some(1),
                    tests_failed: Some(1),
                })
            },
            |_test| vec![],
        );

        assert!(!result.converged);
        assert_eq!(result.turns_executed, 2);
        assert_eq!(result.stop_reason, "max_repair_turns_reached");
    }

    #[test]
    fn test_repair_loop_cost_cap() {
        let config = RepairConfig {
            max_repair_turns: 10,
            cost_cap_dollars: 0.75,
            stop_on_success: true,
        };

        let result = run_repair_loop(
            &config,
            || {
                Ok(TestRunResult {
                    framework: TestFramework::CargoTest,
                    command: "cargo test".into(),
                    exit_code: 1,
                    stdout: "failed".into(),
                    stderr: "".into(),
                    passed: false,
                    tests_run: Some(1),
                    tests_failed: Some(1),
                })
            },
            |_test| vec![],
        );

        assert!(!result.converged);
        assert_eq!(result.stop_reason, "cost_cap_exceeded");
        assert_eq!(result.turns_executed, 2); // 2nd turn exceeds cap
    }
}
