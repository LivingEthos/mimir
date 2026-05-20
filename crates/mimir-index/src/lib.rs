//! `mimir-index` — Repo index: files, imports, exports, language detection,
//! content hashing, and index caching.
//!
//! This crate provides:
//! - [`walk_files`]: file tree walker with `.gitignore` support
//! - [`detect_language`]: language detection by extension, shebang, and content sniff
//! - [`extract_imports`], [`extract_exports`]: regex-based import/export extraction
//! - [`content_hash`]: BLAKE3 hash for incremental indexing
//! - [`RepoIndex`]: the in-memory index with import/export graphs
//! - [`IndexCache`]: persistent cache keyed by content-hash + repo-hash

#![warn(missing_docs)]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during indexing operations.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// IO error while reading a file or directory.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    /// Serialization / deserialization error.
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    /// UTF-8 conversion error.
    #[error("invalid utf-8 in path")]
    InvalidUtf8,
}

/// Result alias for indexing operations.
pub type Result<T> = std::result::Result<T, IndexError>;

// ---------------------------------------------------------------------------
// Language detection
// ---------------------------------------------------------------------------

static EXT_MAP: Lazy<HashMap<&str, &str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    m.insert("rs", "rust");
    m.insert("py", "python");
    m.insert("js", "javascript");
    m.insert("mjs", "javascript");
    m.insert("cjs", "javascript");
    m.insert("ts", "typescript");
    m.insert("mts", "typescript");
    m.insert("cts", "typescript");
    m.insert("tsx", "typescript");
    m.insert("jsx", "javascript");
    m.insert("go", "go");
    m.insert("java", "java");
    m.insert("kt", "kotlin");
    m.insert("scala", "scala");
    m.insert("cpp", "cpp");
    m.insert("cc", "cpp");
    m.insert("cxx", "cpp");
    m.insert("c", "c");
    m.insert("h", "c");
    m.insert("hpp", "cpp");
    m.insert("rb", "ruby");
    m.insert("php", "php");
    m.insert("swift", "swift");
    m.insert("md", "markdown");
    m.insert("yaml", "yaml");
    m.insert("yml", "yaml");
    m.insert("json", "json");
    m.insert("toml", "toml");
    m.insert("sh", "shell");
    m.insert("bash", "shell");
    m.insert("zsh", "shell");
    m.insert("fish", "shell");
    m.insert("html", "html");
    m.insert("css", "css");
    m.insert("sql", "sql");
    m.insert("dockerfile", "dockerfile");
    m
});

/// Detect the programming language of a file from its path and optional contents.
///
/// Detection order:
/// 1. Extension lookup in [`EXT_MAP`].
/// 2. Shebang parsing (e.g. `#!/usr/bin/env python3`).
/// 3. Content sniff for common markers (e.g. `package.json` -> "json").
///
/// Returns `"unknown"` when no heuristic matches.
pub fn detect_language(path: &Path, content: Option<&str>) -> String {
    // 1. Extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        let ext_lower = ext.to_lowercase();
        if let Some(lang) = EXT_MAP.get(ext_lower.as_str()) {
            return (*lang).to_string();
        }
    }

    // 2. Shebang
    if let Some(text) = content {
        let first = text.lines().next().unwrap_or("");
        if first.starts_with("#!/") {
            let line = first.trim();
            if line.contains("python") {
                return "python".to_string();
            }
            if line.contains("node") || line.contains("nodejs") {
                return "javascript".to_string();
            }
            if line.contains("bash") || line.contains("sh") || line.contains("zsh") {
                return "shell".to_string();
            }
            if line.contains("ruby") {
                return "ruby".to_string();
            }
            if line.contains("perl") {
                return "perl".to_string();
            }
        }

        // 3. Content sniff for well-known filenames
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            let name_lower = name.to_lowercase();
            if name_lower == "dockerfile" || name_lower.starts_with("dockerfile.") {
                return "dockerfile".to_string();
            }
            if name_lower == "makefile" || name_lower == "gnumakefile" {
                return "makefile".to_string();
            }
        }
    }

    "unknown".to_string()
}

// ---------------------------------------------------------------------------
// Import / export extraction (regex-based)
// ---------------------------------------------------------------------------

static RUST_USE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)^\s*(?:pub\s+)?use\s+(?:crate::|self::|super::)?([a-zA-Z_][a-zA-Z0-9_]*(?:::[a-zA-Z_][a-zA-Z0-9_]*)*)"#,
    )
    .unwrap()
});

static RUST_PUB_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?m)^\s*pub\s+(?:fn|struct|enum|trait|type|const|static|mod|use)\s+([a-zA-Z_][a-zA-Z0-9_]*)"#,
    )
    .unwrap()
});

static TS_IMPORT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*import\s+(?:(?:\{[^}]*\}|\*\s+as\s+\w+|\w+)\s+from\s+)?['"]([^'"]+)['"]"#)
        .unwrap()
});

static TS_EXPORT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*export\s+(?:default\s+)?(?:function|class|interface|type|const|let|var|enum)?\s*([a-zA-Z_$][a-zA-Z0-9_$]*)?"#).unwrap()
});

static PY_IMPORT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*(?:from\s+([a-zA-Z_][a-zA-Z0-9_.]*)\s+import|import\s+([a-zA-Z_][a-zA-Z0-9_.]*(?:\s*,\s*[a-zA-Z_][a-zA-Z0-9_.]*)*))"#).unwrap()
});

static PY_DEF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)^\s*(?:async\s+)?def\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).unwrap());

static PY_CLASS_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)^\s*class\s+([a-zA-Z_][a-zA-Z0-9_]*)"#).unwrap());

/// Extract import specifiers from source text given its language.
///
/// Supported languages: `rust`, `typescript`, `javascript`, `python`.
/// For other languages an empty vector is returned.
pub fn extract_imports(content: &str, language: &str) -> Vec<String> {
    let mut out = Vec::new();
    match language {
        "rust" => {
            for cap in RUST_USE_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    out.push(m.as_str().to_string());
                }
            }
        }
        "typescript" | "javascript" => {
            for cap in TS_IMPORT_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    out.push(m.as_str().to_string());
                }
            }
        }
        "python" => {
            for cap in PY_IMPORT_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    out.push(m.as_str().to_string());
                } else if let Some(m) = cap.get(2) {
                    for part in m.as_str().split(',') {
                        let trimmed = part.trim();
                        if !trimmed.is_empty() {
                            out.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Extract exported symbol names from source text given its language.
///
/// Supported languages: `rust`, `typescript`, `javascript`, `python`.
/// For other languages an empty vector is returned.
pub fn extract_exports(content: &str, language: &str) -> Vec<String> {
    let mut out = Vec::new();
    match language {
        "rust" => {
            for cap in RUST_PUB_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    out.push(m.as_str().to_string());
                }
            }
        }
        "typescript" | "javascript" => {
            for cap in TS_EXPORT_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    out.push(m.as_str().to_string());
                }
            }
        }
        "python" => {
            for cap in PY_DEF_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    out.push(m.as_str().to_string());
                }
            }
            for cap in PY_CLASS_RE.captures_iter(content) {
                if let Some(m) = cap.get(1) {
                    out.push(m.as_str().to_string());
                }
            }
        }
        _ => {}
    }
    out
}

// ---------------------------------------------------------------------------
// Content hashing
// ---------------------------------------------------------------------------

/// Compute a BLAKE3 hash of file contents.
///
/// This is used for incremental indexing: if the hash hasn't changed,
/// the file does not need to be re-parsed.
pub fn content_hash(content: &[u8]) -> String {
    blake3::hash(content).to_hex().to_string()
}

// ---------------------------------------------------------------------------
// File walking
// ---------------------------------------------------------------------------

/// Walk a directory tree respecting `.gitignore` files.
///
/// Returns an iterator over relative [`PathBuf`]s from `root`.
/// Hidden directories and files are skipped by default.
pub fn walk_files(root: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
        .filter_map(move |e| {
            e.path()
                .strip_prefix(root)
                .ok()
                .map(std::path::Path::to_path_buf)
        })
}

// ---------------------------------------------------------------------------
// RepoIndex
// ---------------------------------------------------------------------------

/// A single file entry in the repo index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative path from repo root.
    pub path: String,
    /// Language (e.g., "rust", "typescript").
    pub language: String,
    /// BLAKE3 content hash.
    pub content_hash: String,
    /// Estimated token count.
    pub token_count: u32,
    /// Exported symbols.
    pub exports: Vec<String>,
    /// Imported symbols / modules.
    pub imports: Vec<String>,
}

/// The repo index.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct RepoIndex {
    /// Files by path.
    pub files: HashMap<String, FileEntry>,
    /// Import graph: path -> list of imported module specifiers.
    pub import_graph: HashMap<String, Vec<String>>,
    /// Export graph: path -> list of exported symbol names.
    pub export_graph: HashMap<String, Vec<String>>,
}

impl RepoIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file entry, updating import and export graphs.
    pub fn add(&mut self, entry: FileEntry) {
        self.import_graph
            .insert(entry.path.clone(), entry.imports.clone());
        self.export_graph
            .insert(entry.path.clone(), entry.exports.clone());
        self.files.insert(entry.path.clone(), entry);
    }

    /// Get a file by path.
    pub fn get(&self, path: &str) -> Option<&FileEntry> {
        self.files.get(path)
    }

    /// Total tokens across all files.
    pub fn total_tokens(&self) -> u32 {
        self.files.values().map(|f| f.token_count).sum()
    }

    /// Number of indexed files.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Returns `true` if the index contains no files.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Remove a file and its graph entries.
    pub fn remove(&mut self, path: &str) {
        self.files.remove(path);
        self.import_graph.remove(path);
        self.export_graph.remove(path);
    }

    /// Compute a stable hash of the entire index.
    ///
    /// The hash is derived from the sorted list of file paths and their
    /// individual content hashes, making it suitable for cache invalidation.
    pub fn index_hash(&self) -> String {
        let mut keys: Vec<&String> = self.files.keys().collect();
        keys.sort();
        let mut hasher = blake3::Hasher::new();
        for k in keys {
            hasher.update(k.as_bytes());
            if let Some(entry) = self.files.get(k) {
                hasher.update(entry.content_hash.as_bytes());
            }
            hasher.update(&[0]); // delimiter
        }
        hasher.finalize().to_hex().to_string()
    }
}

// ---------------------------------------------------------------------------
// IndexCache
// ---------------------------------------------------------------------------

/// Persistent on-disk cache for [`RepoIndex`] snapshots.
///
/// The cache key is `repo_hash + "/" + index_hash`, allowing quick lookup
/// when neither the repository structure nor individual file contents have
/// changed.
#[derive(Debug, Clone)]
pub struct IndexCache {
    root: PathBuf,
}

impl IndexCache {
    /// Open (or create) a cache directory.
    pub fn open(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    /// Return the path where a given cache key would be stored.
    pub fn cache_path(&self, key: &str) -> PathBuf {
        // Sanitize key for filesystem safety
        let safe = key.replace(['/', '\\', ':'], "_");
        self.root.join(format!("{}.json", safe))
    }

    /// Store an index under the given key.
    pub fn put(&self, key: &str, index: &RepoIndex) -> Result<()> {
        let path = self.cache_path(key);
        let json = serde_json::to_vec_pretty(index)?;
        atomic_write(&path, &json)?;
        Ok(())
    }

    /// Load an index by key if it exists.
    pub fn get(&self, key: &str) -> Result<Option<RepoIndex>> {
        let path = self.cache_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path)?;
        let index = serde_json::from_slice(&bytes)?;
        Ok(Some(index))
    }

    /// Remove a cached entry.
    pub fn invalidate(&self, key: &str) -> Result<()> {
        let path = self.cache_path(key);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("tmp");
    fs::write(&temp, contents)?;
    fs::rename(&temp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// High-level builder
// ---------------------------------------------------------------------------

/// Build a [`RepoIndex`] by walking `root` and parsing every supported file.
///
/// This is a convenience function; production callers may want to use
/// [`walk_files`], [`detect_language`], [`extract_imports`], etc. directly
/// for finer-grained control and parallelisation.
pub fn build_index(root: &Path) -> Result<RepoIndex> {
    let mut index = RepoIndex::new();
    for rel in walk_files(root) {
        let full = root.join(&rel);
        let bytes = fs::read(&full)?;
        let hash = content_hash(&bytes);
        let text = String::from_utf8_lossy(&bytes);
        let lang = detect_language(&rel, Some(&text));
        let imports = extract_imports(&text, &lang);
        let exports = extract_exports(&text, &lang);
        let entry = FileEntry {
            path: rel.to_string_lossy().to_string(),
            language: lang,
            content_hash: hash,
            token_count: estimate_tokens(&text),
            exports,
            imports,
        };
        index.add(entry);
    }
    Ok(index)
}

fn estimate_tokens(text: &str) -> u32 {
    (text.chars().count() as u32).div_ceil(4)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_get() {
        let mut index = RepoIndex::new();
        let entry = FileEntry {
            path: "src/main.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "abc".to_string(),
            token_count: 100,
            exports: vec!["main".to_string()],
            imports: vec![],
        };
        index.add(entry);
        assert!(index.get("src/main.rs").is_some());
        assert_eq!(index.get("src/main.rs").unwrap().token_count, 100);
    }

    #[test]
    fn total_tokens_sum() {
        let mut index = RepoIndex::new();
        index.add(FileEntry {
            path: "a.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "a".to_string(),
            token_count: 100,
            exports: vec![],
            imports: vec![],
        });
        index.add(FileEntry {
            path: "b.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "b".to_string(),
            token_count: 200,
            exports: vec![],
            imports: vec![],
        });
        assert_eq!(index.total_tokens(), 300);
    }

    #[test]
    fn detect_language_by_extension() {
        assert_eq!(detect_language(Path::new("foo.rs"), None), "rust");
        assert_eq!(detect_language(Path::new("bar.ts"), None), "typescript");
        assert_eq!(detect_language(Path::new("baz.py"), None), "python");
        assert_eq!(detect_language(Path::new("qux.go"), None), "go");
    }

    #[test]
    fn detect_language_by_shebang() {
        let py = "#!/usr/bin/env python3\nprint(1)\n";
        assert_eq!(detect_language(Path::new("script"), Some(py)), "python");
        let sh = "#!/bin/bash\necho hi\n";
        assert_eq!(detect_language(Path::new("script"), Some(sh)), "shell");
        let node = "#!/usr/bin/env node\nconsole.log(1)\n";
        assert_eq!(
            detect_language(Path::new("script"), Some(node)),
            "javascript"
        );
    }

    #[test]
    fn detect_language_dockerfile() {
        let content = "FROM rust:latest\n";
        assert_eq!(
            detect_language(Path::new("Dockerfile"), Some(content)),
            "dockerfile"
        );
    }

    #[test]
    fn extract_rust_imports() {
        let src = r#"
use std::collections::HashMap;
use crate::foo::bar;
pub use serde::Serialize;
"#;
        let imports = extract_imports(src, "rust");
        assert!(imports.contains(&"std::collections::HashMap".to_string()));
        assert!(imports.contains(&"foo::bar".to_string()));
        assert!(imports.contains(&"serde::Serialize".to_string()));
    }

    #[test]
    fn extract_rust_exports() {
        let src = r#"
pub fn hello() {}
pub struct Foo;
pub enum Bar {}
"#;
        let exports = extract_exports(src, "rust");
        assert!(exports.contains(&"hello".to_string()));
        assert!(exports.contains(&"Foo".to_string()));
        assert!(exports.contains(&"Bar".to_string()));
    }

    #[test]
    fn extract_typescript_imports() {
        let src = r#"
import { foo } from "./bar";
import * as baz from "baz-lib";
import "side-effect";
"#;
        let imports = extract_imports(src, "typescript");
        assert!(imports.contains(&"./bar".to_string()));
        assert!(imports.contains(&"baz-lib".to_string()));
        assert!(imports.contains(&"side-effect".to_string()));
    }

    #[test]
    fn extract_typescript_exports() {
        let src = r#"
export function foo() {}
export class Bar {}
export const BAZ = 1;
export default App;
"#;
        let exports = extract_exports(src, "typescript");
        assert!(exports.contains(&"foo".to_string()));
        assert!(exports.contains(&"Bar".to_string()));
        assert!(exports.contains(&"BAZ".to_string()));
        assert!(exports.contains(&"App".to_string()));
    }

    #[test]
    fn extract_python_imports() {
        let src = r#"
import os
import sys, json
from collections import OrderedDict
"#;
        let imports = extract_imports(src, "python");
        assert!(imports.contains(&"os".to_string()));
        assert!(imports.contains(&"sys".to_string()));
        assert!(imports.contains(&"json".to_string()));
        assert!(imports.contains(&"collections".to_string()));
    }

    #[test]
    fn extract_python_exports() {
        let src = r#"
def hello():
    pass

async def world():
    pass

class Foo:
    pass
"#;
        let exports = extract_exports(src, "python");
        assert!(exports.contains(&"hello".to_string()));
        assert!(exports.contains(&"world".to_string()));
        assert!(exports.contains(&"Foo".to_string()));
    }

    #[test]
    fn content_hash_stable() {
        let data = b"hello world";
        let h1 = content_hash(data);
        let h2 = content_hash(data);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn index_hash_stable_and_sensitive() {
        let mut idx1 = RepoIndex::new();
        idx1.add(FileEntry {
            path: "a.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "h1".to_string(),
            token_count: 0,
            exports: vec![],
            imports: vec![],
        });
        let mut idx2 = idx1.clone();
        assert_eq!(idx1.index_hash(), idx2.index_hash());

        idx2.add(FileEntry {
            path: "b.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "h2".to_string(),
            token_count: 0,
            exports: vec![],
            imports: vec![],
        });
        assert_ne!(idx1.index_hash(), idx2.index_hash());
    }

    #[test]
    fn walk_files_respects_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("keep.txt"), "keep").unwrap();
        fs::write(root.join("ignore_me.tmp"), "ignore").unwrap();
        fs::write(root.join(".gitignore"), "*.tmp\n").unwrap();

        let files: Vec<String> = walk_files(root)
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        assert!(files.contains(&"keep.txt".to_string()));
        assert!(!files.contains(&"ignore_me.tmp".to_string()));
        assert!(!files.contains(&".gitignore".to_string()));
    }

    #[test]
    fn index_cache_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = IndexCache::open(tmp.path()).unwrap();

        let mut index = RepoIndex::new();
        index.add(FileEntry {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "deadbeef".to_string(),
            token_count: 42,
            exports: vec!["foo".to_string()],
            imports: vec!["std::io".to_string()],
        });

        let key = "repoA/indexA";
        cache.put(key, &index).unwrap();
        let loaded = cache.get(key).unwrap().expect("cache miss");
        assert_eq!(loaded, index);
    }

    #[test]
    fn index_cache_invalidate() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = IndexCache::open(tmp.path()).unwrap();

        let mut index = RepoIndex::new();
        index.add(FileEntry {
            path: "x.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "c".to_string(),
            token_count: 0,
            exports: vec![],
            imports: vec![],
        });

        cache.put("k", &index).unwrap();
        assert!(cache.get("k").unwrap().is_some());
        cache.invalidate("k").unwrap();
        assert!(cache.get("k").unwrap().is_none());
    }

    #[test]
    fn build_index_smoke() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        let rust_src = r#"
pub fn add(a: i32, b: i32) -> i32 { a + b }
use std::collections::HashMap;
"#;
        let py_src = "def hello(): pass\n";

        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), rust_src).unwrap();
        fs::write(root.join("hello.py"), py_src).unwrap();
        fs::write(root.join(".gitignore"), "*.pyc\n").unwrap();

        let index = build_index(root).unwrap();
        assert_eq!(index.len(), 2);
        assert!(index.get("src/lib.rs").is_some());
        assert!(index.get("hello.py").is_some());

        let lib = index.get("src/lib.rs").unwrap();
        assert_eq!(lib.language, "rust");
        assert!(lib.exports.contains(&"add".to_string()));
        assert!(lib
            .imports
            .contains(&"std::collections::HashMap".to_string()));

        let py = index.get("hello.py").unwrap();
        assert_eq!(py.language, "python");
        assert!(py.exports.contains(&"hello".to_string()));
    }

    #[test]
    fn repo_index_remove() {
        let mut index = RepoIndex::new();
        index.add(FileEntry {
            path: "a.rs".to_string(),
            language: "rust".to_string(),
            content_hash: "h".to_string(),
            token_count: 0,
            exports: vec!["x".to_string()],
            imports: vec!["y".to_string()],
        });
        assert_eq!(index.len(), 1);
        index.remove("a.rs");
        assert_eq!(index.len(), 0);
        assert!(index.get("a.rs").is_none());
        assert!(!index.import_graph.contains_key("a.rs"));
        assert!(!index.export_graph.contains_key("a.rs"));
    }
}
