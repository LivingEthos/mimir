//! Criterion benchmarks for Mimir session bootstrap paths.
//!
//! Covers the two provider-free entry points a fresh workspace hits first:
//!
//! * [`init_project_files`] — seeds the `.mimir` workflow scaffold (config,
//!   project rules, checks, command recipes) into an empty directory. This is
//!   the cost of `mimir init`. Target: < 100ms.
//! * [`TurnRunner::doctor`] — runs the local diagnostic probes (config parse,
//!   provider-capability registry, local token counter, context-packet build,
//!   write-permission check, credential presence). This is the cost of
//!   `mimir doctor`. Target: < 2s.
//!
//! All inputs are local and synthetic: a fresh tempdir per measured `init`
//! iteration (via `iter_batched`, so directory teardown is excluded from the
//! timed region) and a single pre-initialized tempdir reused for the doctor
//! probe. There are no provider or network calls.

use camino::Utf8PathBuf;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use mimir_session::{init_project_files, DoctorRequest, TurnRunner};
use tempfile::TempDir;

/// Create a fresh tempdir and return it alongside its UTF-8 path. The handle is
/// returned so the caller keeps the directory alive for as long as it is needed.
fn fresh_workspace() -> (TempDir, Utf8PathBuf) {
    let dir = TempDir::new().expect("create tempdir for bench workspace");
    let root = Utf8PathBuf::from_path_buf(dir.path().to_path_buf())
        .expect("tempdir path should be valid UTF-8");
    (dir, root)
}

/// Benchmark seeding the `.mimir` scaffold into a fresh, empty directory.
///
/// Uses `iter_batched` with a per-iteration tempdir setup closure so that
/// directory creation and teardown stay outside the measured region; only the
/// `init_project_files` call is timed.
fn bench_init_project_files(c: &mut Criterion) {
    let mut group = c.benchmark_group("session_init");
    group.sample_size(20);
    group.bench_function("init_project_files", |b| {
        b.iter_batched(
            fresh_workspace,
            |(_dir, root)| {
                let created = init_project_files(black_box(&root))
                    .expect("init_project_files should succeed");
                black_box(created)
            },
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

/// Benchmark the provider-free doctor probe over an initialized workspace.
///
/// The workspace is initialized once (its `.mimir/config.yaml` and friends are
/// what several probes read), then the diagnostic run is timed in a loop. The
/// permission probe writes and immediately removes a tiny marker file, so the
/// directory state is restored each iteration.
fn bench_doctor(c: &mut Criterion) {
    let (_dir, root) = fresh_workspace();
    init_project_files(&root).expect("seed .mimir scaffold for doctor bench");
    let runner = TurnRunner::for_workspace(root);

    let mut group = c.benchmark_group("session_doctor");
    group.sample_size(10);
    group.bench_function("doctor_probe", |b| {
        b.iter(|| {
            let request = DoctorRequest {
                version: "bench".to_string(),
            };
            let result = runner
                .doctor(black_box(request))
                .expect("doctor probe should succeed");
            black_box(result)
        });
    });
    group.finish();
}

criterion_group!(benches, bench_init_project_files, bench_doctor);
criterion_main!(benches);
