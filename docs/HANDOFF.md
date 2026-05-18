# Mimir Development Handoff

## Current Status

**Version:** v0.3.0-phase2
**Branch:** main
**Tests:** 84 passing
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

## Architecture Invariants (Maintained)
- Only mimir-providers speaks HTTP to providers
- Only mimir-runs writes under .mimir/runs/
- Gateway boundary check script passes

## Next Steps (Phase 3)

Per 15-PHASES.md:

1. **Token counting integration** (mimir-core)
   - Replace rough word estimate with proper tokenizer
   - Server-side count endpoint fallback

2. **Streaming support** (mimir-providers)
   - SSE parsing for streaming responses
   - Chunked delivery to CLI

3. **Prompt caching** (mimir-providers)
   - Cache control headers
   - Cache hit/miss tracking

4. **Tag v0.4.0-phase3**

## Key Files
- `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir/` - Repo root
- `crates/mimir-index/src/lib.rs` - Repo index, file walking, language detection
- `crates/mimir-retrieval/src/lib.rs` - Retrieval pipeline, ranking, budget packing
- `crates/mimir-context/src/recall.rs` - Recall guard
- `crates/mimir-cli/src/main.rs` - CLI entry point
- `crates/mimir-security/src/redactor.rs` - Secret redaction

## API Keys Available
- GLM 5.1 (Z.AI)
- MiniMax M2.7
- Kimi K2.6 (via Kimi For Coding)

## Commands
```bash
cd /Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir
. "$HOME/.cargo/env"
cargo test --workspace  # Run all tests (84 passing)
cargo build --release   # Build release binary
./target/release/mimir --help
```

## Open Questions
- No official Anthropic tokenizer asset available (using server count endpoint)
- Local token count is rough word estimate (needs calibration)
- Streaming support not yet implemented
- Prompt caching not yet implemented

## Handoff Instructions
To continue development:
1. Read this HANDOFF.md
2. Read the relevant phase spec from Mimir-Hermes-Handoff/*.md
3. Run `cargo test --workspace` to verify baseline (84 tests)
4. Create branch `phase3/streaming-cache`
5. Implement Phase 3 features
6. Run multi-model review at milestone
