# AGENTS.md

This file is the first stop for coding agents and contributors working inside the Mimir implementation repo.

## Start Here

- Treat this directory, `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir`, as the repo root.
- The parent `Mimir-Hermes-Handoff` directory is the historical product/spec handoff. Use it for phase specs when needed, but prefer current implementation docs when they disagree.
- Read `docs/HANDOFF.md` before making changes. It has the current branch status, dirty work buckets, validation commands, and remaining release blockers.
- Read `README.md` for product shape and `V1.0-ROADMAP.md` for the current v1.0 exit gates.

## Working Rules

- Preserve existing worktree changes. This repo often has active hardening work in several dirty buckets; do not revert files unless the user explicitly asks.
- Start unfamiliar work with semantic code search or `rg`, then read only the files needed for the task.
- Use `mimir context suggest "<task>"` when you want a persisted, provider-free starting packet.
- Use `mimir explore "<question>"` when you want read-only evidence before planning or editing.
- Run `mimir check --ci` when source-controlled checks should gate local work.
- Scope every edit to the smallest relevant crate, package, script, or doc set.
- Prefer context-building before patching: map the flow, identify invariants, then edit.
- Use subagents for bounded exploration when available, especially for independent questions about separate crates or release workstreams.
- Keep generated files synchronized. Schema changes usually require regenerating the SDK mirrors in `packages/sdk`.

## Product Invariants

- The Context Governor should keep packets lean, hashable, replayable, and capped around 64k tokens by default.
- Provider credentials stay environment-only.
- Only `mimir-providers` speaks HTTP to model providers.
- Only `mimir-runs` writes under `.mimir/runs/`.
- `mimir code` must require explicit editable paths and fail closed on unsafe or uncertain edits.
- Run artifacts must be redacted, inspectable, and replayable when possible.
- JSON schemas are load-bearing contracts; update schemas before code when artifact shapes change.

## Workspace Map

- `crates/mimir-cli` - CLI entry point and command orchestration.
- `crates/mimir-core` - orchestration layer.
- `crates/mimir-context` - Context packet build, validation, policy, hashing, recall guard.
- `crates/mimir-edit` - patch validation, application, test detection, repair loop.
- `crates/mimir-eval` - context recall eval harness.
- `crates/mimir-index` - repo indexing.
- `crates/mimir-memory` - SQLite memory store, importers, publishing.
- `crates/mimir-providers` - provider-neutral gateway and provider adapters.
- `crates/mimir-retrieval` - ranked retrieval.
- `crates/mimir-review` - review, override, and source-controlled checks.
- `crates/mimir-runs` - run directory layout and `.mimir/runs/` writer.
- `crates/mimir-schemas` - JSON Schema types.
- `crates/mimir-security` - safety classification and secret redaction.
- `crates/mimir-server` - JSON-RPC/LSP server transport and session handling.
- `crates/mimir-subagents` - subagent registry, cost tiers, evidence collection.
- `crates/mimir-telemetry` - trace spans and audit events.
- `crates/mimir-tools` - tool runner with safety classification.
- `crates/mimir-tui` - terminal UI and live server refresh.
- `packages/cli` - private Node wrapper for native binary pack/install smoke tests.
- `packages/sdk` - generated TypeScript schema mirrors and SDK surface.

## Validation

Use the smallest check that covers the files you changed while iterating:

```bash
cargo fmt --all -- --check
cargo clippy -p <crate-name> --all-targets -- -D warnings
cargo test -p <crate-name> --all-targets
```

Common focused checks:

```bash
mimir context suggest "map the task before editing"
mimir explore "where is this flow handled?"
mimir check --ci
cargo test -p mimir-cli --all-targets
cargo test -p mimir-server --all-targets
cargo test -p mimir-tui --all-targets
cargo test -p mimir-context -p mimir-providers -p mimir-schemas --doc
npm --prefix packages/sdk run generate
npm --prefix packages/sdk run check:schema-drift
npm --prefix packages/sdk run build
```

Before release handoff, run the full gate:

```bash
./scripts/validate-production.sh
```

## Release Notes

Current v1.0 work is mostly production hardening: release archives, Homebrew checksums, private Node package smoke tests, final green CI, and preserving the packet sharing/replay and eval guarantees already wired into the repo.
