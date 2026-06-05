# ADR-009: Reversible Context Compression (RCC)

## Status

Accepted — implemented in v1.1

## Context

Mimir's core wedge is **deterministic, hashable, replayable context packets**. As repositories grow, the retrieval pipeline naturally produces candidates whose combined token count exceeds the provider's input cap (≈64k tokens). The naive solution is to omit large candidates entirely, but this loses information that the model could use.

Headroom's `Kompress-base` (a learned text compressor) was considered and explicitly rejected: a neural compression model is non-deterministic (weights, temperature, inference path), un-auditable, and breaks the replayability contract.

We need a compression strategy that:
1. Reduces token count deterministically.
2. Preserves the original verbatim for retrieval/replay.
3. Does not introduce any non-determinism into the packet path.

## Decision

Implement **deterministic, rule-based compressors** with **originals-on-disk**:

- **`CodeSkeleton`** — regex/line-based signature preservation for Rust/TS/JS/Python. Keeps imports, doc comments, and every function/struct/enum/class signature line; replaces body blocks with a single elision marker.
- **`JsonCrush`** — for homogeneous JSON arrays, emit a compact header+rows representation; for nested JSON, key-sorted pretty-print with truncated long strings.
- **`None`** — identity pass-through for unknown languages or when compression wouldn't help.

When a candidate exceeds `compress_threshold_tokens` (default 2048) and the compressed form achieves ≥25% reduction, the builder:
1. Includes the compressed text in the provider prompt.
2. Records compression metadata (`algorithm`, `original_tokens`, `compressed_tokens`, `original_hash`, `original_artifact_path`) in the packet's `IncludedItem.compression`.
3. Writes the **original** bytes to `.mimir/runs/<run_id>/artifacts/<hash>.orig` via `mimir-runs` (the only crate permitted to write there).

The `source_hash` in the packet remains the hash of the **original**, so replay verification is unchanged. `tokens` reflects the compressed body actually sent.

## Consequences

### Positive
- Large files that used to be omitted now appear in context in a condensed form.
- `mimir context expand <run-id> <path>` can retrieve the verbatim original, hash-verified, fail-closed on mismatch.
- Compression is pure (no I/O, no network, no RNG), so it is trivially deterministic and unit-testable.
- Prompt-cache stability is preserved: the cached prefix (system prompt, repo map) is built before compressed bodies enter the prompt.

### Negative
- Slightly increased builder complexity: two-pass budget accounting for promoted omitted candidates.
- Artifact storage grows by one `.orig` file per compressed candidate.
- Code skeletonization is regex-based, not AST-based; edge cases (macros, complex nesting) may produce sub-optimal skeletons. Tree-sitter is noted as a future upgrade.

## Alternatives Considered

- **ML compression (Kompress-base)**: Rejected on determinism grounds. A learned model is a black box; its output varies across versions and cannot be replayed.
- **On-the-fly compression without artifact storage**: Rejected because it breaks `mimir context expand` and makes replay impossible when the working-tree file changes.
- **Storing compressed text in the packet**: Rejected because `ContextPacket` is a metadata manifest, not a content store, and the schema intentionally keeps it lean.
