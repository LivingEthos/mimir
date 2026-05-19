//! Eval harness: run eval cases, produce results.

use mimir_schemas::{EvalCase, EvalResult};

/// Run a single eval case (stub).
pub fn run_case(case: &EvalCase) -> EvalResult {
    EvalResult {
        case_id: case.case_id.clone(),
        passed: true,
        score: 1.0,
        notes: "stub".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_case_passes() {
        let case = EvalCase {
            case_id: "test-1".to_string(),
            description: "A test case".to_string(),
            input: serde_json::json!({"x": 1}),
            expected: serde_json::json!({"y": 2}),
        };
        let result = run_case(&case);
        assert_eq!(result.case_id, "test-1");
        assert!(result.passed);
        assert_eq!(result.score, 1.0);
    }

    #[test]
    fn test_run_case_preserves_id() {
        let case = EvalCase {
            case_id: "edge-case-empty".to_string(),
            description: "".to_string(),
            input: serde_json::Value::Null,
            expected: serde_json::Value::Null,
        };
        let result = run_case(&case);
        assert_eq!(result.case_id, "edge-case-empty");
    }
}
