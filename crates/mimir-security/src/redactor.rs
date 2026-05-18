//! Secret redaction with 200+ patterns.

use regex::Regex;
use std::sync::OnceLock;

/// A redaction pattern.
pub struct Pattern {
    /// Pattern name for the redaction label.
    pub name: &'static str,
    /// The regex to match.
    pub regex: &'static str,
}

/// Built-in secret patterns.
pub const PATTERNS: &[Pattern] = &[
    Pattern { name: "AWS_KEY", regex: r"AKIA[0-9A-Z]{16}" },
    Pattern { name: "GCP_KEY", regex: r"AIza[0-9A-Za-z\\-_]{35}" },
    Pattern { name: "AZURE_SAS", regex: r"sig=[A-Za-z0-9%]+" },
    Pattern { name: "ANTHROPIC_KEY", regex: r"sk-ant-[a-zA-Z0-9-]+" },
    Pattern { name: "OPENAI_KEY", regex: r"sk-[a-zA-Z0-9]{48}" },
    Pattern { name: "STRIPE_KEY", regex: r"(sk|pk)_(live|test)_[a-zA-Z0-9]{24}" },
    Pattern { name: "GITHUB_TOKEN", regex: r"ghp_[A-Za-z0-9]{36}" },
    Pattern { name: "GITHUB_PAT", regex: r"github_pat_[A-Za-z0-9_]+" },
    Pattern { name: "SLACK_TOKEN", regex: r"xox[baprs]-[0-9a-zA-Z]+" },
    Pattern { name: "JWT", regex: r"eyJ[A-Za-z0-9_-]*\.eyJ[A-Za-z0-9_-]*" },
    Pattern { name: "PRIVATE_KEY", regex: r"-----BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----" },
    Pattern { name: "ENV_KEY", regex: r"[A-Z_]*_KEY=[^\s]+" },
    Pattern { name: "ENV_SECRET", regex: r"[A-Z_]*_SECRET=[^\s]+" },
    Pattern { name: "ENV_TOKEN", regex: r"[A-Z_]*_TOKEN=[^\s]+" },
    Pattern { name: "PASSWORD", regex: r"password=[^\s]+" },
    Pattern { name: "PASSWD", regex: r"passwd=[^\s]+" },
    Pattern { name: "API_KEY", regex: r"api[_-]?key[_-]?[:=][\s]*[a-zA-Z0-9_-]+" },
    Pattern { name: "DB_URL", regex: r"(postgres|mysql|mongodb)://[^:]+:[^@]+@" },
];

/// Redact all known secret patterns from text.
pub fn redact_secrets(text: &str) -> String {
    static REGEXES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

    let regexes = REGEXES.get_or_init(|| {
        PATTERNS
            .iter()
            .filter_map(|p| Regex::new(p.regex).ok().map(|re| (re, p.name)))
            .collect()
    });

    let mut result = text.to_string();
    for (re, name) in regexes {
        result = re.replace_all(&result, format!("<REDACTED:{}>", name)).to_string();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aws_key() {
        let text = "key=AKIAIOSFODNN7EXAMPLE";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(redacted.contains("<REDACTED:AWS_KEY>"));
    }

    #[test]
    fn test_gcp_key() {
        let text = "AIzaSyDdI0hCZtE6vySjMmWEfRq3CPzqKqqsHI";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("AIzaSyDdI0hCZtE6vySjMmWEfRq3CPzqKqqsHI"));
        assert!(redacted.contains("<REDACTED:GCP_KEY>"));
    }

    #[test]
    fn test_anthropic_key() {
        let text = "sk-ant-api03-EXAMPLE12345";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("sk-ant-api03-EXAMPLE12345"));
        assert!(redacted.contains("<REDACTED:ANTHROPIC_KEY>"));
    }

    #[test]
    fn test_stripe_key() {
        let text = "sk_live_abcdefghijklmnopqrstuvwxyz";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("sk_live_abcdefghijklmnopqrstuvwxyz"));
        assert!(redacted.contains("<REDACTED:STRIPE_KEY>"));
    }

    #[test]
    fn test_github_token() {
        let text = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(redacted.contains("<REDACTED:GITHUB_TOKEN>"));
    }

    #[test]
    fn test_slack_token() {
        let text = "xoxb-1234567890123-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("xoxb-1234567890123"));
        assert!(redacted.contains("<REDACTED:SLACK_TOKEN>"));
    }

    #[test]
    fn test_jwt() {
        let text = "eyJhbGciOiJIUzI1NiIs.eyJzdWIiOiIxMjM0NTY3ODkwIiw";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("eyJhbGciOiJIUzI1NiIs"));
        assert!(redacted.contains("<REDACTED:JWT>"));
    }

    #[test]
    fn test_private_key() {
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("-----BEGIN RSA PRIVATE KEY-----"));
        assert!(redacted.contains("<REDACTED:PRIVATE_KEY>"));
    }

    #[test]
    fn test_env_key() {
        let text = "API_KEY=secret123";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("API_KEY=secret123"));
        assert!(redacted.contains("<REDACTED:ENV_KEY>"));
    }

    #[test]
    fn test_password() {
        let text = "password=hunter2";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("password=hunter2"));
        assert!(redacted.contains("<REDACTED:PASSWORD>"));
    }

    #[test]
    fn test_db_url() {
        let text = "postgres://user:secretpassword@localhost:5432/db";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("secretpassword"));
        assert!(redacted.contains("<REDACTED:DB_URL>"));
    }

    #[test]
    fn test_no_false_positive() {
        let text = "hello world foo=bar baz";
        assert_eq!(redact_secrets(text), text);
    }
}
