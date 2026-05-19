# Phase 7 Implementation Plan — COMPLETE

## Goal
Complete Phase 7: Server Transport, TUI Live Data, Session Importers, SDK/TS Types, Packaging

## Status: ALL ITEMS COMPLETE — 222 tests passing, 0 failed

## Completed Workstreams

### WS1: mimir-server — Wire tower-lsp transport ✅
- Implemented `MimirLspBackend` with `tower_lsp::LanguageServer` trait in `crates/mimir-server/src/lsp.rs`
- Handles LSP lifecycle: `initialize`, `initialized`, `shutdown`
- Custom JSON-RPC methods via `mimir/ping`, `mimir/getSession`, `mimir/createSession`
- TCP (`TcpListener::bind`) and stdio (`tokio::io::stdin/stdout`) transports in `run_server()`
- 6 integration tests in `crates/mimir-server/tests/server_integration.rs`
- 4 unit tests in `crates/mimir-server/src/lib.rs`

### WS2: mimir-tui — Connect to live data ✅
- Added `--packet` and `--pipeline-result` CLI flags
- `App::load_packet()` and `App::load_pipeline_result()` methods
- Real PipelineResult loading into panels
- 5 tests in `crates/mimir-tui/src/lib.rs`

### WS3: mimir-memory — Session importers ✅
- `Importer` trait with `import()` method
- `AiderImporter` — parses `.aider.chat.history.md`
- `ClaudeCodeImporter` — parses `conversation.json`
- `CodexImporter` — parses `.jsonl` logs
- `OpenCodeImporter` — parses `.json` logs
- All in `crates/mimir-memory/src/importers/`
- 27 tests in `mimir-memory`

### WS4: SDK/TS Types ✅
- Generated 21 `.ts` files from JSON schemas using `json-schema-to-typescript`
- Resolved `$ref` URLs via local file resolution (sed-replaced `https://mimir.dev/schemas/` with `./`)
- `packages/sdk/package.json` — `@mimir/sdk` v0.7.0
- `packages/sdk/index.d.ts` — bundled declarations
- `packages/sdk/README.md` — usage docs

### WS5: Packaging ✅
- `cargo-dist` v0.31.0 installed
- `dist init --yes` generated `dist-workspace.toml`
- Targets: `aarch64-apple-darwin`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`
- `[profile.dist]` in `Cargo.toml`
- `.github/workflows/release.yml` generated

### WS6: Tests & Integration ✅
- Added tests across multiple crates:
  - `mimir-server`: +6 integration, +4 unit = 10 total
  - `mimir-telemetry`: +3 tests (new)
  - `mimir-eval`: +2 tests (new)
  - `mimir-runs`: +3 tests
  - `mimir-providers`: +4 retry tests, +8 error tests = 37 total
- **Final count: 222 tests passing** (was 185 at start, +37 total)

## Exit Gates — ALL PASSED
- [x] All new tests pass
- [x] Total test count > 220 (222)
- [x] `cargo build --workspace` succeeds
- [x] Server starts on TCP and stdio
- [x] TUI loads real context packets
- [x] Session importers implemented with tests

## Known Limitations Addressed
| Limitation | Resolution |
|------------|------------|
| Server transport stubbed | tower-lsp TCP + stdio wired, 10 tests |
| TUI panels placeholder data | `--packet` / `--pipeline-result` flags, live loading |
| Session importers stubs | 4 importer implementations with 27 tests |
| No SDK/TS types | 21 generated types, package.json, index.d.ts |
| No cargo dist | dist-workspace.toml, release workflow, 5 targets |
