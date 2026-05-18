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
```

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
