//! Focused test runner with auto-detection.

use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use camino::Utf8Path;
use serde::{Deserialize, Serialize};

use crate::{EditError, Result};

const REDACTION_OVERLAP_BYTES: usize = 512;
const FALLBACK_PATH: &str = "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin";

const SAFE_TEST_ENV_EXACT: &[&str] = &[
    "CARGO_TARGET_DIR",
    "CARGO_TERM_COLOR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "PATH",
    "RUSTUP_TOOLCHAIN",
    "TERM",
];

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
    #[serde(default)]
    pub timed_out: bool,
    pub tests_run: Option<u32>,
    pub tests_failed: Option<u32>,
}

/// Test runner controls.
#[derive(Debug, Clone)]
pub struct TestRunnerConfig {
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

impl Default for TestRunnerConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(120),
            max_stdout_bytes: 50_000,
            max_stderr_bytes: 10_000,
        }
    }
}

/// Auto-detect test framework and run tests.
pub fn run_tests(base: &Utf8Path, framework: Option<TestFramework>) -> Result<TestRunResult> {
    run_tests_with_config(base, framework, &TestRunnerConfig::default())
}

/// Auto-detect test framework and run tests with explicit controls.
pub fn run_tests_with_config(
    base: &Utf8Path,
    framework: Option<TestFramework>,
    config: &TestRunnerConfig,
) -> Result<TestRunResult> {
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

    run_test_command(base, detected, cmd, &args, config)
}

fn run_test_command(
    base: &Utf8Path,
    framework: TestFramework,
    cmd: &str,
    args: &[&str],
    config: &TestRunnerConfig,
) -> Result<TestRunResult> {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .current_dir(base.as_std_path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let _isolated_env = configure_sanitized_test_env(&mut command, framework)?;
    configure_child_process_group(&mut command);

    let mut child = command
        .spawn()
        .map_err(|e| EditError::Io(format!("test command failed: {e}")))?;

    let process_id = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| EditError::Io("test command stdout pipe unavailable".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| EditError::Io("test command stderr pipe unavailable".to_string()))?;
    let stdout_reader = read_capped(stdout, config.max_stdout_bytes);
    let stderr_reader = read_capped(stderr, config.max_stderr_bytes);

    let deadline = Instant::now() + config.timeout;
    let (exit_code, timed_out) = loop {
        match child
            .try_wait()
            .map_err(|e| EditError::Io(format!("test command wait failed: {e}")))?
        {
            Some(status) => break (status.code().unwrap_or(-1), false),
            None if Instant::now() >= deadline => {
                terminate_test_process(process_id);
                let _ = child.kill();
                let _ = child.wait();
                break (-1, true);
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };

    let reader_timeout = Duration::from_millis(500);
    let stdout = join_capped_reader(stdout_reader, "stdout", reader_timeout)?;
    let mut stderr = join_capped_reader(stderr_reader, "stderr", reader_timeout)?;
    if timed_out {
        stderr.push_str(&format!(
            "\n[test command timed out after {}ms]",
            config.timeout.as_millis()
        ));
    }

    let stdout = mimir_security::redact_secrets(&stdout);
    let stderr = mimir_security::redact_secrets(&stderr);

    let (tests_run, tests_failed) = parse_test_counts(&framework, &stdout, &stderr);

    Ok(TestRunResult {
        framework,
        command: format!("{} {}", cmd, args.join(" ")),
        exit_code,
        stdout: truncate(&stdout, config.max_stdout_bytes),
        stderr: truncate(&stderr, config.max_stderr_bytes),
        passed: !timed_out && exit_code == 0,
        timed_out,
        tests_run,
        tests_failed,
    })
}

struct IsolatedTestEnv {
    root: PathBuf,
}

impl Drop for IsolatedTestEnv {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn configure_sanitized_test_env(
    command: &mut Command,
    framework: TestFramework,
) -> Result<IsolatedTestEnv> {
    let safe_env = sanitized_test_env(
        std::env::vars_os(),
        matches!(framework, TestFramework::CargoTest),
    );
    let isolated_env = create_isolated_test_env()?;
    let home = isolated_env.root.join("home");
    let cargo_home = isolated_env.root.join("cargo-home");
    let tmp = isolated_env.root.join("tmp");

    command.env_clear();
    for (key, value) in safe_env {
        command.env(key, value);
    }
    if std::env::var_os("PATH").is_none() {
        command.env("PATH", FALLBACK_PATH);
    }
    command.env("HOME", &home);
    command.env("CARGO_HOME", &cargo_home);
    command.env("TMPDIR", &tmp);
    command.env("TMP", &tmp);
    command.env("TEMP", &tmp);
    Ok(isolated_env)
}

fn create_isolated_test_env() -> Result<IsolatedTestEnv> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let root = std::env::temp_dir().join(format!("mimir-test-env-{}-{nanos}", std::process::id()));
    fs::create_dir_all(root.join("home"))
        .map_err(|err| EditError::Io(format!("create isolated test HOME: {err}")))?;
    fs::create_dir_all(root.join("cargo-home"))
        .map_err(|err| EditError::Io(format!("create isolated test CARGO_HOME: {err}")))?;
    fs::create_dir_all(root.join("tmp"))
        .map_err(|err| EditError::Io(format!("create isolated test TMPDIR: {err}")))?;
    Ok(IsolatedTestEnv { root })
}

fn sanitized_test_env<I>(vars: I, inherit_rustup_home: bool) -> Vec<(OsString, OsString)>
where
    I: IntoIterator<Item = (OsString, OsString)>,
{
    vars.into_iter()
        .filter(|(key, _)| should_inherit_test_env(&key.to_string_lossy(), inherit_rustup_home))
        .collect()
}

fn should_inherit_test_env(key: &str, inherit_rustup_home: bool) -> bool {
    let upper = key.to_ascii_uppercase();
    if is_sensitive_test_env(&upper) {
        return false;
    }
    if inherit_rustup_home && upper == "RUSTUP_HOME" {
        return true;
    }
    SAFE_TEST_ENV_EXACT.contains(&upper.as_str()) || upper.starts_with("LC_")
}

fn is_sensitive_test_env(upper_key: &str) -> bool {
    matches!(
        upper_key,
        "ANTHROPIC_API_KEY"
            | "GLM_API_KEY"
            | "OPENAI_API_KEY"
            | "ZAI_API_KEY"
            | "OPENAI_BASE_URL"
            | "OPENAI_MODEL"
            | "MIMIR_BASE_URL"
            | "MIMIR_MODEL"
            | "MIMIR_PROVIDER"
    ) || upper_key.contains("KEY")
        || upper_key.contains("TOKEN")
        || upper_key.contains("SECRET")
        || upper_key.contains("PASSWORD")
        || upper_key.contains("CREDENTIAL")
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
        if content.contains("vitest") {
            return TestFramework::Vitest;
        }
        if content.contains("jest") {
            return TestFramework::Jest;
        }
        if content.contains("mocha") {
            return TestFramework::Mocha;
        }
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
    if let Ok(re) = regex::Regex::new(r"(\d+) passed(?:, (\d+) failed)?") {
        if let Some(caps) = re.captures(&combined) {
            if let Ok(passed) = caps[1].parse::<u32>() {
                let failed = caps
                    .get(2)
                    .and_then(|m| m.as_str().parse::<u32>().ok())
                    .unwrap_or(0);
                return (Some(passed + failed), Some(failed));
            }
        }
    }

    // Try cargo test pattern
    if let Ok(re) = regex::Regex::new(r"test result: ok\. (\d+) passed") {
        if let Some(caps) = re.captures(&combined) {
            if let Ok(passed) = caps[1].parse::<u32>() {
                return (Some(passed), Some(0));
            }
        }
    }
    if let Ok(re) = regex::Regex::new(r"test result: FAILED\. (\d+) passed; (\d+) failed") {
        if let Some(caps) = re.captures(&combined) {
            if let (Ok(passed), Ok(failed)) = (caps[1].parse::<u32>(), caps[2].parse::<u32>()) {
                return (Some(passed + failed), Some(failed));
            }
        }
    }

    (None, None)
}

#[cfg(unix)]
fn configure_child_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_child_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_test_process(process_id: u32) {
    let process_group = format!("-{process_id}");
    let _ = Command::new("kill")
        .args(["-TERM", &process_group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    thread::sleep(Duration::from_millis(100));
    let _ = Command::new("kill")
        .args(["-KILL", &process_group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
fn terminate_test_process(_process_id: u32) {}

fn read_capped<R>(mut reader: R, max_bytes: usize) -> mpsc::Receiver<Result<String>>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut stored = Vec::new();
        let mut total_bytes = 0usize;
        let mut chunk = [0u8; 8192];
        let store_limit = max_bytes.saturating_add(REDACTION_OVERLAP_BYTES);

        loop {
            let read = match reader.read(&mut chunk) {
                Ok(read) => read,
                Err(error) => {
                    let _ = sender.send(Err(EditError::Io(format!(
                        "test output read failed: {error}"
                    ))));
                    return;
                }
            };
            if read == 0 {
                break;
            }

            total_bytes += read;
            let remaining = store_limit.saturating_sub(stored.len());
            if remaining > 0 {
                stored.extend_from_slice(&chunk[..read.min(remaining)]);
            }
        }

        let mut text = String::from_utf8_lossy(&stored).to_string();
        if total_bytes > stored.len() {
            text.push_str(&format!(
                "\n... [truncated {} bytes]",
                total_bytes - stored.len()
            ));
        }
        let _ = sender.send(Ok(text));
    });
    receiver
}

fn join_capped_reader(
    reader: mpsc::Receiver<Result<String>>,
    stream_name: &str,
    timeout: Duration,
) -> Result<String> {
    reader
        .recv_timeout(timeout)
        .map_err(|_| EditError::Io(format!("test {stream_name} reader timed out")))?
}

fn truncate(s: &str, max_len: usize) -> String {
    let truncated = s.chars().take(max_len).collect::<String>();
    if truncated.len() == s.len() {
        s.to_string()
    } else {
        format!(
            "{truncated}... [truncated {} chars]",
            s.chars().count() - max_len
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn workspace_root() -> Utf8PathBuf {
        Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap()
    }

    #[test]
    fn truncate_handles_multibyte_boundaries() {
        let output = truncate("alpha 😄 omega", 8);

        assert!(output.starts_with("alpha 😄 "));
        assert!(output.contains("truncated"));
    }

    #[test]
    fn sanitized_env_strips_provider_keys_and_generic_secrets() {
        let env = sanitized_test_env(
            [
                (OsString::from("PATH"), OsString::from("/bin:/usr/bin")),
                (OsString::from("HOME"), OsString::from("/tmp/home")),
                (OsString::from("RUSTUP_HOME"), OsString::from("/tmp/rustup")),
                (OsString::from("OPENAI_API_KEY"), OsString::from("sk-test")),
                (OsString::from("GLM_API_KEY"), OsString::from("glm-test")),
                (OsString::from("ZAI_API_KEY"), OsString::from("zai-test")),
                (
                    OsString::from("ANTHROPIC_API_KEY"),
                    OsString::from("anthropic-test"),
                ),
                (
                    OsString::from("PROJECT_TOKEN"),
                    OsString::from("token-test"),
                ),
                (
                    OsString::from("DATABASE_PASSWORD"),
                    OsString::from("password-test"),
                ),
                (
                    OsString::from("SERVICE_SECRET"),
                    OsString::from("secret-test"),
                ),
                (
                    OsString::from("CI_CREDENTIAL"),
                    OsString::from("credential-test"),
                ),
                (
                    OsString::from("OPENAI_BASE_URL"),
                    OsString::from("https://example.invalid"),
                ),
            ],
            false,
        );
        let env = env
            .into_iter()
            .map(|(key, value)| (key.to_string_lossy().into_owned(), value))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(env.get("PATH"), Some(&OsString::from("/bin:/usr/bin")));
        assert!(!env.contains_key("HOME"));
        assert!(!env.contains_key("CARGO_HOME"));
        assert!(!env.contains_key("RUSTUP_HOME"));
        assert!(!env.contains_key("OPENAI_API_KEY"));
        assert!(!env.contains_key("GLM_API_KEY"));
        assert!(!env.contains_key("ZAI_API_KEY"));
        assert!(!env.contains_key("ANTHROPIC_API_KEY"));
        assert!(!env.contains_key("PROJECT_TOKEN"));
        assert!(!env.contains_key("DATABASE_PASSWORD"));
        assert!(!env.contains_key("SERVICE_SECRET"));
        assert!(!env.contains_key("CI_CREDENTIAL"));
        assert!(!env.contains_key("OPENAI_BASE_URL"));
    }

    #[test]
    fn sanitized_env_keeps_rustup_home_only_for_cargo() {
        let env = sanitized_test_env(
            [(OsString::from("RUSTUP_HOME"), OsString::from("/tmp/rustup"))],
            true,
        );

        assert_eq!(
            env,
            vec![(OsString::from("RUSTUP_HOME"), OsString::from("/tmp/rustup"))]
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_subprocess_cannot_read_provider_key_env() {
        let _guard = env_lock().lock().unwrap();
        let old_openai = std::env::var_os("OPENAI_API_KEY");
        let old_glm = std::env::var_os("GLM_API_KEY");
        std::env::set_var("OPENAI_API_KEY", "sk-test-provider-env");
        std::env::set_var("GLM_API_KEY", "glm-test-provider-env");

        let config = TestRunnerConfig {
            timeout: Duration::from_secs(5),
            max_stdout_bytes: 1_000,
            max_stderr_bytes: 1_000,
        };

        let result = run_test_command(
            &workspace_root(),
            TestFramework::Unknown,
            "sh",
            &[
                "-c",
                "if [ -n \"${OPENAI_API_KEY:-}\" ] || [ -n \"${GLM_API_KEY:-}\" ]; then echo leaked; exit 42; fi",
            ],
            &config,
        )
        .unwrap();

        restore_env("OPENAI_API_KEY", old_openai);
        restore_env("GLM_API_KEY", old_glm);

        assert!(result.passed);
        assert!(!result.stdout.contains("leaked"));
    }

    #[cfg(unix)]
    #[test]
    fn test_subprocess_gets_isolated_home_and_cargo_home() {
        let _guard = env_lock().lock().unwrap();
        let old_home = std::env::var_os("HOME");
        let old_cargo_home = std::env::var_os("CARGO_HOME");
        let host_home = tempfile::TempDir::new().unwrap();
        let host_cargo_home = tempfile::TempDir::new().unwrap();
        std::fs::write(host_home.path().join(".provider-secret"), "secret").unwrap();
        std::fs::write(host_cargo_home.path().join("credentials.toml"), "secret").unwrap();
        std::env::set_var("HOME", host_home.path());
        std::env::set_var("CARGO_HOME", host_cargo_home.path());

        let config = TestRunnerConfig {
            timeout: Duration::from_secs(5),
            max_stdout_bytes: 1_000,
            max_stderr_bytes: 1_000,
        };

        let result = run_test_command(
            &workspace_root(),
            TestFramework::Unknown,
            "sh",
            &[
                "-c",
                "test -d \"$HOME\" && test -d \"$CARGO_HOME\" && test ! -f \"$HOME/.provider-secret\" && test ! -f \"$CARGO_HOME/credentials.toml\"",
            ],
            &config,
        )
        .unwrap();

        restore_env("HOME", old_home);
        restore_env("CARGO_HOME", old_cargo_home);

        assert!(result.passed, "stderr: {}", result.stderr);
    }

    fn restore_env(key: &str, value: Option<OsString>) {
        if let Some(value) = value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }

    #[cfg(unix)]
    #[test]
    fn timeout_returns_failed_result() {
        let config = TestRunnerConfig {
            timeout: Duration::from_millis(50),
            max_stdout_bytes: 1_000,
            max_stderr_bytes: 1_000,
        };

        let result = run_test_command(
            &workspace_root(),
            TestFramework::Unknown,
            "sh",
            &["-c", "sleep 5"],
            &config,
        )
        .unwrap();

        assert!(result.timed_out);
        assert!(!result.passed);
        assert_eq!(result.exit_code, -1);
        assert!(result.stderr.contains("timed out"));
    }

    #[cfg(unix)]
    #[test]
    fn output_is_capped_and_redacted() {
        let config = TestRunnerConfig {
            timeout: Duration::from_secs(5),
            max_stdout_bytes: 32,
            max_stderr_bytes: 1_000,
        };

        let result = run_test_command(
            &workspace_root(),
            TestFramework::Unknown,
            "sh",
            &[
                "-c",
                "printf 'api_key=secret1234567890 extra data that should be capped'",
            ],
            &config,
        )
        .unwrap();

        assert!(result.passed);
        assert!(!result.stdout.contains("secret1234567890"));
        assert!(result.stdout.contains("<REDACTED:API_KEY>"));
        assert!(result.stdout.contains("truncated"));
    }

    #[cfg(unix)]
    #[test]
    fn secret_split_at_cap_boundary_is_redacted() {
        let config = TestRunnerConfig {
            timeout: Duration::from_secs(5),
            max_stdout_bytes: 48,
            max_stderr_bytes: 1_000,
        };
        let result = run_test_command(
            &workspace_root(),
            TestFramework::Unknown,
            "sh",
            &[
                "-c",
                "printf 'prefix-prefix-prefix-OPENAI_API_KEY=sk-12345678901234567890-tail'",
            ],
            &config,
        )
        .unwrap();

        assert!(result.passed);
        assert!(!result.stdout.contains("sk-12345678901234567890"));
        assert!(!result.stdout.contains("OPENAI_API_KEY=sk-"));
        assert!(result.stdout.contains("<REDACTED:"));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendant_holding_output_pipes() {
        let config = TestRunnerConfig {
            timeout: Duration::from_millis(75),
            max_stdout_bytes: 1_000,
            max_stderr_bytes: 1_000,
        };

        let result = run_test_command(
            &workspace_root(),
            TestFramework::Unknown,
            "sh",
            &["-c", "(sleep 5) & printf start; sleep 5"],
            &config,
        )
        .unwrap();

        assert!(result.timed_out);
        assert!(!result.passed);
        assert!(result.stderr.contains("timed out"));
    }
}
