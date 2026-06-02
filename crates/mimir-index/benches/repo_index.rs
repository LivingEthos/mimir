//! Criterion benchmark for `mimir-index` repository indexing.
//!
//! Measures two scenarios against a *synthetic* repo materialised in a
//! tempdir (no network, no real provider calls):
//!
//! 1. **cold** — [`mimir_index::build_index`] over the whole synthetic repo
//!    from scratch (cold in-memory index; OS page cache warm after the first
//!    iteration, which is representative of a re-run on a checked-out repo).
//! 2. **incremental** — touch a single source file (rewrite its contents) and
//!    re-run `build_index`. `mimir-index` exposes no diff-based incremental
//!    API, so the realistic "incremental re-index" is a full rebuild with a
//!    warm cache; this is what an editor save-and-reindex actually pays.
//!
//! ## Corpus size
//!
//! The synthetic repo contains **600 small source files** spread across five
//! languages (Rust, TypeScript, Python, Go, JSON), laid out under a handful of
//! nested directories. This is deliberately ~1.5% of the 10k-file target named
//! in the perf goals (10k cold < 30 s / incremental < 2 s): a 10k-file tree is
//! too slow to materialise and iterate under criterion's default sampling, so
//! we measure a representative 600-file corpus and report that size honestly.
//! Scale the cold timing roughly linearly to estimate the 10k figure.

use std::fs;
use std::path::Path;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tempfile::TempDir;

/// Number of synthetic source files generated for the benchmark corpus.
const CORPUS_FILES: usize = 600;

/// Generate one small, language-appropriate source body for file index `i`.
///
/// Bodies carry a few imports/exports so the regex extractors in
/// `build_index` do real work rather than skipping empty files.
fn synthetic_source(lang_idx: usize, i: usize) -> (String, String) {
    match lang_idx {
        0 => (
            format!("src/rust/mod_{i}.rs"),
            format!(
                "use crate::util::helper_{prev};\n\
                 use std::collections::HashMap;\n\n\
                 pub fn compute_{i}(a: i32, b: i32) -> i32 {{ a + b + {i} }}\n\
                 pub struct Widget{i};\n",
                prev = i.saturating_sub(1),
            ),
        ),
        1 => (
            format!("src/ts/comp_{i}.ts"),
            format!(
                "import {{ helper{prev} }} from './comp_{prev}';\n\
                 import type {{ Thing }} from '../types';\n\n\
                 export function render{i}(x: number): number {{ return x + {i}; }}\n\
                 export const NAME_{i} = 'comp_{i}';\n",
                prev = i.saturating_sub(1),
            ),
        ),
        2 => (
            format!("src/py/module_{i}.py"),
            format!(
                "from .util import helper_{prev}\n\
                 import os\n\n\
                 def compute_{i}(a, b):\n    return a + b + {i}\n\n\
                 class Service{i}:\n    pass\n",
                prev = i.saturating_sub(1),
            ),
        ),
        3 => (
            format!("src/go/pkg_{i}.go"),
            format!(
                "package pkg{i}\n\n\
                 import \"fmt\"\n\n\
                 func Compute{i}(a, b int) int {{ return a + b + {i} }}\n\
                 func Print{i}() {{ fmt.Println({i}) }}\n",
            ),
        ),
        _ => (
            format!("data/config_{i}.json"),
            format!("{{ \"id\": {i}, \"name\": \"config_{i}\", \"enabled\": true }}\n"),
        ),
    }
}

/// Materialise the synthetic repo into a fresh tempdir and return it.
///
/// The [`TempDir`] is returned so the caller keeps it alive (and cleans it up
/// on drop). Round-robins across five languages so `detect_language`,
/// `extract_imports`, and `extract_exports` all exercise real branches.
fn build_corpus() -> TempDir {
    let dir = TempDir::new().expect("create tempdir");
    let root = dir.path();
    for i in 0..CORPUS_FILES {
        let (rel, body) = synthetic_source(i % 5, i);
        let full = root.join(&rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&full, body).expect("write synthetic source");
    }
    dir
}

/// Cold full index build over the whole synthetic corpus.
fn bench_cold(c: &mut Criterion) {
    let corpus = build_corpus();
    let root = corpus.path().to_path_buf();
    let mut group = c.benchmark_group("repo_index");
    group.sample_size(10);
    group.bench_function("cold_build_index_600", |b| {
        b.iter(|| {
            let index = mimir_index::build_index(black_box(root.as_path()))
                .expect("build_index over synthetic repo");
            black_box(index.len())
        });
    });
    group.finish();
}

/// Incremental re-index: rewrite one file, then rebuild with a warm cache.
fn bench_incremental(c: &mut Criterion) {
    let corpus = build_corpus();
    let root = corpus.path().to_path_buf();
    // Prime the index / OS page cache once so we measure the warm re-index path.
    let _ = mimir_index::build_index(root.as_path()).expect("prime build_index");

    let touch_target = root.join("src/rust/mod_0.rs");
    let mut counter: usize = 0;
    let mut group = c.benchmark_group("repo_index");
    group.sample_size(10);
    group.bench_function("incremental_reindex_600", |b| {
        b.iter(|| {
            // Mutate a single file so the rebuild reflects a real edit.
            counter = counter.wrapping_add(1);
            rewrite_touch_target(touch_target.as_path(), counter);
            let index = mimir_index::build_index(black_box(root.as_path()))
                .expect("incremental build_index");
            black_box(index.len())
        });
    });
    group.finish();
}

/// Rewrite the single "edited" file with a new body keyed on `n`.
fn rewrite_touch_target(path: &Path, n: usize) {
    let body = format!(
        "use crate::util::helper_{n};\n\n\
         pub fn compute_edited(a: i32) -> i32 {{ a + {n} }}\n\
         pub struct Edited{n};\n",
    );
    fs::write(path, body).expect("rewrite touch target");
}

criterion_group!(benches, bench_cold, bench_incremental);
criterion_main!(benches);
