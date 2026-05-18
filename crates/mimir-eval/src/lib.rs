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
