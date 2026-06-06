//! Test evidence check for review.
//!
//! Wraps mimir-edit test_runner to provide TestCard summarization.

pub use mimir_edit::test_runner::{detect_framework, run_tests, TestFramework, TestRunResult};

/// Summarize a test result into a compact, failure-focused preview.
///
/// Keeps failing test names + first assertion/error line per failure,
/// drops passing-test noise, sorts failures by name for determinism.
pub fn summarize_test_result(result: &TestRunResult) -> String {
    let status = if result.passed { "PASS" } else { "FAIL" };
    let framework = format!("{:?}", result.framework);
    let tests = match (result.tests_run, result.tests_failed) {
        (Some(r), Some(f)) => format!("{}/{} passed", r - f, r),
        (Some(r), None) => format!("{} run", r),
        _ => "unknown count".to_string(),
    };

    if result.passed {
        return format!(
            "[{}] {} | {} | exit={}",
            status, framework, tests, result.exit_code
        );
    }

    let failures = extract_failures(result);
    let mut lines = vec![format!(
        "[{}] {} | {} | exit={}",
        status, framework, tests, result.exit_code
    )];

    for (name, message) in &failures {
        lines.push(format!("  FAIL: {}", name));
        if let Some(msg) = message {
            lines.push(format!("    {}", msg));
        }
    }

    let preview = lines.join("\n");
    cap_preview(&preview, 2_000)
}

fn cap_preview(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        text.to_string()
    } else {
        // Truncate on a UTF-8 char boundary so multi-byte chars in test output
        // (panic messages, non-ASCII assertions) never split mid-character.
        let mut end = cap;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n…(+{} chars)", &text[..end], text.len() - end)
    }
}

/// Extract (test_name, first_error_line) pairs, sorted by test_name.
fn extract_failures(result: &TestRunResult) -> Vec<(String, Option<String>)> {
    let combined = format!("{}\n{}", result.stdout, result.stderr);
    let mut failures: Vec<(String, Option<String>)> = Vec::new();

    // Cargo test pattern: "---- test_name stdout ----"
    // followed by error lines until next "----" or "failures:".
    let cargo_re = regex::Regex::new(r"(?m)^----\s+(\S+)\s+stdout\s+----$").unwrap();
    for cap in cargo_re.captures_iter(&combined) {
        let name = cap.get(1).unwrap().as_str().to_string();
        let start = cap.get(0).unwrap().end();
        let rest = &combined[start..];
        let end = rest
            .find("\n---- ")
            .or_else(|| rest.find("\nfailures:"))
            .or_else(|| rest.find("\nrunning "))
            .unwrap_or(rest.len());
        let block = &rest[..end];
        let first_error = block
            .lines()
            .map(str::trim)
            .find(|line| {
                !line.is_empty() && !line.starts_with("note:") && !line.starts_with("help:")
            })
            .map(|s| s.to_string());
        failures.push((name, first_error));
    }

    // Pytest pattern: "FAILED file.py::test_name - message"
    let pytest_re = regex::Regex::new(r"(?m)^FAILED\s+\S+::(\S+)\s+-\s+(.*)$").unwrap();
    for cap in pytest_re.captures_iter(&combined) {
        let name = cap.get(1).unwrap().as_str().to_string();
        let message = cap.get(2).unwrap().as_str().to_string();
        failures.push((name, Some(message)));
    }

    // Generic fallback: lines containing "test " + "failed" / "FAILED"
    if failures.is_empty() {
        for line in combined.lines() {
            let lower = line.to_lowercase();
            if lower.contains("test") && (lower.contains("failed") || lower.contains("failure")) {
                let trimmed = line.trim().to_string();
                if !trimmed.is_empty() {
                    failures.push((trimmed.clone(), None));
                }
            }
        }
    }

    // Deduplicate by name and sort.
    failures.sort_by(|a, b| a.0.cmp(&b.0));
    failures.dedup_by(|a, b| a.0 == b.0);
    failures
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

    #[test]
    fn failure_focused_cargo_test() {
        let stdout = r#"
running 3 tests
test test_a ... ok
test test_b ... FAILED
test test_c ... FAILED

failures:

---- test_b stdout ----
thread 'test_b' panicked at 'assertion failed: 1 == 2', src/lib.rs:10:5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- test_c stdout ----
thread 'test_c' panicked at 'called `Result::unwrap()` on an `Err` value', src/lib.rs:20:9
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    test_b
    test_c

test result: FAILED. 1 passed; 2 failed
"#;
        let result = TestRunResult {
            framework: TestFramework::CargoTest,
            command: "cargo test".into(),
            exit_code: 101,
            stdout: stdout.into(),
            stderr: "".into(),
            passed: false,
            timed_out: false,
            tests_run: Some(3),
            tests_failed: Some(2),
        };
        let summary = summarize_test_result(&result);
        assert!(summary.contains("FAIL"));
        assert!(summary.contains("test_b"));
        assert!(summary.contains("test_c"));
        assert!(summary.contains("assertion failed"));
        assert!(
            !summary.contains("test_a"),
            "passing tests should be dropped"
        );
        // Deterministic ordering
        let b_pos = summary.find("test_b").unwrap();
        let c_pos = summary.find("test_c").unwrap();
        assert!(b_pos < c_pos);
    }

    #[test]
    fn failure_focused_pytest() {
        let stdout = r#"
::test_a PASSED
::test_b FAILED
::test_c FAILED
"#;
        let stderr = r#"
FAILED tests/test_demo.py::test_b - AssertionError: expected 1 but got 2
FAILED tests/test_demo.py::test_c - ValueError: invalid literal
"#;
        let result = TestRunResult {
            framework: TestFramework::Pytest,
            command: "pytest".into(),
            exit_code: 1,
            stdout: stdout.into(),
            stderr: stderr.into(),
            passed: false,
            timed_out: false,
            tests_run: Some(3),
            tests_failed: Some(2),
        };
        let summary = summarize_test_result(&result);
        assert!(summary.contains("FAIL"));
        assert!(summary.contains("test_b"));
        assert!(summary.contains("test_c"));
        assert!(summary.contains("AssertionError"));
        assert!(!summary.contains("PASSED"));
    }

    #[test]
    fn preview_is_capped() {
        let mut stdout = String::new();
        for i in 0..50 {
            stdout.push_str(&format!(
                "---- test_{i} stdout ----\nthread 'test_{i}' panicked at 'this is a very long error message with lots of details about what went wrong in the test'\n\n"
            ));
        }
        let result = TestRunResult {
            framework: TestFramework::CargoTest,
            command: "cargo test".into(),
            exit_code: 1,
            stdout,
            stderr: "".into(),
            passed: false,
            timed_out: false,
            tests_run: Some(50),
            tests_failed: Some(50),
        };
        let summary = summarize_test_result(&result);
        assert!(
            summary.contains("…(+"),
            "summary should be capped: len={}",
            summary.len()
        );
    }

    #[test]
    fn cap_preview_handles_multibyte_at_boundary() {
        // 2-byte chars filling past the 2000-byte cap so the boundary lands
        // mid-character; must truncate safely instead of panicking.
        let text = "é".repeat(1500); // 3000 bytes
        let capped = cap_preview(&text, 2000);
        assert!(capped.contains("…(+"));
        assert!(capped.is_char_boundary(capped.len()));
    }
}
