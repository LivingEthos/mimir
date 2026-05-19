# Changelog

All notable changes to Mimir are documented in this file.

## [v1.0.0] - 2025-05-18

### Added
- `mimir ask` — answer questions with context retrieval
- `mimir packet share` — sanitize and export packets
- `mimir packet replay` — reproduce prompts from local artifacts
- `mimir override request` — cap override flow with audit logging
- `mimir trace export --redact` — trace portability
- Security tests: cap compliance 100%, prompt injection resistance
- Redactor coverage: 18 patterns with full test coverage
- Documentation: 5 ADRs, CLI exit codes, providers, security, performance
- cargo-dist packaging for 5 targets
- TypeScript SDK with 21 generated types

### Changed
- Server transport: tower-lsp TCP + stdio
- TUI panels: live data loading via `--packet` and `--pipeline-result`
- Session importers: Aider, Claude Code, Codex, OpenCode

### Fixed
- All 232 tests passing
- cargo audit clean
- cargo deny passes

## [v0.7.0-phase7] - 2025-05-18

### Added
- `mimir-server`: tower-lsp LanguageServer implementation
- `mimir-tui`: ratatui-based interactive terminal UI
- `mimir-memory`: SQLite-backed memory store with FTS5
- Session importers for external tools
- TypeScript SDK generation from JSON schemas
- cargo-dist release pipeline

## [v0.6.0-phase5] - 2025-05-18

### Added
- `mimir-subagents`: subagent registry and execution
- Cost-tier routing (Haiku/Sonnet)
- `mimir agent` subcommand

## [v0.5.0-phase4] - 2025-05-18

### Added
- `mimir review`: diff-based review with committee mode
- Override request flow
- Source-controlled checks framework

## [v0.4.0-phase3] - 2025-05-18

### Added
- Token counting with tiktoken-rs
- SSE streaming support
- Prompt caching with cache control headers

## [v0.3.0-phase2] - 2025-05-18

### Added
- `mimir index`: repo map generation
- `mimir retrieve`: ranked retrieval pipeline
- Recall guard for high-risk omissions

## [v0.2.0-phase1] - 2025-05-18

### Added
- Anthropic provider adapter
- Secret redaction
- Trace recording
- `mimir context build/inspect/budget/omitted/call`

## [v0.1.0-phase0] - 2025-05-18

### Added
- Workspace scaffold with 12 crates
- JSON schemas
- `mimir init/doctor/version`
- CI pipeline
