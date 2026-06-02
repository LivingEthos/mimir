# Performance Baselines

Targets come from [`17-PERFORMANCE-TARGETS.md`](../17-PERFORMANCE-TARGETS.md).
The numbers below are the **actual** criterion median point estimates committed
in [`bench/baselines.json`](../bench/baselines.json) (`generated_from:
"cargo bench"`, commit `dc3051d`), converted from nanoseconds to the unit each
key implies. All benches use local, synthetic, in-repo data only — no provider
or network calls.

## Current Metrics

| Metric | Target | Measured (median) | Bench id | Crate | Notes |
|--------|--------|-------------------|----------|-------|-------|
| `mimir init` | <100 ms | 1.415 ms | `session_init/init_project_files` | mimir-session | direct |
| Cold startup (warm cache) | <200 ms | 0.647 ms | `context_packet_build` | mimir-context | upper bound (bench rebuilds index each iter, i.e. no-cache path) |
| Cold startup (no cache) | <2 s | 0.647 ms | `context_packet_build` | mimir-context | small synthetic repo (~6 files) — underestimate vs real repo |
| Packet build (small task) | <500 ms | 0.647 ms | `context_packet_build` | mimir-context | direct |
| Repo index (10k files, cold) | <30 s | 0.0178 s | `repo_index/cold_build_index_600` | mimir-index | measured on **600** files; see corpus caveat |
| Repo index (incremental) | <2 s | 0.0184 s | `repo_index/incremental_reindex_600` | mimir-index | measured on **600** files; rewrite-one-file + rebuild |
| Token count (local) | <50 ms | 0.0069 ms | `count_local/102420` | mimir-providers | worst case = 100 KB input |
| TUI render frame | <16 ms | 0.461 ms | `render_frame_120x40` | mimir-tui | direct |
| Doctor command | <2 s | 0.00073 s | `session_doctor/doctor_probe` | mimir-session | direct |

### Directly measured vs representative-corpus

- **Directly measured** (the bench exercises the exact target path/scale):
  `mimir init`, packet build (small task), token count (local), TUI render
  frame, doctor command.
- **Representative corpus (extrapolation noted in `baselines.json`)**:
  - `repo_index_10k_cold_s` — the bench runs on a **600-file** synthetic tree,
    not 10k. `build_index` is roughly linear in file count, so 10k extrapolates
    to ~0.30 s (still far under the 30 s target). The committed value is the
    actual 600-file median, **not** the extrapolation. A 10k tree is too slow to
    materialise and iterate under criterion's default sampling.
  - `repo_index_incremental_s` — also 600 files; mimir-index has no diff-based
    incremental API, so the bench rewrites one source file and re-runs
    `build_index` with the OS page cache warm (the realistic "edit one file then
    reindex" path).
- **Conservative upper bound**: `cold_startup_warm_cache_ms` and
  `cold_startup_no_cache_ms` both map to `context_packet_build`, which rebuilds
  the `RepoIndex` from scratch every iteration (no `IndexCache` reuse). That is
  the no-cache path; a genuinely warm-cache build can only be faster, so the
  recorded value is a safe ceiling for the warm-cache target. The synthetic repo
  is tiny (~6 files), so the absolute figure underestimates a real cold start;
  it is recorded as the actual measured value, never inflated.

Each row's full rationale and corpus conditions live in the `measurement.<key>`
block of `bench/baselines.json`.

## Running the benches

```bash
. "$HOME/.cargo/env"   # repo convention: cargo 1.95

# Run a single crate's benches:
cargo bench -p mimir-providers
cargo bench -p mimir-context
cargo bench -p mimir-index
cargo bench -p mimir-tui
cargo bench -p mimir-session
```

Criterion writes machine-readable estimates to
`target/criterion/<bench-id>/new/estimates.json`. The `median.point_estimate`
field (nanoseconds) is the source of truth for the baseline numbers.

## How `bench/baselines.json` is generated

1. Run `cargo bench` for each of the five crates above.
2. Read each `target/criterion/<bench-id>/new/estimates.json` and take the
   `median.point_estimate` (ns).
3. Convert ns → ms (`/1e6`) or ns → s (`/1e9`) per the metric key suffix and
   write it into `metrics`. Record the bench id, corpus, and any
   extrapolation/caveat in the matching `measurement.<key>` block. Stamp
   `generated_from: "cargo bench"` and `generated_commit` (`git rev-parse
   --short HEAD`).
4. **No silent caps.** Never write a number that was not measured. If a target
   can only be reached by extrapolation, record the actual measured value and
   say so explicitly in the metric's notes.

## Regression checks

There are two complementary guards.

### 1. Deterministic guard (fast, CI-safe, never flaky)

`crates/mimir-core/tests/perf_regression.rs` — a normal Rust integration test
that loads `bench/baselines.json` and asserts structural invariants **without
running any bench or touching the clock**:

- the file parses and every expected metric key is present and numeric;
- every recorded baseline is `<=` its target (so a committed baseline can never
  silently claim a worse-than-target number);
- the `targets` block (if present) agrees with `17-PERFORMANCE-TARGETS.md`;
- the `ci_gates` fields are present and positive.

```bash
cargo test -p mimir-core --test perf_regression
```

### 2. Timing comparison (slow, real benches)

`scripts/check-perf-regression.sh` — runs the full bench suite, parses the fresh
criterion medians, maps each bench id to its metric, and fails if any metric
regressed beyond `ci_gates.bench_regression_threshold_percent` (currently 20%)
over the committed baseline.

```bash
scripts/check-perf-regression.sh             # run benches, then compare
SKIP_BENCH=1 scripts/check-perf-regression.sh   # reuse existing criterion data
```

It exits non-zero on regression (and on missing data), printing a per-metric
table of baseline vs fresh vs delta-%.

## CI gates (from `17-PERFORMANCE-TARGETS.md`)

CI fails if any of:

- `cargo test` takes >10× the prior baseline (`test_timeout_multiplier`).
- `mimir eval context` takes >5× the prior baseline (`eval_timeout_multiplier`).
- The bench suite shows >20% regression on any tracked metric without an ADR
  (`bench_regression_threshold_percent`).

When a target is missed: investigate and profile the hot path, optimize it, and
if it still cannot be met, file an ADR in `docs/adr/` proposing a new target.
Do not silently downgrade a baseline.
