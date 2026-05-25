# Phase 6 Implementation Plan

## Goal
Complete Phase 6: Memory, Server/SDK, TUI, Packaging

## Current State
- 158 tests passing, 0 failed
- Branch: phase6/memory-server-tui
- Phases 0-5 complete

## Workstreams

### WS1: mimir-memory crate (SQLite + FTS + Decision Engine)
- SQLite store with rusqlite
- FTS5 over paths, symbols, packet IDs
- MemoryDecisionEngine with scoring
- Marker-block publishing to .mimir/project-rules.md
- Session importers (claude-code, codex, aider stubs)
- Commands: list, show, why, distill, forget, publish, import-sessions

### WS2: mimir-server crate (JSON-RPC)
- tower-lsp based JSON-RPC server
- Endpoints: /workspace/context-governor, /workspace/providers, /session/:id/...
-stdio and TCP modes

### WS3: mimir-tui crate (ratatui)
- Budget ledger panel
- Included ranges panel
- Omitted candidates panel
- Provider count panel
- Permissions panel
- Diff/review panel

### WS4: CLI Integration
- Wire MemoryCmd, ServeArgs, Tui into mimir-cli
- Add global flags
- Update Cargo.toml workspace

### WS5: Tests
- Unit tests for memory store, decision engine
- Integration tests for server
- TUI render tests
- End-to-end memory command tests

## Exit Gates
- Memory can be disabled without breaking core workflow
- All new tests pass
- Total test count > 200
- cargo build --workspace succeeds
- cargo clippy --workspace -- -D warnings passes
