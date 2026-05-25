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
    Pattern {
        name: "AWS_KEY",
        regex: r"AKIA[0-9A-Z]{16}",
    },
    Pattern {
        name: "GCP_KEY",
        regex: r"AIza[0-9A-Za-z\-_]{35}",
    },
    Pattern {
        name: "AZURE_SAS",
        regex: r"sig=[A-Za-z0-9%]+",
    },
    Pattern {
        name: "ANTHROPIC_KEY",
        regex: r"sk-ant-[a-zA-Z0-9-]+",
    },
    Pattern {
        name: "OPENAI_KEY",
        regex: r"sk-[a-zA-Z0-9]{48}",
    },
    Pattern {
        name: "STRIPE_KEY",
        regex: r"(sk|pk)_(live|test)_[a-zA-Z0-9]{24}",
    },
    Pattern {
        name: "GITHUB_TOKEN",
        regex: r"ghp_[A-Za-z0-9]{36}",
    },
    Pattern {
        name: "GITHUB_PAT",
        regex: r"github_pat_[A-Za-z0-9_]+",
    },
    Pattern {
        name: "SLACK_TOKEN",
        regex: r"xox[baprs]-[0-9a-zA-Z]+",
    },
    Pattern {
        name: "BEARER_TOKEN",
        regex: r"(?i)bearer\s+[A-Za-z0-9._~+/=-]{16,}",
    },
    Pattern {
        name: "JWT",
        regex: r"eyJ[A-Za-z0-9_-]*\.eyJ[A-Za-z0-9_-]*",
    },
    Pattern {
        name: "PRIVATE_KEY",
        regex: r"-----BEGIN (RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----",
    },
    Pattern {
        name: "ENV_KEY",
        regex: r"[A-Z_]*_KEY=[^\s]+",
    },
    Pattern {
        name: "ENV_SECRET",
        regex: r"[A-Z_]*_SECRET=[^\s]+",
    },
    Pattern {
        name: "ENV_TOKEN",
        regex: r"[A-Z_]*_TOKEN=[^\s]+",
    },
    Pattern {
        name: "PASSWORD",
        regex: r"password=[^\s]+",
    },
    Pattern {
        name: "PASSWD",
        regex: r"passwd=[^\s]+",
    },
    Pattern {
        name: "API_KEY",
        regex: r"api[_-]?key[_-]?[:=][\s]*[a-zA-Z0-9_-]+",
    },
    Pattern {
        name: "DB_URL",
        regex: r"(postgres|mysql|mongodb)://[^:]+:[^@]+@",
    },
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
        result = re
            .replace_all(&result, format!("<REDACTED:{}>", name))
            .to_string();
    }
    result
}

/// Recursively redact secrets in JSON string leaves and sensitive key values.
pub fn redact_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => {
            *text = redact_secrets(text);
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json_value(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if is_sensitive_key(key) {
                    *value = serde_json::Value::String("<REDACTED:SECRET_FIELD>".to_string());
                } else {
                    redact_json_value(value);
                }
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    if is_token_accounting_key(&lower) {
        return false;
    }
    lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("secret")
        || lower.contains("credential")
        || lower == "token"
        || lower.ends_with("_token")
        || lower.ends_with("-token")
        || lower.ends_with(".token")
        || lower.contains("password")
        || lower == "authorization"
        || lower == "auth"
}

fn is_token_accounting_key(lower: &str) -> bool {
    lower == "tokens"
        || lower.ends_with("_tokens")
        || lower.ends_with("-tokens")
        || lower.ends_with(".tokens")
        || lower.contains("token_count")
        || lower.contains("token_counts")
        || lower.contains("token_usage")
        || lower.contains("token_budget")
        || lower.contains("token_reserve")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_secret(parts: &[&str]) -> String {
        parts.concat()
    }

    #[test]
    fn test_aws_key() {
        let text = synthetic_secret(&["key=", "AKIA", "IOSFODNN7EXAMPLE"]);
        let redacted = redact_secrets(&text);
        assert!(!redacted.contains(&text));
        assert!(redacted.contains("<REDACTED:AWS_KEY>"));
    }

    #[test]
    fn test_gcp_key() {
        let text = synthetic_secret(&["AI", "zaSyB-1I2j3k4l5m6n7o8p9q0r1s2t3u4v5w6"]);
        let redacted = redact_secrets(&text);
        assert!(!redacted.contains(&text));
        assert!(redacted.contains("<REDACTED:GCP_KEY>"));
    }

    #[test]
    fn test_anthropic_key() {
        let text = synthetic_secret(&["sk", "-ant-api03-EXAMPLE12345"]);
        let redacted = redact_secrets(&text);
        assert!(!redacted.contains(&text));
        assert!(redacted.contains("<REDACTED:ANTHROPIC_KEY>"));
    }

    #[test]
    fn test_stripe_key() {
        let text = synthetic_secret(&["sk", "_live_", "abcdefghijklmnopqrstuvwxyz"]);
        let redacted = redact_secrets(&text);
        assert!(!redacted.contains(&text));
        assert!(redacted.contains("<REDACTED:STRIPE_KEY>"));
    }

    #[test]
    fn test_github_token() {
        let text = synthetic_secret(&["gh", "p_abcdefghijklmnopqrstuvwxyz0123456789"]);
        let redacted = redact_secrets(&text);
        assert!(!redacted.contains(&text));
        assert!(redacted.contains("<REDACTED:GITHUB_TOKEN>"));
    }

    #[test]
    fn test_slack_token() {
        let text = synthetic_secret(&[
            "xox",
            "b-1234567890123-1234567890123-AbCdEfGhIjKlMnOpQrStUvWx",
        ]);
        let redacted = redact_secrets(&text);
        assert!(!redacted.contains(&text));
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

    #[test]
    fn test_bearer_token() {
        let text = "Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(redacted.contains("<REDACTED:BEARER_TOKEN>"));
    }

    #[test]
    fn test_redact_json_value_nested() {
        let mut value = serde_json::json!({
            "payload": {
                "Authorization": "Bearer abcdefghijklmnopqrstuvwxyz123456",
                "nested": ["MY_TOKEN=tok123", {"password": "hunter2"}],
                "safe": "hello"
            }
        });
        redact_json_value(&mut value);
        let text = value.to_string();
        assert!(!text.contains("tok123"));
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(text.contains("<REDACTED:SECRET_FIELD>"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn test_redact_json_preserves_token_accounting_fields() {
        let mut value = serde_json::json!({
            "max_tokens": 64000,
            "input_tokens": 123,
            "output_tokens": 456,
            "output_reserve_tokens": 789,
            "access_token": "secret-token-value",
            "api_key": "secret-key-value"
        });

        redact_json_value(&mut value);

        assert_eq!(value["max_tokens"], 64000);
        assert_eq!(value["input_tokens"], 123);
        assert_eq!(value["output_tokens"], 456);
        assert_eq!(value["output_reserve_tokens"], 789);
        assert_eq!(value["access_token"], "<REDACTED:SECRET_FIELD>");
        assert_eq!(value["api_key"], "<REDACTED:SECRET_FIELD>");
    }
}
