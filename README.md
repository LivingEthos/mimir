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

# Suggest starting context without a provider call
mimir context suggest "fix server refresh"

# Run source-controlled checks
mimir check --ci

# Persist read-only exploration evidence
mimir explore "where is packet replay handled?"

# Share or replay a portable redacted packet bundle
mimir packet share <run-id> --output shared-packet.json
mimir packet replay shared-packet.json --request-json

# Ask a provider for an implementation plan
mimir plan --editable src/lib.rs "Plan the change"

# Generate, validate, apply, and test a safe patch
mimir code --editable src/lib.rs --dry-run "Implement the change"
mimir code --editable src/lib.rs "Implement the change"
mimir code --editable src/lib.rs --recipe focused --param target=src/lib.rs "Implement the change"

# Import local session history as private provisional memory
mimir memory import-sessions --from codex path/to/session.jsonl
mimir memory import-sessions --from codex --discover --dry-run

# Inspect a saved packet in the TUI
mimir tui --packet .mimir/runs/<run-id>/context_packet.json
mimir tui --server 127.0.0.1:7788 --task "Refresh context for this repo"

# Open the local Studio UI
mimir ui
mimir serve --ui --port 7788

# Run the local context recall eval dataset
mimir eval context --dataset fixtures/context-recall-v1.yaml

# Start the local LSP/JSON-RPC server
mimir serve --rpc-stdio
mimir serve --port 7788
```

Provider credentials are environment-only (`GLM_API_KEY`, `ZAI_API_KEY`, `OPENAI_API_KEY`, or provider-compatible equivalents). `mimir code` requires explicit `--editable` paths, refuses pre-existing dirty target files, writes redacted run artifacts under `.mimir/runs/<run-id>/`, validates strict packet-bound patch recipes with a dry-run preflight, records provider-suggested tests without executing them, and can run a bounded repair loop when safe detected tests fail. Auto-run test subprocesses are launched with provider keys and generic secret-like environment variables stripped. If detected tests still fail, Mimir fails closed after writing artifacts and keeps the failed patch in the worktree for inspection.

See [docs/context-packets.md](docs/context-packets.md) for the build, share, and replay lifecycle.
See [docs/agent-workflows.md](docs/agent-workflows.md) for project guidance, context suggestion, source-controlled checks, code recipes, and read-only exploration.

For development work, start with [AGENTS.md](AGENTS.md) and [docs/HANDOFF.md](docs/HANDOFF.md). `AGENTS.md` is the short repo-entry guide; `docs/HANDOFF.md` tracks the current hardening status, dirty work buckets, validation commands, and release blockers.

## Workspace

- `crates/mimir-cli` — CLI entry point and command orchestration
- `crates/mimir-core` — orchestration layer
- `crates/mimir-context` — Context Governor (build, validate, hash)
- `crates/mimir-edit` — patch validation, application, test detection, repair loop
- `crates/mimir-eval` — context recall eval harness
- `crates/mimir-index` — repo index (files, imports, exports)
- `crates/mimir-memory` — durable lesson store, importers, publishing
- `crates/mimir-providers` — provider-neutral gateway and provider adapters
- `crates/mimir-retrieval` — ranked context retrieval
- `crates/mimir-review` — review, override, and source-controlled checks
- `crates/mimir-runs` — run directory layout (sole writer under `.mimir/runs/`)
- `crates/mimir-schemas` — JSON Schema types
- `crates/mimir-security` — safety classification, secret redaction
- `crates/mimir-server` — JSON-RPC/LSP server transport and session handling
- `crates/mimir-subagents` — subagent registry, cost tiers, evidence collection
- `crates/mimir-telemetry` — trace spans, audit events
- `crates/mimir-tools` — tool runner with safety classification
- `crates/mimir-tui` — terminal UI and live server refresh

## Development

Use focused checks while iterating, then run the full production gate before release handoff.

```bash
cargo fmt --all -- --check
cargo clippy -p <crate-name> --all-targets -- -D warnings
cargo test -p <crate-name> --all-targets
```

Full validation:

```bash
./scripts/validate-production.sh
```

Common broader checks:

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo build --release
```

## License

Apache-2.0
