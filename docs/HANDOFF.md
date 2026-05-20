# Mimir Development Handoff

## Current Status

**Version:** v0.6.0-phase6
**Branch:** phase6/memory-server-tui
**Tests:** 185 passing
**Commits:** 6 on current branch

## Completed Phases

### Phase 0 (v0.1.0-phase0) - Workspace Scaffold
- 12-crate Rust workspace initialized
- All 21 JSON schemas copied
- CLI commands: init, doctor, version, context build
- CI workflow, deny.toml, LICENSE, README
- 25 tests passing

### Phase 1 (v0.2.0-phase1) - Gateway & Provider Adapter
- Anthropic adapter with HTTP calls (POST /v1/messages, /count_tokens)
- Provider-neutral types (ProviderRequest, ProviderResponse, ToolSchema, etc.)
- Error mapping per 26-FIRST-PROVIDER-SPEC.md
- Retry logic for 429/529/5xx/408, fail-fast on 400/401/403/404/413
- Secret redactor with 18 patterns (AWS, GCP, Azure, Anthropic, OpenAI, Stripe, GitHub, Slack, JWT, private keys, env vars, passwords, API keys, DB URLs)
- Enhanced CLI: context build, inspect, budget, omitted, call
- 37 tests passing

### Phase 2 (v0.3.0-phase2) - Repo Map, Ranked Retrieval, Recall Guard
- **mimir-index**: File tree walking with .gitignore support, language detection (by extension + shebang), import/export extraction for Rust/TypeScript/Python, BLAKE3 content hashing, persistent index cache
- **mimir-retrieval**: 7-stage retrieval pipeline (cheap scan, structural expansion, semantic ranking, dedup/merge, greedy budget packing, sufficiency check, recall guard), ContextCandidate scoring, PackedManifest generation
- **mimir-context**: Recall guard with 5 risk categories (import_orphan, config_missing, schema_missing, test_missing, caller_missing), context why command
- **mimir-cli**: index, retrieve, context why subcommands
- 84 tests passing

### Phase 3 (v0.4.0-phase3) - Token Counting, Streaming, Prompt Caching
- **mimir-core**: Token counting integration with tiktoken-rs fallback
- **mimir-providers**: SSE streaming support, chunked delivery
- **mimir-providers**: Prompt caching with cache control headers, hit/miss tracking
- 120 tests passing

### Phase 4 (v0.5.0-phase4) - Review & Override
- **mimir-review**: Diff-based review with uninspected file detection
- **mimir-review**: Generated file edit detection
- **mimir-review**: Source-controlled checks framework
- **mimir-cli**: review subcommand with --committee and --checks flags
- 140 tests passing

### Phase 5 (v0.6.0-phase5) - Subagents
- **mimir-subagents**: Subagent registry with cost tiers
- **mimir-subagents**: Execute stub with evidence collection
- **mimir-cli**: agent subcommand with --list flag
- 158 tests passing

### Phase 6 (v0.6.0-phase6) - Memory, Server/SDK, TUI, Packaging
- **mimir-memory** (new crate): SQLite-backed memory store with FTS5 full-text search
- **mimir-memory**: Memory Decision Engine with weighted scoring signals
- **mimir-memory**: Marker-block publishing into `.mimir/project-rules.md`
- **mimir-memory**: Session importer stubs (Aider, Claude Code, Codex, OpenCode)
- **mimir-server** (new crate): JSON-RPC server backend with session management
- **mimir-server**: SessionStore with DashMap-backed concurrent storage
- **mimir-tui** (new crate): ratatui-based interactive terminal UI with 6 panels
- **mimir-cli**: memory, tui, serve subcommands
- 185 tests passing

## Architecture Invariants (Maintained)
- Only mimir-providers speaks HTTP to providers
- Only mimir-runs writes under .mimir/runs/
- Gateway boundary check script passes

## Next Steps (Phase 7)

Per 15-PHASES.md:

1. **Wire server transport** (mimir-server)
   - Real tower-lsp TCP/stdio transport
   - LSP initialization handshake

2. **Connect TUI to live data** (mimir-tui)
   - Load real context packets into panels
   - Connect to running server for live updates

3. **Implement session importers** (mimir-memory)
   - Aider conversation import
   - Claude Code/Codex/OpenCode session import

4. **Generate SDK types** (packages/sdk)
   - TypeScript type generation from schemas
   - NPM package wrapper

5. **Packaging** (cargo dist)
   - Cross-platform binary distribution
   - Homebrew formula

6. **Tag v0.7.0-phase7**

## Key Files
- `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir/` - Repo root
- `crates/mimir-memory/src/store.rs` - SQLite memory store with FTS5
- `crates/mimir-memory/src/engine.rs` - Memory Decision Engine
- `crates/mimir-memory/src/publish.rs` - Marker-block publishing
- `crates/mimir-server/src/rpc.rs` - JSON-RPC handlers
- `crates/mimir-server/src/session.rs` - Session management
- `crates/mimir-tui/src/lib.rs` - TUI app and event loop
- `crates/mimir-tui/src/panels/` - TUI panels (budget, included, omitted, etc.)
- `crates/mimir-cli/src/main.rs` - CLI entry point with memory/tui/serve commands
- `crates/mimir-cli/tests/integration_phase6.rs` - Phase 6 integration tests

## API Keys Available
- GLM 5.1 (Z.AI)
- MiniMax M2.7
- Kimi K2.6 (via Kimi For Coding)

## Commands
```bash
cd /Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir
. "$HOME/.cargo/env"
cargo test --workspace  # Run all tests (185 passing)
cargo build --release   # Build release binary
./target/release/mimir --help
./target/release/mimir memory --help
./target/release/mimir serve --help
```

## Open Questions
- Server transport is stubbed (needs tower-lsp wiring in Phase 7)
- TUI panels render placeholder data (needs real packet loading in Phase 7)
- Session importers are stubs (need actual tool integration in Phase 7)
- No SDK/TS types generated yet (needs schema codegen pipeline in Phase 7)
- No cargo dist or npm wrapper yet (Phase 7)

## Handoff Instructions
To continue development:
1. Read this HANDOFF.md
2. Read the relevant phase spec from Mimir-Hermes-Handoff/*.md
3. Run `cargo test --workspace` to verify baseline (185 tests)
4. Create branch `phase7/packaging`
5. Implement Phase 7 features
6. Run multi-model review at milestone

## Cleanup Note - 2026-05-20

The worktree contains several coherent dirty buckets from the production-hardening passes. Keep them separate when reviewing or committing:

1. Provider gateway boundary hardening: `crates/mimir-providers/src/{adapters,capabilities.rs,gateway.rs,lib.rs,count.rs,stream.rs}`, `crates/mimir-cli/src/main.rs`, `crates/mimir-cli/tests/provider_plan_code.rs`, `crates/mimir-server/src/{lsp.rs,rpc.rs}`, `crates/mimir-server/tests/server_integration.rs`, `providers/anthropic.yaml`, and `scripts/check-gateway-boundary.sh`.
2. CLI plan/code safety and repair hardening: `crates/mimir-cli/src/main.rs`, `crates/mimir-cli/tests/integration_phase6.rs`, `crates/mimir-edit/src/{apply.rs,backup.rs,git.rs,lib.rs,repair.rs,test_runner.rs}`, `crates/mimir-security/src/{lib.rs,redactor.rs}`, and related context/runs/review changes.
3. Schema, SDK, and example synchronization: `schemas/*.schema.json`, `schemas/README.md`, `examples/*.example.json`, `examples/README.md`, `packages/sdk/*.ts`, `packages/sdk/index.d.ts`, `packages/sdk/scripts/*.mjs`, `packages/sdk/package.json`, and `packages/sdk/package-lock.json`. The TypeScript schema mirrors and `index.d.ts` are generated; regenerate with `cd packages/sdk && npm run generate && npm run check:schema-drift && npm run build`.
4. Package distribution fixes: `packages/cli/bin/mimir`, `packages/cli/install.js`, and `packages/cli/README.md`.
5. Review and cleanup handoff material: `MIMIR-CLI-CODEBASE-REVIEW.md`, this note, and `scripts/validate-production.sh`.

Recommended commit order:

1. Gateway boundary and provider capability registry.
2. Schema/API wire-shape changes plus regenerated SDK and examples.
3. CLI plan/code execution, patch safety, artifact caps, and repair loop.
4. Package distribution wrapper/install fixes.
5. Validation script and handoff/review docs.

Validation commands:

```bash
cargo fmt --all -- --check
git diff --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test -p mimir-context -p mimir-providers -p mimir-schemas --doc
cargo build --release
./scripts/check-gateway-boundary.sh
cargo audit
cargo deny check
npm --prefix packages/sdk run generate
npm --prefix packages/sdk run check:schema-drift
npm --prefix packages/sdk run build
node --check packages/cli/bin/mimir
node --check packages/cli/install.js
node --check packages/sdk/scripts/generate.mjs
node --check packages/sdk/scripts/check-drift.mjs
node --check packages/sdk/scripts/build.mjs
npm --prefix .. run validate:examples
```

`cargo deny check` is expected to pass with warning-only notices for duplicate versions, wildcard path dependencies, and license allow-list entries that do not currently match a used crate. Provider credentials must stay environment-only; use only synthetic values in tests and fixtures.
