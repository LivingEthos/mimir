# Changelog

All notable changes to Mimir are documented in this file.

## [Unreleased]

### Added
- **Reversible Context Compression (RCC)** — deterministic, rule-based compressors (`CodeSkeleton`, `JsonCrush`) in new `mimir-compress` crate. Large files that exceed `compress_threshold_tokens` are compressed rather than omitted; originals are preserved under `.mimir/runs/<run_id>/artifacts/<hash>.orig` and retrievable via `mimir context expand`
- `mimir context expand <run-id> <path|hash>` — retrieve the verbatim original of a compressed or omitted candidate, hash-verified, fail-closed on mismatch
- `IncludedItem.compression` metadata field in `ContextPacket` schema (additive, optional, backward-compatible)
- `TokenPolicy` gains `compression_enabled` (default true) and `compress_threshold_tokens` (default 2048)
- Answer-quality eval tier `mimir eval answer` — opt-in harness that builds verbatim and compressed packets per case and reports offline input-token savings (CI-safe, no key required; seed fixture at `fixtures/answer-quality-v1.yaml`). Live provider answer-grading is scaffolded but not yet wired (tracked in `V1.1-ROADMAP.md`)
- ADR-009 documenting reversible context compression design rationale and trade-offs
- Cache-stability test asserting provider prompt is byte-identical across two builds of the same packet with compression enabled
- Failure-focused `TestCard` summarization in `mimir-review`: keeps failing test names + first error line per failure, drops passing noise, deterministic ordering

### Changed
- `ToolResultCard` struct now matches its schema: full fields including `card_id`, `command`, `cwd`, `safety_class`, `timeout_ms`, `duration_ms`, `stdout_preview`, `stderr_preview`, `estimated_tokens`, `inclusion_policy`, plus optional `*_artifact_path` and `*_original_size_bytes`
- `created_at` in `ContextPacket` is now derived from the explicit `run_id` timestamp when available, making `packet_hash` stable across rebuilds

## [v1.0.0] - 2026-05-22

### Added
- Criterion performance harness: 5 benches (`packet_build`, `repo_index`, `token_count`, `init_and_doctor`, `render_frame`) with committed median baselines in `bench/baselines.json` and measured-vs-representative notes in `docs/perf.md`
- Deterministic perf-regression guard `crates/mimir-core/tests/perf_regression.rs` — asserts every committed baseline is at or below its target without running a bench (CI-safe, never flaky), backed by the slow timing gate `scripts/check-perf-regression.sh`
- Cap-compliance gate `crates/mimir-eval/tests/cap_compliance.rs` — runs the full 15-case `context-recall-v1` fixture across every mode (0, 2, 3, 4, 5) and asserts every built packet stays at or below the 64000-token cap
- 19-pattern redactor corpus test `test_redactor_corpus_covers_every_pattern` — one synthetic sample per `PATTERNS` entry, kept 1:1 with the array so a new pattern without coverage fails the build
- Outbound-redaction tests `crates/mimir-cli/tests/outbound_redaction.rs` — plants synthetic secrets into every provider-bound CLI path and asserts the persisted `provider_request.redacted.json` artifact carries `<REDACTED:...>` markers, never the secret
- Rust gateway-boundary test `crates/mimir-providers/tests/gateway_boundary.rs` — dependency-free source scan asserting only `mimir-providers` imports an HTTP client and provider dispatch goes through `ProviderGateway`
- Risk-register regression tests: R-01 recall-guard indirect-dependency flag (`crates/mimir-cli`), R-02 token-count drift calibration (`crates/mimir-context/tests/token_drift_calibration.rs`), R-14 memory-pollution guard (imported entries must be `safe_to_send=false`)
- User-journey DOD tests: `journey_ask_code.rs` (end-to-end `ask`/`code` against an in-process mock provider) and `journey_init_doctor.rs` (provider-free `init`/`doctor` scaffold checks)

### Changed
- `mimir context inspect` now surfaces included items with their line ranges and omitted candidates with their `reason_for_omission`, in both text and `--json` output (`crates/mimir-cli/tests/context_inspect.rs`)

## [v1.0.0] - 2026-05-22

### Added
- `mimir ask` — answer questions with context retrieval
- `mimir packet share` — export portable redacted packet replay bundles
- `mimir packet replay` — verify local packets or shared bundles and emit byte-identical redacted provider requests
- `mimir override request` — cap override flow with audit logging
- `mimir trace export --redact` — trace portability
- Security tests: cap compliance 100%, prompt injection resistance
- Redactor coverage: 19 patterns with full test coverage
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
