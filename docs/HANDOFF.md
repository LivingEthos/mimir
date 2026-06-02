# Mimir Development Handoff

## Current Status

**Version:** v1.0.0 production-readiness hardening in progress
**Branch:** phase6/memory-server-tui
**Tests:** full `./scripts/validate-production.sh` passing locally after eval and packaging-guard updates
**Commits:** current branch has Phase 7 cleanup commits plus this uncommitted hardening bucket

## v1.0 Exit-Gate Slices - 2026-06-02

Branch `phase7/v1-exit-gates` (off `phase6/memory-server-tui`). Four exit-gate
workstreams landed; all changes are test/doc/bench, no runtime behavior change.

- **Perf-bench harness.** Five criterion benches now exist with real `[[bench]]`
  `harness = false` entries: `packet_build` (mimir-context), `repo_index`
  (mimir-index), `token_count` (mimir-providers), `init_and_doctor`
  (mimir-session), `render_frame` (mimir-tui). `bench/baselines.json` now carries
  measured criterion medians (no longer hand-written), and `docs/perf.md` maps
  each metric to its bench id with directly-measured vs representative-corpus
  caveats. Two guards: deterministic `crates/mimir-core/tests/perf_regression.rs`
  (CI-safe, asserts every baseline <= target without timing) and the slow
  `scripts/check-perf-regression.sh` (real benches, 20% regression gate). Closes
  the perf-bench gap flagged in the 2026-06-01 note.
- **Security / compliance test gates.** Cap-compliance gate
  `crates/mimir-eval/tests/cap_compliance.rs` runs the 15-case
  `context-recall-v1` fixture across modes 0,2,3,4,5 and asserts every packet
  stays at or below the 64000-token cap. The redactor `PATTERNS` array is **19**
  (stale docs said 18, now corrected everywhere); `test_redactor_corpus_covers_every_pattern`
  keeps the corpus 1:1 with the array. Outbound-redaction tests
  (`crates/mimir-cli/tests/outbound_redaction.rs`) assert
  `provider_request.redacted.json` carries `<REDACTED:...>` markers, never the
  planted secret. A dependency-free Rust gateway-boundary test
  (`crates/mimir-providers/tests/gateway_boundary.rs`) asserts only
  `mimir-providers` imports an HTTP client; `scripts/check-gateway-boundary.sh`
  is wired into `.github/workflows/ci.yml`. Risk regressions: R-01 recall-guard,
  R-02 token-drift (`token_drift_calibration.rs`), R-14 memory-pollution
  (`safe_to_send=false`).
- **User-journey DOD tests + context inspect enhancement.** New end-to-end
  journey tests `journey_ask_code.rs` (ask/code against an in-process mock
  provider) and `journey_init_doctor.rs` (provider-free init/doctor scaffold).
  `mimir context inspect` now emits included items with line ranges and omitted
  candidates with their `reason_for_omission`, in both text and `--json`
  (`crates/mimir-cli/tests/context_inspect.rs`).
- **Docs completeness.** Three new ADRs landed — ADR-006 (fail-closed editing),
  ADR-007 (override auto-grant after repeated failures), ADR-008 (secret
  redaction) — bringing the named-topic set to five (003 gateway boundary, 002
  schema-as-contract, 006, 007, 008). `CHANGELOG.md` Unreleased section is
  current, and `docs/{cli-exit-codes,providers,security,perf}.md` are all present
  and grounded in current code (exit codes 0/1/2 only, with the 3–16/64/70/…
  scheme marked reserved/planned). The `V1.0-ROADMAP.md` exit gate "5 ADRs,
  CHANGELOG, docs complete" is now ticked.

## Override Audit, Prompt-Injection, and Security Slices - 2026-06-01

Branch `phase7/override-audit-and-injection` (off `phase6/memory-server-tui`).

- **`override request` now logs grants (DOD met).** Every request appends a
  redacted `override_requested` audit event to the run's `events.jsonl`
  (requested cap, reason, requester, threshold, prior failures). The
  `--auto-grant-after` threshold has real semantics, driven through
  `mimir-review`'s `OverrideManager`: prior failed attempts are counted from a
  run's `events.jsonl` (attach with `--run-id`; a fresh run starts at zero), and
  when the count meets the threshold a new `OverrideGrant` artifact
  (`override_grant.json`) plus a redacted `override_granted` event are written
  (`granted_by=auto_after_failures`). Added the `OverrideGrant` schema, example,
  generated Rust type, and SDK mirror. Auto-grant trigger: `prior_failures >=
  auto_grant_after` (so `--auto-grant-after 0` grants immediately). Failure event
  types counted: `cost_cap_aborted`, `repair_cost_cap_preflight_exceeded`,
  `patch_rejected`, `repair_patch_rejected`, `patch_tests_failed`,
  `override_attempt_failed`.
- **Prompt-injection containment test (DOD R-12 met).** New fixture
  `fixtures/prompt-injection/poisoned-context.md` and
  `crates/mimir-cli/tests/prompt_injection.rs` drive `mimir code` through the
  localhost-mock provider with a compromised model that obeys an injected
  "ignore previous instructions" payload and targets a path outside the
  `--editable` set, a `../` escape, and an absolute path. All three fail closed:
  injection reaches the model yet no out-of-set file is mutated and the run exits
  non-zero.
- **Two security slices.** `read_shared_packet_bundle` now rejects symlinks /
  non-regular files and caps on-disk size; `trace export --redact` now scrubs
  local filesystem paths (via new `mimir_runs::redact_trace_paths`) and refuses
  to write `--output` through a symlink.
- **Toolchain note:** clippy 1.95 promoted `get_first` / `manual_repeat_n` to
  deny-level; cleared three pre-existing lints (mimir-index, mimir-server test)
  so `cargo clippy --workspace --all-targets -- -D warnings` passes.
- **Still open (out of scope here):** the perf-bench gap — `bench/baselines.json`
  is hand-written static numbers with no `[[bench]]`/criterion harness.

## Agent Entry Point Update - 2026-05-23

- `AGENTS.md` now acts as the short first-read guide for future coding-agent sessions in this repo.
- Start work from `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir`; the parent handoff folder remains useful for historical phase specs, but this repo's current docs and CLI help win when details conflict.
- Use semantic search or `rg` first for unfamiliar flows, then read the minimum relevant files. Keep edits scoped to the smallest crate, package, script, or doc set.
- Prefer focused validation while iterating and reserve `./scripts/validate-production.sh` for release handoff or changes that cross multiple workstreams.
- The large-codebase operating model for Mimir itself is now explicit: context first, scoped edit targets, replayable evidence, subagent-assisted exploration when useful, and fail-closed validation.

## Agent Workflow Productization - 2026-05-23

- `mimir init` now seeds `.mimir/project-rules.md`, `.mimir/checks/no-provider-secrets.md`, `.mimir/commands/fast-check.md`, and `.mimir/commands/release-check.md` without overwriting existing files.
- Context packets now include safe repository guidance files with `reason_code=manifest_reference`: `.mimir/project-rules.md`, `AGENTS.md`, and `CLAUDE.md`; documentation-oriented tasks may also include `README.md` and `docs/HANDOFF.md`.
- `mimir context suggest "<task>"` writes a provider-free starting packet and reports guidance files, likely files, risky omissions, source-controlled check count, and next steps.
- `mimir check --ci --json` runs `.mimir/checks/*.md` source-controlled checks without provider calls and exits non-zero for blocking findings in CI mode.
- `mimir explore "<question>" --json` runs read-only search-subagent exploration and persists `.mimir/runs/<run-id>/explore_evidence.json`.
- `mimir code --recipe <name> --param key=value` validates code-mode `.mimir/commands/*.md` recipes, renders parameters, persists `.mimir/runs/<run-id>/command_recipe.json`, and keeps explicit `--editable` enforcement.
- New focused coverage lives in `crates/mimir-cli/tests/agent_workflow_cli.rs`; guidance inclusion tests live in `crates/mimir-context/src/builder.rs`.

## Packaging Verification Update - 2026-05-25

- cargo-dist artifacts are present locally under `target/distrib/` for all five configured targets: macOS arm64/x64, Linux arm64/x64, and Windows x64.
- Installed local release build tools needed to produce the cross-target artifacts from macOS: `zig`, `cargo-zigbuild`, and `cargo-xwin`.
- Staged all five native binaries with `scripts/stage-npm-platform-package.mjs`; every private Node platform package verification passes, and `node packages/cli/bin/mimir --version` reports `mimir 1.0.0` through the Node wrapper.
- `npm pack --dry-run --json` passes for the private `@mimir/sdk`, all five private `@mimir/cli-*` platform packages, and private root `@mimir/cli`.
- Homebrew formula checksums are populated for all macOS/Linux archives; `update-homebrew-checksums.mjs --check` and strict platform verification both pass against `target/distrib/`.
- `scripts/stage-npm-platform-package.mjs` now rejects missing flag values and unknown arguments instead of falling back to host defaults.
- npm registry publication is disabled by policy. The canonical GitHub repository is now `MisterWonderful/mimir`; local `origin`, package metadata, Homebrew URLs, and release tooling have been reattached to it. The stale local `v1.0.0` tag that pointed at `136771a` has been deleted so it cannot be pushed accidentally.

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
- **mimir-subagents**: Deterministic read-only local evidence execution
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

## Phase 7 / Production Readiness Status

Per 15-PHASES.md:

1. **Wire server transport** (mimir-server) — mostly complete
   - tower-lsp stdio transport is wired.
   - TCP transport accepts real framed LSP connections and now accepts multiple clients through a listener loop.
   - LSP `initialize` records `rootUri` / first `workspaceFolder` and context-building RPCs use it for retrieval-backed packets.

2. **Connect TUI to live data** (mimir-tui) — live server refresh now wired
   - `mimir tui --packet` loads real `ContextPacket` JSON into budget/provider/included/omitted panels.
   - `mimir tui --pipeline-result` loads real retrieval `PipelineResult` JSON when available.
   - Included/omitted panels fall back to packet data when no separate pipeline artifact exists.
   - `mimir tui --server 127.0.0.1:7788 --task "<task>"` connects to a running TCP `mimir serve --port 7788`, fetches a retrieval-backed packet, and refreshes on `r` or `--refresh-ms`.

3. **Implement session importers** (mimir-memory) — explicit import plus safe discovery complete
   - Importer implementations exist for Aider markdown, Claude Code JSON/JSONL, Codex JSONL/rollout JSONL, and OpenCode JSON/SQLite DB synthetic native shapes.
   - `mimir memory import-sessions --from <tool> <path>...` inserts imported entries into `.mimir/memory.db`; `--discover --dry-run` previews default locations without writing.
   - Imported entries are schema-valid, deterministic, `confidence=provisional`, `scope=private`, and `safe_to_send=false`.
   - Discovery is bounded by `--max-files`, uses environment roots only (`HOME`, `CODEX_HOME`, `XDG_DATA_HOME`), and does not include provider credentials.

4. **Generate SDK types** (packages/sdk) — complete with drift checks
   - TypeScript schema mirrors regenerate with `npm run generate`.
   - `index.d.ts` is now a real root declaration barrel for `import type { ContextPacket } from "@mimir/sdk"`.
   - Drift checks validate key wire-shape invariants.

5. **Packaging** (cargo dist / private Node pack smoke / Homebrew) — local artifacts and strict checks complete
   - cargo-dist workflow and private Node wrapper exist.
   - `mimir-cli`, private `@mimir/cli`, private `@mimir/sdk`, private Node platform-package manifests, and Homebrew artifact URL names are aligned to v1.0.0 / cargo-dist `mimir-cli-*` artifacts.
   - All five native platform packages have staged binaries and pass `npm pack --dry-run`.
   - Homebrew checksums match the local macOS/Linux cargo-dist archives.
   - Remaining release work: push from a GitHub account with write access, run CI on the exact release commit, create the `v1.0.0` tag, upload GitHub release assets, and then run Homebrew smoke checks against the live asset URLs.

6. **Context sharing / packet replay** — portable replay bundles now wired
   - `mimir packet share <run-id>` writes a redacted `mimir.packet_share` bundle by default, with the schema-valid packet plus the redacted provider request and checksums.
   - `mimir packet share <run-id> --packet-only` preserves metadata-only packet export for callers that need raw `ContextPacket` JSON.
   - `mimir packet replay <run-id> --request-json` emits the saved redacted provider request when available, falling back to deterministic reconstruction for build-only packets.
   - `mimir packet replay shared-packet.json --request-json` works from a fresh directory and emits byte-identical redacted request JSON.
   - `ask` and `context call` now write `provider_request.redacted.json`; plan/code already did.
   - Sharing still refuses packet metadata or provider request artifacts containing secret-like text.

7. **Context eval harness** — local context recall dataset now wired
   - `mimir eval context --dataset fixtures/context-recall-v1.yaml` runs 15 schema-shaped cases across modes 0, 2, 3, 4, and 5 without provider calls.
   - The command writes schema-valid `EvalResult` arrays under `.mimir/evals/` and prints aggregate recall, precision, cap compliance, and whether mode 4 beats mode 0 on mean file recall.
   - Current local release-binary smoke reports cap compliance and mode 4 beating mode 0, with partial full-recall case pass counts still visible in the summary.

8. **Tag v1.0.0** — pending GitHub release assets, Homebrew live-URL smoke checks, and green CI on the release commit

9. **Agent workflow productization** — provider-free entry points now wired
   - `init`, `context suggest`, `check`, and `explore` expose the large-codebase workflow as reusable CLI features.
   - Context packet guidance discovery is implemented with secret/size fail-closed behavior.
   - Code-mode `.mimir/commands/*.md` recipe execution is wired for `mimir code --recipe`; ask/validation recipe execution and richer provider-backed exploration remain future work.

## Key Files
- `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir/` - Repo root
- `AGENTS.md` - first-read guide for coding agents and contributors
- `README.md` - product overview, workspace map, and public development commands
- `docs/HANDOFF.md` - current development status, dirty work buckets, validation, and release blockers
- `docs/agent-workflows.md` - product documentation for init guidance, context suggest, checks, and exploration
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
cargo fmt --all -- --check
cargo clippy -p <crate-name> --all-targets -- -D warnings
cargo test -p <crate-name> --all-targets
cargo test --workspace  # Run workspace tests
cargo build --release   # Build release binary
./target/release/mimir --help
./target/release/mimir memory --help
./target/release/mimir serve --help
./target/release/mimir eval context --dataset fixtures/context-recall-v1.yaml
mimir context suggest "map current task before editing"
mimir explore "where is this flow handled?"
mimir check --ci
./scripts/validate-production.sh  # Full release handoff gate
```

## Open Questions
- Homebrew formula SHA256 values are complete for the local macOS/Linux artifacts.
- Private Node platform packages are scaffolded, staged, and have `prepack` guards.
- Parent handoff/spec docs still contain older command names in places; `docs/context-packets.md` and the CLI help reflect the current packet lifecycle.
- Code-mode `.mimir/commands/*.md` recipes are executable through `mimir code --recipe`; seeded ask-mode validation recipes are still documented workflow material until an ask/validation recipe runner is added.
- `mimir explore` uses deterministic read-only local search evidence; richer provider-backed exploration remains future work.
- Full production validation passes locally; final release still needs GitHub write access, green CI on the exact release commit, a new correct `v1.0.0` tag, uploaded GitHub release assets, and Homebrew smoke checks against the live asset URLs.

## Handoff Instructions
To continue development:
1. Read `AGENTS.md`
2. Read this HANDOFF.md
3. Read the relevant phase spec from Mimir-Hermes-Handoff/*.md only when needed
4. Run a focused crate/package check for the files you plan to touch, or `cargo test --workspace` for a broad baseline
5. Create branch `phase7/packaging` or a narrower branch matching the current workstream
6. Implement Phase 7 features
7. Run multi-model review at milestone

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
./target/release/mimir eval context --dataset fixtures/context-recall-v1.yaml
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
