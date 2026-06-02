//! Criterion benchmark for local token counting.
//!
//! Exercises [`mimir_providers::count::count_local`] over synthetic text of
//! several sizes (~1 KB, ~10 KB, ~100 KB) built entirely in-bench. No network
//! or provider calls are made: tokenization runs purely on local, in-repo data.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mimir_providers::count::count_local;

/// Build a synthetic English-like string of approximately `target_bytes` bytes.
///
/// Uses a repeating sentence so the tokenizer sees realistic word boundaries
/// rather than a single degenerate token. The result is deterministic.
fn synthetic_text(target_bytes: usize) -> String {
    const SAMPLE: &str = "The quick brown fox jumps over the lazy dog. ";
    let mut text = String::with_capacity(target_bytes + SAMPLE.len());
    while text.len() < target_bytes {
        text.push_str(SAMPLE);
    }
    text
}

fn bench_count_local(c: &mut Criterion) {
    let mut group = c.benchmark_group("count_local");
    for size_bytes in [1_024_usize, 10_240, 102_400] {
        let text = synthetic_text(size_bytes);
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(text.len()), &text, |b, text| {
            b.iter(|| count_local(black_box(text)));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_count_local);
criterion_main!(benches);
