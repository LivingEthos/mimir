//! Gateway-boundary build/test gate.
//!
//! Invariant under test: only `mimir-providers` may speak HTTP. Every other
//! workspace crate must NOT import an HTTP client (`reqwest` / `hyper` /
//! `ureq`) and must NOT invoke the provider adapter's `.call(` dispatch
//! surface directly. Provider dispatch must go through `ProviderGateway`.
//!
//! This mirrors `scripts/check-gateway-boundary.sh` as a deterministic,
//! dependency-free Rust source scan so the boundary is enforced by
//! `cargo test` (and thus by CI's `--all-targets` gate), not just an
//! out-of-band shell script.
//!
//! Self-check (proof the gate fires on a planted violation): if any scanned
//! crate's `src` gained a line such as
//!     `use reqwest::Client;`
//! or
//!     `let _ = adapter.call(request);`
//! the scanner below records the file path + line and the test panics with
//! that path. We verify this on a synthetic in-memory sample in
//! `planted_violation_is_detected` so the detection logic itself is exercised
//! without mutating any real crate source.

use std::fs;
use std::path::{Path, PathBuf};

/// Crate whose job IS to speak HTTP. Exempt from the scan.
const GATEWAY_CRATE: &str = "mimir-providers";

/// HTTP-client crate roots that no non-gateway crate may import.
const HTTP_CLIENT_CRATES: [&str; 3] = ["reqwest", "hyper", "ureq"];

/// Resolve the workspace `crates/` directory from this crate's manifest dir.
///
/// `CARGO_MANIFEST_DIR` for this test is `<workspace>/crates/mimir-providers`,
/// so `../..` is the workspace root and `../../crates` holds every crate.
fn crates_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent() // <workspace>/crates
        .and_then(Path::parent) // <workspace>
        .map(|root| root.join("crates"))
        .expect("manifest dir should have a workspace grandparent")
}

/// Collect every `*.rs` file under `dir`, recursively.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Strip a line of any trailing `//` line comment, returning the code portion.
///
/// This is intentionally conservative: it does not parse string literals, but
/// the tokens we look for (`use reqwest`, `.call(`) do not legitimately appear
/// inside string literals in real source, and stripping comments removes the
/// common false-positive (a doc/comment mentioning the banned token).
///
/// Both `//` line comments and inline single-line `/* ... */` block comments
/// are removed so a banned token mentioned inside a comment never trips the
/// gate.
fn code_part(line: &str) -> String {
    // Drop everything from the first `//` line-comment marker.
    let line = match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    };
    // Remove inline single-line `/* ... */` block comments.
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find("/*") {
        out.push_str(&rest[..open]);
        match rest[open + 2..].find("*/") {
            Some(close) => rest = &rest[open + 2 + close + 2..],
            // Unterminated on this line: drop the remainder.
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Does this code line import a banned HTTP-client crate?
///
/// Matches `use <crate>...` and `extern crate <crate>...` at statement start,
/// requiring a crate-path boundary (`::`, `;`, `{`, whitespace, or EOL) after
/// the name so `reqwest_helpers` etc. are not falsely flagged.
fn imports_http_client(code: &str) -> Option<&'static str> {
    let trimmed = code.trim_start();
    let rest = trimmed
        .strip_prefix("use ")
        .or_else(|| trimmed.strip_prefix("extern crate "))?
        .trim_start();
    for client in HTTP_CLIENT_CRATES {
        if let Some(after) = rest.strip_prefix(client) {
            // Boundary check: next char must end the crate name.
            let boundary = after
                .chars()
                .next()
                .is_none_or(|c| matches!(c, ':' | ';' | '{' | ' ' | '\t'));
            if boundary {
                return Some(client);
            }
        }
    }
    None
}

/// Does this code line invoke a `.call(` dispatch surface directly?
///
/// Mirrors the shell gate's `\.call[[:space:]]*\(` pattern.
fn invokes_adapter_call(code: &str) -> bool {
    let bytes = code.as_bytes();
    let mut i = 0;
    while let Some(rel) = code[i..].find(".call") {
        let pos = i + rel;
        let after = &code[pos + ".call".len()..];
        if after.trim_start().starts_with('(') {
            return true;
        }
        i = pos + 1;
        if i >= bytes.len() {
            break;
        }
    }
    false
}

/// A recorded boundary violation: which file, which line, and why.
#[derive(Debug)]
struct Violation {
    path: PathBuf,
    line_no: usize,
    reason: String,
}

/// Scan a single source line, returning a reason string if it violates the
/// boundary. Shared by the real-crate walk and the planted-violation test so
/// both exercise identical detection logic.
fn scan_line(line: &str) -> Option<String> {
    let code = code_part(line);
    if let Some(client) = imports_http_client(&code) {
        return Some(format!("imports HTTP client `{client}`"));
    }
    if invokes_adapter_call(&code) {
        return Some("invokes provider adapter `.call(` directly".to_string());
    }
    None
}

#[test]
fn non_provider_crates_do_not_speak_http() {
    let crates_dir = crates_dir();
    assert!(
        crates_dir.is_dir(),
        "expected workspace crates dir at {}",
        crates_dir.display()
    );

    let mut scanned_files = 0usize;
    let mut violations: Vec<Violation> = Vec::new();

    for entry in fs::read_dir(&crates_dir)
        .expect("read crates dir")
        .flatten()
    {
        let crate_path = entry.path();
        if !crate_path.is_dir() {
            continue;
        }
        let crate_name = crate_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if crate_name == GATEWAY_CRATE {
            continue; // mimir-providers is the one crate allowed to do this.
        }

        let src = crate_path.join("src");
        if !src.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_rs_files(&src, &mut files);
        for file in files {
            scanned_files += 1;
            let contents = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
            for (idx, line) in contents.lines().enumerate() {
                if let Some(reason) = scan_line(line) {
                    violations.push(Violation {
                        path: file.clone(),
                        line_no: idx + 1,
                        reason,
                    });
                }
            }
        }
    }

    // Guard against the scan silently matching nothing (e.g. path drift).
    assert!(
        scanned_files > 0,
        "scanned zero source files under {} — path resolution is broken",
        crates_dir.display()
    );

    assert!(
        violations.is_empty(),
        "gateway boundary violated; only `{GATEWAY_CRATE}` may speak HTTP \
         or call the adapter dispatch surface:\n{}",
        violations
            .iter()
            .map(|v| format!("  {}:{} — {}", v.path.display(), v.line_no, v.reason))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Proves the scanner actually fires on a planted violation. If detection ever
/// regresses to a no-op, this test fails and exposes it — so a green
/// `non_provider_crates_do_not_speak_http` genuinely means "no violations",
/// not "scanner is broken".
#[test]
fn planted_violation_is_detected() {
    // Each of these lines, were it to appear in a non-gateway crate's src,
    // must be flagged.
    assert_eq!(
        scan_line("use reqwest::Client;").as_deref(),
        Some("imports HTTP client `reqwest`"),
    );
    assert_eq!(
        scan_line("    use hyper::Body;").as_deref(),
        Some("imports HTTP client `hyper`"),
    );
    assert_eq!(
        scan_line("extern crate ureq;").as_deref(),
        Some("imports HTTP client `ureq`"),
    );
    assert!(scan_line("    let _ = adapter.call(request).await;").is_some());
    // Whitespace between `.call` and `(` is tolerated, matching the shell
    // gate's `\.call[[:space:]]*\(` pattern.
    assert!(scan_line("let r = adapter.call (req);").is_some());

    // And legitimate lines must NOT be flagged (no false positives).
    assert!(scan_line("// use reqwest::Client; (mentioned in a comment)").is_none());
    assert!(scan_line("use reqwest_stub::FakeClient;").is_none());
    assert!(scan_line("fn recall() { /* not .call( */ }").is_none());
    assert!(scan_line("let recalled = thing.recall();").is_none());
    assert!(scan_line("gateway.dispatch(request);").is_none());
}
