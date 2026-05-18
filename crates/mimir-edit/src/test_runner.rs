//! Focused test runner with auto-detection.

use std::process::Command;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{EditError, Result};

/// Detected test framework.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum TestFramework {
    Pytest,
    Vitest,
    Jest,
    Mocha,
    CargoTest,
    Unknown,
}

/// Test run result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunResult {
    pub framework: TestFramework,
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub passed: bool,
    pub tests_run: Option<u32>,
    pub tests_failed: Option<u32>,
}

/// Auto-detect test framework and run tests.
pub fn run_tests(base: &Utf8Path, framework: Option<TestFramework>) -> Result<TestRunResult> {
    let detected = framework.unwrap_or_else(|| detect_framework(base));

    let (cmd, args): (&str, Vec<&str>) = match detected {
        TestFramework::Pytest => ("pytest", vec!["-xvs"]),
        TestFramework::Vitest => ("npx", vec!["vitest", "run"]),
        TestFramework::Jest => ("npx", vec!["jest"]),
        TestFramework::Mocha => ("npx", vec!["mocha"]),
        TestFramework::CargoTest => ("cargo", vec!["test"]),
        TestFramework::Unknown => {
            return Err(EditError::Io("no test framework detected".to_string()));
        }
    };

    let output = Command::new(cmd)
        .args(&args)
        .current_dir(base.as_std_path())
        .output()
        .map_err(|e| EditError::Io(format!("test command failed: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    let (tests_run, tests_failed) = parse_test_counts(&detected, &stdout, &stderr);

    Ok(TestRunResult {
        framework: detected,
        command: format!("{} {}", cmd, args.join(" ")),
        exit_code,
        stdout: truncate(&stdout, 50000),
        stderr: truncate(&stderr, 10000),
        passed: exit_code == 0,
        tests_run,
        tests_failed,
    })
}

/// Detect test framework from project files.
pub fn detect_framework(base: &Utf8Path) -> TestFramework {
    let files: [(&str, TestFramework); 10] = [
        ("pytest.ini", TestFramework::Pytest),
        ("pyproject.toml", TestFramework::Pytest),
        ("setup.py", TestFramework::Pytest),
        ("vitest.config.ts", TestFramework::Vitest),
        ("vitest.config.js", TestFramework::Vitest),
        ("jest.config.js", TestFramework::Jest),
        ("jest.config.ts", TestFramework::Jest),
        (".mocharc.js", TestFramework::Mocha),
        (".mocharc.json", TestFramework::Mocha),
        ("Cargo.toml", TestFramework::CargoTest),
    ];

    for (file, framework) in &files {
        if base.join(file).exists() {
            return *framework;
        }
    }

    if let Ok(content) = std::fs::read_to_string(base.join("package.json")) {
        if content.contains("vitest") { return TestFramework::Vitest; }
        if content.contains("jest") { return TestFramework::Jest; }
        if content.contains("mocha") { return TestFramework::Mocha; }
    }

    TestFramework::Unknown
}

fn parse_test_counts(
    _framework: &TestFramework,
    stdout: &str,
    stderr: &str,
) -> (Option<u32>, Option<u32>) {
    let combined = format!("{} {}", stdout, stderr);

    // Try pytest pattern
    if let Some(re) = regex::Regex::new(r"(\d+) passed(?:, (\d+) failed)?").ok() {
        if let Some(caps) = re.captures(&combined) {
            if let Ok(passed) = caps[1].parse::<u32>() {
                let failed = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok()).unwrap_or(0);
                return (Some(passed + failed), Some(failed));
            }
        }
    }

    // Try cargo test pattern
    if let Some(re) = regex::Regex::new(r"test result: ok\. (\d+) passed").ok() {
        if let Some(caps) = re.captures(&combined) {
            if let Ok(passed) = caps[1].parse::<u32>() {
                return (Some(passed), Some(0));
            }
        }
    }
    if let Some(re) = regex::Regex::new(r"test result: FAILED\. (\d+) passed; (\d+) failed").ok() {
        if let Some(caps) = re.captures(&combined) {
            if let (Ok(passed), Ok(failed)) = (caps[1].parse::<u32>(), caps[2].parse::<u32>()) {
                return (Some(passed + failed), Some(failed));
            }
        }
    }

    (None, None)
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}... [truncated {} chars]", &s[..max_len], s.len() - max_len)
    }
}
