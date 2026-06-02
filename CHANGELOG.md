# Changelog

All notable changes to Mimir are documented in this file.

## [v1.0.0] - 2026-05-22

### Added
- `mimir ask` — answer questions with context retrieval
- `mimir packet share` — export portable redacted packet replay bundles
- `mimir packet replay` — verify local packets or shared bundles and emit byte-identical redacted provider requests
- `mimir override request` — cap override flow with audit logging
- `mimir trace export --redact` — trace portability
- Security tests: cap compliance 100%, prompt injection resistance
- Redactor coverage: 18 patterns with full test coverage
- Documentation: 5 ADRs, CLI exit codes, providers, security, performance
- cargo-dist packaging for 5 targets
- TypeScript SDK with 21 generated types
- Live TUI refresh from a running TCP `mimir-server`
- Safe session discovery for Aider, Claude Code, Codex, and OpenCode
- private Node platform package manifests for native CLI binary package smoke tests
- `mimir eval context --dataset fixtures/context-recall-v1.yaml` for local context recall/cap checks
- Release checklist and v1.0.0 release notes

### Changed
- Server transport: tower-lsp TCP + stdio
- TUI panels: live data loading via `--packet`, `--pipeline-result`, and `--server`
- Session importers: Aider, Claude Code, Codex, OpenCode with private provisional memory semantics
- Release metadata aligned around `mimir-cli`, private Node package manifests, and cargo-dist `mimir-cli-*` artifacts
- `ask` and `context call` now write `provider_request.redacted.json` artifacts for later packet sharing
- private Node platform packages now fail `prepack` if native binaries have not been staged

### Fixed
- Full production validation passing
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
