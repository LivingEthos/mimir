//! Criterion benchmark for `ContextPacket` construction.
//!
//! Benchmarks `ContextBuilder::build`, which (with a `repo_root` set) walks a
//! repository, builds an index, runs the retrieval pipeline, and assembles a
//! sealed packet. This is the dominant cost of cold/warm run startup, so the
//! same path stands in for startup latency. All inputs are local and synthetic
//! (an in-tempdir repo of a handful of `.rs` files); there are no provider or
//! network calls. Target: packet build < 500ms.

use std::path::Path;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mimir_context::ContextBuilder;
use mimir_runs::RunId;
use tempfile::TempDir;

/// Write a handful of small synthetic `.rs` files plus a guidance file into a
/// fresh tempdir, returning the directory handle so the repo stays on disk for
/// the duration of the benchmark.
fn synthetic_repo() -> TempDir {
    let dir = TempDir::new().expect("create tempdir for synthetic repo");
    let root: &Path = dir.path();

    // A small spread of modules with simple import/export relationships so the
    // index and retrieval pipeline have realistic-but-tiny graph to traverse.
    let files: &[(&str, &str)] = &[
        (
            "context_builder.rs",
            "pub struct ContextBuilder;\n\nimpl ContextBuilder {\n    pub fn build(&self) -> u32 {\n        42\n    }\n}\n",
        ),
        (
            "retrieval.rs",
            "use crate::context_builder::ContextBuilder;\n\npub fn retrieve() -> u32 {\n    ContextBuilder.build()\n}\n",
        ),
        (
            "index.rs",
            "pub struct RepoIndex {\n    pub files: u32,\n}\n\npub fn build_index() -> RepoIndex {\n    RepoIndex { files: 0 }\n}\n",
        ),
        (
            "packet.rs",
            "pub struct ContextPacket {\n    pub id: u32,\n}\n\npub fn seal(packet: ContextPacket) -> u32 {\n    packet.id\n}\n",
        ),
        (
            "util.rs",
            "pub fn token_estimate(text: &str) -> usize {\n    text.split_whitespace().count()\n}\n",
        ),
    ];
    for (name, body) in files {
        std::fs::write(root.join(name), body).expect("write synthetic source file");
    }

    // Repository guidance file (always-included path in the builder).
    std::fs::write(
        root.join("AGENTS.md"),
        "# Agent rules\n\nPrefer small, focused changes. Read this first.\n",
    )
    .expect("write synthetic guidance file");

    dir
}

fn bench_packet_build(c: &mut Criterion) {
    let repo = synthetic_repo();
    let root = repo.path().to_path_buf();

    c.bench_function("context_packet_build", |b| {
        b.iter(|| {
            let packet = ContextBuilder::new()
                .run_id(RunId("20260101-120000-abcdef00".to_string()))
                .task_card("Build the ContextBuilder retrieval packet")
                .provider("glm")
                .model("glm-5.1")
                .repo_root(black_box(root.clone()))
                .edit_targets(vec!["context_builder.rs".to_string()])
                .build()
                .expect("synthetic packet build should succeed");
            black_box(packet)
        });
    });
}

criterion_group!(benches, bench_packet_build);
criterion_main!(benches);
