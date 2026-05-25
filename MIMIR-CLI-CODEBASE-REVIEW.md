# Mimir CLI Codebase Review

Review date: 2026-05-19  
Scope: `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir` plus the handoff validation script in the parent directory.  
Secret handling: MiniMax and GLM keys were used only as transient shell variables for live smoke tests. They are not written in this report.

## Executive Summary

Mimir currently builds and its unit tests pass, but the shipped CLI is not yet a functional coding-agent CLI. The central provider path is mostly stubbed: `mimir ask`, `mimir context build`, `mimir context call`, `mimir plan`, and `mimir code` create placeholder artifacts or print "requires API key" messages instead of performing retrieval, gateway validation, provider calls, patch generation, or real repair. Packaging also has a release-blocking npm wrapper bug that causes `packages/cli/bin/mimir` to recursively call itself when no platform binary is installed.

Ratings:

| Dimension | Rating | Rationale |
| --- | --- | --- |
| Correctness | D+ | Core CLI commands return successful-looking output while doing placeholder work; run IDs drift between run directories and packets. |
| Security | C- | Secret redaction exists, but share/trace paths do not use it; patch application can escape the workspace if exposed to model-generated paths. |
| Provider readiness | D | Anthropic adapter exists, but CLI does not invoke it; no MiniMax or GLM/OpenAI-compatible provider support. |
| Maintainability | C | Reasonable crate boundaries and tests exist, but generated-schema docs break strict linting and many stubs are indistinguishable from shipped behavior. |
| Performance | B- | Local operations are fast enough for current stub behavior; real provider/retrieval performance is not measurable yet. |
| Packaging | F | Published npm bin wrapper can hang indefinitely without a copied platform binary. |

## Verification Matrix

| Check | Result | Notes |
| --- | --- | --- |
| `cargo test --workspace --all-targets` | Pass | 230 tests passed. Several warnings emitted. |
| `cargo build --release` | Pass | Built successfully in release mode, with warnings. |
| `cargo test -p mimir-cli --test integration_phase6` | Pass | 3 tests passed, but they only exercise help/version output. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Fail | Fails immediately on 139 missing-doc warnings in `crates/mimir-schemas/src/generated.rs`. |
| `cargo fmt --all -- --check` | Fail | Many formatting diffs across CLI, context, edit, retrieval, review, runs, security, server, subagents, and TUI crates. |
| `cargo audit` | Pass | No RustSec advisory failures found. |
| `cargo deny check` | Pass with warnings | Warns about unmatched license allowances, duplicate crate versions, and wildcard path dependencies. |
| `scripts/check-gateway-boundary.sh` | Fail | Flags `mimir-cli` references to `.mimir/runs`. The current script also matches user-facing strings. |
| Parent `node scripts/validate-examples.mjs` | Fail | Missing root Node dependencies: `ajv`, `ajv-formats`, `yaml`. No root `package.json` is present. |
| `target/debug/mimir --help` | Pass | CLI surface renders. |
| `target/debug/mimir ask --json 'Reply exactly OK'` | Pass but misleading | Creates a zero-token stub packet; no provider call. |
| `target/debug/mimir context call <packet> --stream --cache` | Fail behaviorally | Prints placeholder text; no provider call occurs. |
| `timeout 3s packages/cli/bin/mimir --help` | Fail | Times out with exit 124 because wrapper recurses into itself. |
| MiniMax direct API smoke | Provider reachable, quota blocked | OpenAI-compatible call reached MiniMax but returned HTTP 429 usage-limit response. |
| GLM direct API smoke | Pass | Z.AI coding endpoint returned HTTP 200 for `glm-5.1`; with `thinking` disabled, response content was `OK.` |

Provider documentation consulted:

- MiniMax documents OpenAI-compatible `https://api.minimax.io/v1`, Anthropic-compatible `https://api.minimax.io/anthropic`, and model IDs `MiniMax-M2.7` / `MiniMax-M2.7-highspeed`: https://platform.minimax.io/docs/token-plan/other-tools
- Z.AI documents the general endpoint `https://api.z.ai/api/paas/v4/` and Coding Plan endpoint `https://api.z.ai/api/coding/paas/v4`: https://docs.z.ai/guides/develop/http/introduction
- Z.AI documents `glm-5.1` usage and OpenAI SDK compatibility: https://docs.z.ai/guides/llm/glm-5.1

## Findings

### P0 - npm package bin recurses and hangs

Affected files:

- `packages/cli/bin/mimir:1-4`
- `packages/cli/install.js:29-43`

The published bin file is named `mimir`, and its fallback wrapper runs `"$SCRIPT_DIR/mimir" "$@"`. If `install.js` does not copy a platform binary over that path, the wrapper executes itself forever. `install.js` then exits 0 even when no binary was found, so npm installation can appear successful while the CLI is unusable.

Evidence:

- `timeout 3s packages/cli/bin/mimir --help` timed out with exit 124.

Recommendation:

- Use a different wrapper path and binary path, for example `bin/mimir` dispatching to `bin/mimir-${platformKey}`.
- If the optional platform binary is missing, fail postinstall with a clear error or install from a reliable release artifact.
- Add an npm smoke test that runs the packaged bin from a clean temp install.

### P0 - CLI provider commands do not call providers

Affected files:

- `crates/mimir-cli/src/main.rs:363-375`
- `crates/mimir-cli/src/main.rs:659-680`
- `crates/mimir-providers/src/adapters/anthropic.rs:271-380`

`mimir context call` only prints that a provider call would require `ANTHROPIC_API_KEY`; it never loads the packet, validates it through `ProviderGateway`, constructs `ProviderRequest`, calls `AnthropicAdapter`, streams, or writes usage/trace artifacts. `mimir ask` similarly writes a stub packet and exits. The adapter code exists but is not wired into the CLI flow.

Evidence:

- `target/debug/mimir context call .mimir/runs/.../context_packet.json --stream --cache` printed only placeholder text and exited 0.
- `target/debug/mimir ask --json 'Reply exactly OK'` returned a run/packet JSON with `tokens:0`.

Recommendation:

- Implement a real dispatch path: load packet -> validate packet and capability snapshot -> count server/local as policy requires -> call adapter -> write response, usage, budget ledger, trace, and audit event.
- Return non-zero for unsupported/stubbed functionality until implemented.
- Add integration tests with `wiremock` for count, call, streaming, error mapping, and cap refusal.

### P1 - Context packets are empty, unhashed, and use mismatched run IDs

Affected files:

- `crates/mimir-context/src/builder.rs:45-73`
- `crates/mimir-cli/src/main.rs:659-670`

`ContextBuilder::build` generates a new `RunId` internally, sets `packet_hash` to an empty string, sets `estimated_input_tokens` to 0, and includes no files, tools, evidence, memory, or omitted candidates. The CLI also creates a separate run directory ID before calling the builder, so the run directory and `ContextPacket.run_id` can diverge.

Evidence:

- `mimir ask --json` printed run ID `20260519-053948-7d018343`, but the saved packet contained `run_id: 20260519-053948-8f22127b`.
- Saved packet had empty `packet_hash`, empty `included`, and `estimated_input_tokens: 0`.

Recommendation:

- Pass the externally created run ID into `ContextBuilder`.
- Build from actual retrieval/index results and populate included/omitted/evidence/tool/memory sections.
- Compute `packet_hash` with the canonical packet hash function before writing.
- Add a regression test asserting run directory ID equals packet `run_id` and `packet_hash` is non-empty.

### P1 - Read-only commands create new run directories on misses

Affected files:

- `crates/mimir-runs/src/lib.rs:41-43`
- `crates/mimir-cli/src/main.rs:684-690`
- `crates/mimir-cli/src/main.rs:707-713`
- `crates/mimir-cli/src/main.rs:761-766`

`packet share`, `packet replay`, and `trace export` call `RunDir::create` before checking whether artifacts exist. This mutates `.mimir/runs` for invalid input, creating empty directories for missing run IDs.

Evidence:

- Running `mimir packet share missing-run-xyz` and `mimir trace export missing-trace-xyz --redact` returned errors but created `.mimir/runs/missing-run-xyz` and `.mimir/runs/missing-trace-xyz`.

Recommendation:

- Add `RunDir::open` or `RunDir::path_for_existing` that never creates directories.
- Use creation only for commands that start a new run.
- Add tests that missing read-only operations leave no filesystem artifacts.

### P1 - Patch application can escape the workspace

Affected file:

- `crates/mimir-edit/src/apply.rs:39-118`

Patch application joins model-provided paths directly against `base_path` without canonicalization, absolute-path rejection, `..` component rejection, symlink checks, or editable-set enforcement inside `PatchEngine::apply`. If model-generated patch steps ever reach this engine, a path such as `../outside.txt` or an absolute path can write/delete/move files outside the intended workspace.

Recommendation:

- Normalize each target path and reject absolute paths, parent components, Windows prefixes, and symlink escapes.
- Require `verify_editable_set` at the same boundary that applies patches, not only as a caller convention.
- Add tests for `../`, absolute paths, symlink traversal, and move destination escapes.

### P1 - Secret redaction is too narrow in export paths

Affected files:

- `crates/mimir-cli/src/main.rs:692-699`
- `crates/mimir-cli/src/main.rs:776-783`
- `crates/mimir-security/src/redactor.rs:13-34`

The project has a general redactor, but packet share only removes top-level `api_key` and `secrets` fields. Trace export only redacts `payload.api_key`. Nested values, bearer tokens, JWTs, database URLs, environment-style secrets, provider request bodies, and arbitrary event details can leak.

Recommendation:

- Apply `mimir_security::redact_secrets` recursively to all string leaves before any share/export/log output.
- Add fixtures with nested `Authorization`, JWT, `*_TOKEN`, DB URLs, and provider body examples.
- Treat redaction as a shared serializer rather than one-off field deletion.

### P1 - Provider gateway silently accepts unknown models

Affected file:

- `crates/mimir-providers/src/gateway.rs:43-58`

`ProviderGateway::validate` falls back to a 65,536-token cap when the model is unknown. This can allow unsupported or misspelled models through policy validation. It also ignores `packet.provider`, does not check provider mismatch, uses `max_context_tokens` instead of `max_input_tokens`, and does not include drift reserve in its `total`.

Recommendation:

- Reject unknown provider/model combinations with structured `gateway_unknown_model` or equivalent.
- Validate against `max_input_tokens` and include output reserve plus drift reserve.
- Compare the packet capability snapshot reference/hash with the registry entry before dispatch.

### P1 - MiniMax and GLM cannot be used through Mimir CLI today

Affected files:

- `crates/mimir-cli/src/main.rs:421-422`
- `crates/mimir-cli/src/main.rs:666-667`
- `crates/mimir-providers/src/adapters/anthropic.rs:46-55`
- `crates/mimir-providers/src/adapters/anthropic.rs:85-95`

The CLI hardcodes Anthropic provider/model defaults, and the adapter hardcodes `https://api.anthropic.com` plus `x-api-key` headers. MiniMax's OpenAI-compatible and Anthropic-compatible endpoints and Z.AI's OpenAI-compatible Coding Plan endpoint cannot be selected through CLI flags, config, or environment variables.

Evidence:

- MiniMax direct API call reached the service but returned HTTP 429 due current quota exhaustion.
- GLM direct API call to `https://api.z.ai/api/coding/paas/v4/chat/completions` using `glm-5.1` returned HTTP 200.
- Mimir itself could not route either provider because provider selection is not implemented beyond hardcoded packet metadata.

Recommendation:

- Add provider config for OpenAI-compatible endpoints: `OPENAI_BASE_URL`, `OPENAI_API_KEY`, model name, and provider name.
- Add Anthropic-compatible base URL/auth-token support for MiniMax-style endpoints.
- Surface `--provider`, `--model`, `--base-url`, and config-file equivalents.
- Include provider contract tests for Anthropic, MiniMax-compatible, and Z.AI-compatible response shapes.

### P1 - Documented CI gates are not green

Affected files:

- `README.md` development section
- `Cargo.toml` workspace lints
- `crates/mimir-schemas/src/generated.rs:15-42`
- `scripts/check-gateway-boundary.sh`

The README advertises `cargo clippy --workspace -- -D warnings`, but that fails because generated/schema-stub fields emit missing-doc warnings. `cargo fmt --check` also fails. The gateway boundary script fails on CLI user-facing strings containing `.mimir/runs`.

Recommendation:

- Run `cargo fmt --all`.
- For generated/schema stub types, either generate field docs from schema descriptions or add a scoped `#![allow(missing_docs)]` to generated code.
- Make gateway-boundary checks AST/import-oriented, or at minimum avoid matching string literals intended for user guidance.

### P2 - Integration tests are shallow

Affected files:

- `crates/mimir-cli/tests/integration_phase6.rs:5-45`
- `crates/mimir-server/tests/server_integration.rs:10-22`

The CLI integration tests spawn `cargo run` and assert only that help/version text contains certain words. The TCP server test binds to port 0, does not discover the assigned port, and only asserts that the spawned task is still running after a sleep.

Recommendation:

- Use `assert_cmd` with the built binary instead of nested `cargo run`.
- Add real CLI assertions for artifact contents, exit codes, missing-run behavior, provider mock calls, and JSON schema validity.
- For server TCP tests, bind a known free port or expose the listener address and send a real JSON-RPC request.

### P2 - `mimir code` is largely a test runner wrapper

Affected file:

- `crates/mimir-cli/src/main.rs:396-459`

The command builds an editable set from only top-level files when no `--editable` is supplied, despite the comment saying "all tracked source files". It creates a stub packet, runs `cargo test --workspace`, and the repair closure always returns no patches.

Recommendation:

- Populate editable defaults from `git ls-files` or explicit project policy.
- Generate and validate a real `PatchPlan`.
- Apply patches through the safety/edit engine, re-run targeted tests, and honor cost caps with provider usage.

### P2 - Parent schema/example validation is not reproducible

Affected files:

- Parent `scripts/validate-examples.mjs`
- Parent root package metadata is absent

The validation script imports `ajv`, `ajv-formats`, and `yaml`, but there is no parent `package.json` declaring those dependencies. Fresh clones cannot run the documented validation gate.

Recommendation:

- Add a root `package.json` with `devDependencies` and a `validate:examples` script.
- Commit a lockfile if reproducibility matters.
- Consider moving validation script/deps into the implementation repo if it is part of CI.

## Optimizations and Improvements

- Make stubs impossible to mistake for complete behavior: return `unimplemented`/non-zero for commands that cannot yet perform their advertised action.
- Centralize provider configuration in a typed registry loaded from `providers/*.yaml`, then snapshot the selected capability into each packet.
- Add `wiremock` tests for provider count/call/streaming, including 401, 429, 5xx, malformed JSON, and truncation responses.
- Replace ad hoc packet/traces writes with one artifact writer that handles atomic writes, redaction, schema validation, and audit events.
- Add a "golden CLI journey" test: `init -> ask/context build -> context call mocked provider -> packet replay/share -> trace export`.
- Tighten dependency hygiene: reduce duplicate versions where practical, remove wildcard path dependency warnings if `cargo deny` should be a high-signal gate.
- Keep GLM/MiniMax provider support protocol-based rather than vendor-specific where possible. OpenAI-compatible support will cover Z.AI Coding Plan and MiniMax OpenAI mode; Anthropic-compatible base URL support will cover MiniMax prompt-cache-oriented usage.

## Positive Observations

- The workspace is sensibly decomposed into CLI, context, providers, retrieval, edit, review, runs, memory, server, tools, telemetry, and TUI crates.
- Provider adapter code already has useful pieces: retry policy, error mapping, prompt-cache metadata parsing, local/server count separation, and response parsing.
- `cargo test --workspace --all-targets` passes, which gives a useful safety net for refactoring.
- The project already has schemas, examples, docs, ADRs, and a boundary-check concept. The shape is good; the implementation needs to catch up to it.

## Recommended Fix Order

1. Fix the npm wrapper/postinstall path so installs either work or fail loudly.
2. Make stubbed CLI commands return non-zero until real provider/retrieval behavior exists.
3. Wire `mimir context call` through a provider gateway using mockable adapters.
4. Fix `ContextBuilder` run ID/hash/token behavior and add schema-valid packet tests.
5. Harden patch path handling and redaction before any model-generated edit path is enabled.
6. Add OpenAI-compatible provider support and config flags, then test GLM/MiniMax through Mimir rather than raw `curl`.
7. Bring `fmt`, `clippy -D warnings`, and gateway-boundary checks into alignment with documented CI.

