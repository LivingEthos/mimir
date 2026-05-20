# Mimir

Replayable Context for coding agents.

## Overview

Mimir is a coding CLI built around a Context Governor that produces hashable, replayable context packets capped at ~64k tokens by default. Every prompt is an inspectable, shareable, replayable manifest.

## Quick Start

```bash
# Initialize a project
mimir init

# Check environment
mimir doctor

# Build a context packet
mimir context build

# Ask a provider for an implementation plan
mimir plan --editable src/lib.rs "Plan the change"

# Generate, validate, apply, and test a safe patch
mimir code --editable src/lib.rs --dry-run "Implement the change"
mimir code --editable src/lib.rs "Implement the change"
```

Provider credentials are environment-only (`GLM_API_KEY`, `ZAI_API_KEY`, `OPENAI_API_KEY`, or provider-compatible equivalents). `mimir code` requires explicit `--editable` paths, refuses pre-existing dirty target files, writes redacted run artifacts under `.mimir/runs/<run-id>/`, validates strict packet-bound patch recipes with a dry-run preflight, records provider-suggested tests without executing them, and can run a bounded repair loop when safe detected tests fail. Auto-run test subprocesses are launched with provider keys and generic secret-like environment variables stripped. If detected tests still fail, Mimir fails closed after writing artifacts and keeps the failed patch in the worktree for inspection.

## Workspace

- `crates/mimir-cli` — CLI entry point
- `crates/mimir-core` — Orchestration layer
- `crates/mimir-context` — Context Governor (build, validate, hash)
- `crates/mimir-providers` — Provider adapters (Anthropic-first)
- `crates/mimir-retrieval` — Ranked context retrieval
- `crates/mimir-index` — Repo index (files, imports, exports)
- `crates/mimir-tools` — Tool runner with safety classification
- `crates/mimir-runs` — Run directory layout (sole writer under `.mimir/runs/`)
- `crates/mimir-security` — Safety classification, secret redaction
- `crates/mimir-memory` — Durable lesson store
- `crates/mimir-eval` — Eval harness
- `crates/mimir-schemas` — JSON Schema types
- `crates/mimir-telemetry` — Trace spans, audit events

## Development

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo build --release
```

## License

Apache-2.0
