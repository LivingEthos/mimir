//! Test evidence check for review.
//!
//! Wraps mimir-edit test_runner to provide TestCard summarization.

pub use mimir_edit::test_runner::{TestFramework, TestRunResult, run_tests, detect_framework};

/// Summarize a test result into a TestCard (compact representation).
pub fn summarize_test_result(result: &TestRunResult) -> String {
    let status = if result.passed { "PASS" } else { "FAIL" };
    let framework = format!("{:?}", result.framework);
    let tests = match (result.tests_run, result.tests_failed) {
        (Some(r), Some(f)) => format!("{}/{} passed", r - f, r),
        (Some(r), None) => format!("{} run", r),
        _ => "unknown count".to_string(),
    };
    format!(
        "[{}] {} | {} | exit={}",
        status, framework, tests, result.exit_code
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarize_pass() {
        let result = TestRunResult {
            framework: TestFramework::CargoTest,
            command: "cargo test".into(),
            exit_code: 0,
            stdout: "ok".into(),
            stderr: "".into(),
            passed: true,
            timed_out: false,
            tests_run: Some(10),
            tests_failed: Some(0),
        };
        let summary = summarize_test_result(&result);
        assert!(summary.contains("PASS"));
        assert!(summary.contains("10/10 passed"));
    }

    #[test]
    fn test_summarize_fail() {
        let result = TestRunResult {
            framework: TestFramework::Pytest,
            command: "pytest".into(),
            exit_code: 1,
            stdout: "failed".into(),
            stderr: "".into(),
            passed: false,
            timed_out: false,
            tests_run: Some(5),
            tests_failed: Some(2),
        };
        let summary = summarize_test_result(&result);
        assert!(summary.contains("FAIL"));
        assert!(summary.contains("3/5 passed"));
    }
}
