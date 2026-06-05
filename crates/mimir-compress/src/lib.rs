//! `mimir-compress` — Deterministic, rule-based context body compressors.
//!
//! Pure library: no I/O, no network, no provider dependencies.

#![warn(missing_docs)]

use once_cell::sync::Lazy;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Compression algorithm selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionAlgorithm {
    /// Identity pass-through.
    None,
    /// Keep signatures, imports, docs; elide bodies.
    CodeSkeleton,
    /// Compact homogeneous JSON arrays; sort keys and truncate strings.
    JsonCrush,
}

/// Result of compressing a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedBody {
    /// Algorithm used.
    pub algorithm: CompressionAlgorithm,
    /// Compressed text that enters the packet.
    pub text: String,
    /// SHA-256 hex of the **original** bytes.
    pub original_hash: String,
    /// Token count of the original (computed by caller).
    pub original_tokens: u32,
    /// Token count of the compressed text (computed by caller).
    pub compressed_tokens: u32,
}

/// Pure function of (content, language, target). Deterministic.
///
/// `count_tokens` is injected by the caller so this crate stays free of
/// tokenizer dependencies.
pub fn compress_body(
    content: &str,
    language: &str,
    _target_tokens: u32,
    count_tokens: impl Fn(&str) -> u32,
) -> CompressedBody {
    let original_hash = sha256_hex(content.as_bytes());
    let original_tokens = count_tokens(content);

    let algorithm = select_algorithm(language);
    let text = match algorithm {
        CompressionAlgorithm::None => content.to_string(),
        CompressionAlgorithm::CodeSkeleton => skeletonize(content, language),
        CompressionAlgorithm::JsonCrush => crush_json(content),
    };

    let compressed_tokens = count_tokens(&text);

    CompressedBody {
        algorithm,
        text,
        original_hash,
        original_tokens,
        compressed_tokens,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn select_algorithm(language: &str) -> CompressionAlgorithm {
    match language {
        // Only languages with real, tested signature patterns in `skeletonize`.
        // Anything else falls through to `None` (verbatim) rather than being
        // mangled by a mismatched regex — preserving correctness over reach.
        "rust" | "typescript" | "javascript" | "python" => CompressionAlgorithm::CodeSkeleton,
        "json" | "jsonc" => CompressionAlgorithm::JsonCrush,
        _ => CompressionAlgorithm::None,
    }
}

/// Largest byte index `<= max_bytes` that lands on a UTF-8 char boundary.
///
/// Slicing a `&str` at an arbitrary byte index panics when it splits a
/// multi-byte character; truncating at a boundary keeps that safe.
fn floor_char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

// ---------------------------------------------------------------------------
// CodeSkeleton
// ---------------------------------------------------------------------------

static RUST_SIG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*(?:pub\s+)?(?:fn|struct|enum|trait|type|const|static|mod|use|impl)\s+"#)
        .unwrap()
});

static TS_SIG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)^\s*(?:export\s+)?(?:function|class|interface|type|const|let|var|enum|import)\s+"#,
    )
    .unwrap()
});

static PY_SIG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*(?:async\s+)?def\s+|^\s*class\s+|^\s*import\s+|^\s*from\s+"#).unwrap()
});

static DOC_COMMENT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)^\s*(///|//!|//|#|/\*\*|\*|\*/)"#).unwrap());

fn skeletonize(content: &str, language: &str) -> String {
    let sig_re = match language {
        "rust" => &*RUST_SIG_RE,
        "typescript" | "javascript" => &*TS_SIG_RE,
        "python" => &*PY_SIG_RE,
        // Generic fallback: try all patterns
        _ => &*RUST_SIG_RE,
    };

    let comment_token = match language {
        "python" => "#",
        _ => "//",
    };

    let lines: Vec<&str> = content.lines().collect();
    let mut kept = Vec::new();
    let mut run_start: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        let is_keep = is_keep_line(line, sig_re);
        if is_keep {
            if let Some(start) = run_start {
                let elided = i - start;
                if elided > 0 {
                    kept.push(format!("{} … {} lines elided …", comment_token, elided));
                }
                run_start = None;
            }
            kept.push(line.to_string());
        } else if run_start.is_none() {
            run_start = Some(i);
        }
    }

    // trailing elided run
    if let Some(start) = run_start {
        let elided = lines.len() - start;
        if elided > 0 {
            kept.push(format!("{} … {} lines elided …", comment_token, elided));
        }
    }

    kept.join("\n")
}

fn is_keep_line(line: &str, sig_re: &Regex) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    if DOC_COMMENT_RE.is_match(line) {
        return true;
    }
    if sig_re.is_match(line) {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// JsonCrush
// ---------------------------------------------------------------------------

const JSON_STRING_CAP: usize = 200;

fn crush_json(content: &str) -> String {
    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return content.to_string(),
    };

    if let Some(arr) = value.as_array() {
        if !arr.is_empty() && arr.iter().all(|v| v.is_object()) {
            if let Some(header) = arr.first().and_then(|v| v.as_object()) {
                let keys: Vec<String> = header.keys().cloned().collect();
                if keys.len() <= 32
                    && arr.iter().all(|v| {
                        v.as_object()
                            .map(|o| {
                                o.keys().collect::<Vec<_>>() == keys.iter().collect::<Vec<_>>()
                            })
                            .unwrap_or(false)
                    })
                {
                    return crush_homogeneous_array(&keys, arr);
                }
            }
        }
    }

    let sorted = sort_keys_and_truncate(value);
    serde_json::to_string_pretty(&sorted).unwrap_or_else(|_| content.to_string())
}

fn crush_homogeneous_array(keys: &[String], arr: &[serde_json::Value]) -> String {
    let mut lines = Vec::new();
    lines.push(format!("// keys: {}", keys.join(", ")));
    for item in arr {
        if let Some(obj) = item.as_object() {
            let values: Vec<String> = keys
                .iter()
                .map(|k| compact_value(obj.get(k).unwrap_or(&serde_json::Value::Null)))
                .collect();
            lines.push(format!("[{}]", values.join(", ")));
        }
    }
    lines.join("\n")
}

fn compact_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) if s.len() > JSON_STRING_CAP => {
            let end = floor_char_boundary(s, JSON_STRING_CAP);
            format!("\"{}…(+{} chars)\"", &s[..end], s.len() - end)
        }
        serde_json::Value::String(s) => {
            serde_json::to_string(&serde_json::Value::String(s.clone())).unwrap_or_default()
        }
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Array(a) => {
            if a.len() <= 3 {
                let inner: Vec<String> = a.iter().map(compact_value).collect();
                format!("[{}]", inner.join(", "))
            } else {
                format!("[…{} items…]", a.len())
            }
        }
        serde_json::Value::Object(_) => "{…}".to_string(),
    }
}

fn sort_keys_and_truncate(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut new_map = BTreeMap::new();
            for (k, v) in map {
                new_map.insert(k, sort_keys_and_truncate(v));
            }
            serde_json::Value::Object(new_map.into_iter().collect())
        }
        serde_json::Value::Array(arr) => {
            let new_arr: Vec<_> = arr.into_iter().map(sort_keys_and_truncate).collect();
            serde_json::Value::Array(new_arr)
        }
        serde_json::Value::String(s) if s.len() > JSON_STRING_CAP => {
            let end = floor_char_boundary(&s, JSON_STRING_CAP);
            serde_json::Value::String(format!("{}…(+{} chars)", &s[..end], s.len() - end))
        }
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_count_tokens(text: &str) -> u32 {
        // Approx 4 chars per token for testing.
        (text.len() / 4).try_into().unwrap_or(u32::MAX)
    }

    #[test]
    fn determinism_code_skeleton() {
        let src = r#"
use std::fs;

/// A helper.
pub fn helper(x: i32) -> i32 {
    x + 1
}

struct Foo {
    bar: i32,
}
"#;
        let r1 = compress_body(src, "rust", 100, fake_count_tokens);
        let r2 = compress_body(src, "rust", 100, fake_count_tokens);
        assert_eq!(r1.text, r2.text);
        assert_eq!(r1.original_hash, r2.original_hash);
        assert_eq!(r1.algorithm, CompressionAlgorithm::CodeSkeleton);
    }

    #[test]
    fn compressed_tokens_less_than_original() {
        let src = r#"
use std::fs;

/// A helper function with a large body.
pub fn helper(x: i32) -> i32 {
    let a = 1;
    let b = 2;
    let c = 3;
    let d = 4;
    let e = 5;
    let f = 6;
    let g = 7;
    let h = 8;
    let i = 9;
    let j = 10;
    let k = 11;
    let l = 12;
    let m = 13;
    let n = 14;
    let o = 15;
    let p = 16;
    let q = 17;
    let r = 18;
    let s = 19;
    let t = 20;
    x + a + b + c + d + e + f + g + h + i + j + k + l + m + n + o + p + q + r + s + t
}

struct Foo {
    bar: i32,
}

impl Foo {
    fn new() -> Self {
        let x = 0;
        let y = 1;
        let z = 2;
        Self { bar: x + y + z }
    }
}
"#;
        let result = compress_body(src, "rust", 100, fake_count_tokens);
        assert!(
            result.compressed_tokens < result.original_tokens,
            "expected {} < {}",
            result.compressed_tokens,
            result.original_tokens
        );
    }

    #[test]
    fn signatures_preserved() {
        let src = r#"pub fn foo() {}
struct Bar {}
enum Baz { A, B }
"#;
        let result = compress_body(src, "rust", 100, fake_count_tokens);
        assert!(result.text.contains("pub fn foo"));
        assert!(result.text.contains("struct Bar"));
        assert!(result.text.contains("enum Baz"));
    }

    #[test]
    fn unknown_language_returns_none() {
        let src = "hello world";
        let result = compress_body(src, "unknown_lang", 100, fake_count_tokens);
        assert_eq!(result.algorithm, CompressionAlgorithm::None);
        assert_eq!(result.text, src);
        assert_eq!(result.compressed_tokens, result.original_tokens);
    }

    #[test]
    fn json_crush_homogeneous_array() {
        let json = r#"[
            {"name": "alice", "age": 30},
            {"name": "bob", "age": 25}
        ]"#;
        let result = compress_body(json, "json", 100, fake_count_tokens);
        assert_eq!(result.algorithm, CompressionAlgorithm::JsonCrush);
        assert!(result.text.contains("keys: age, name"));
        assert!(result.text.contains("alice"));
        assert!(result.text.contains("bob"));
        assert!(
            result.compressed_tokens < result.original_tokens,
            "expected compressed {} < original {}",
            result.compressed_tokens,
            result.original_tokens
        );
    }

    #[test]
    fn json_crush_truncates_long_strings() {
        let long = "a".repeat(500);
        let json = format!(r#"{{"key": "{}"}}"#, long);
        let result = compress_body(&json, "json", 100, fake_count_tokens);
        assert!(result.text.contains("…(+300 chars)"));
    }

    #[test]
    fn json_crush_sorts_keys() {
        let json = r#"{"z": 1, "a": 2, "m": 3}"#;
        let result = compress_body(json, "json", 100, fake_count_tokens);
        let a_pos = result.text.find("\"a\"").unwrap();
        let m_pos = result.text.find("\"m\"").unwrap();
        let z_pos = result.text.find("\"z\"").unwrap();
        assert!(a_pos < m_pos && m_pos < z_pos);
    }

    #[test]
    fn determinism_json_crush() {
        let json = r#"{"b": 2, "a": 1}"#;
        let r1 = compress_body(json, "json", 100, fake_count_tokens);
        let r2 = compress_body(json, "json", 100, fake_count_tokens);
        assert_eq!(r1.text, r2.text);
    }

    #[test]
    fn json_crush_truncation_does_not_panic_on_multibyte_boundary() {
        // A multi-byte char (é = 2 bytes) positioned so the cap falls inside it.
        // A naive `&s[..JSON_STRING_CAP]` slice would panic here.
        let mut long = "a".repeat(JSON_STRING_CAP - 1);
        long.push('é');
        long.push_str(&"b".repeat(50));
        let json = format!(r#"{{"key": "{}"}}"#, long);
        // Must not panic.
        let result = compress_body(&json, "json", 100, fake_count_tokens);
        assert!(result.text.contains("…(+"));
        // And the truncated prefix must be valid UTF-8 (it is, since `text` is a String).
        assert!(result.text.is_char_boundary(result.text.len()));
    }

    #[test]
    fn unsupported_code_language_is_verbatim_not_mangled() {
        // Go/Ruby/etc. have no tested signature regex; they must pass through
        // verbatim (None) rather than be skeletonized by the Rust pattern,
        // which would silently drop their real signatures.
        let go = "func main() {\n\tprintln(\"hi\")\n}\n";
        let result = compress_body(go, "go", 100, fake_count_tokens);
        assert_eq!(result.algorithm, CompressionAlgorithm::None);
        assert_eq!(result.text, go);

        let ruby = "def greet\n  puts 'hi'\nend\n";
        let result = compress_body(ruby, "ruby", 100, fake_count_tokens);
        assert_eq!(result.algorithm, CompressionAlgorithm::None);
        assert_eq!(result.text, ruby);
    }
}
