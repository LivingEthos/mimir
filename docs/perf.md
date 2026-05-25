# Performance Baselines

## Current Metrics (v1.0.0)

| Metric | Target | Actual | Status |
|--------|--------|--------|--------|
| `mimir init` | <100ms | ~50ms | PASS |
| Cold startup (warm cache) | <200ms | ~150ms | PASS |
| Cold startup (no cache) | <2s | ~1.2s | PASS |
| Packet build (small task) | <500ms | ~300ms | PASS |
| Repo index (10k files, cold) | <30s | ~25s | PASS |
| Repo index (incremental) | <2s | ~1.5s | PASS |
| Token count (local) | <50ms | ~20ms | PASS |
| TUI render frame | <16ms | ~8ms | PASS |
| Doctor command | <2s | ~1s | PASS |

## Measuring

Run benchmarks with:
```bash
cargo bench
```

Baselines are stored in `bench/baselines.json` and updated on `main` builds.

## CI Gates

CI fails if:
- `cargo test` takes >10x prior baseline
- `mimir eval context` takes >5x prior baseline
- Any benchmark shows >20% regression without ADR
