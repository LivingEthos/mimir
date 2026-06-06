# Handoff — v1.1 Follow-ups (remaining roadmap items)

**Author:** Opus (review/planning) · **Implementer:** Kimi K2.6 · **Reviewer:** Opus (on return)
**Repo root:** `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir`
**Branch:** continue on `v1.1/reversible-compression` (5 commits already landed: `1f3dfe9`→`30a0550`).

This covers everything still open in [`V1.1-ROADMAP.md`](../V1.1-ROADMAP.md):

| ID | Item | Size | Priority |
|----|------|------|----------|
| **F** | Live provider dispatch + answer grading for `mimir eval answer` | Medium | **1 (do first)** |
| **G** | Larger answer-quality fixture set (50+ cases) | Small | 2 (pairs with F) |
| **H** | Model-callable `retrieve` tool (bounded, ≤3/run) | **Large** | 3 |
| **I** | Tree-sitter `CodeSkeleton` | Large + heavy deps | 4 (optional / may defer) |
| **J** | Close the exit gate: `validate-production.sh` green; v1.0 machinery untouched | Verify | run throughout + last |

---

## 0. Guardrails (unchanged from the RCC handoff — re-read before coding)

Read [`AGENTS.md`](../AGENTS.md) and [`docs/HANDOFF.md`](HANDOFF.md). All v1.0 invariants still bind:

1. **Deterministic, hashable, replayable.** No wall-clock / RNG / map-iteration-order / ML in the packet path. `hash_packet` hashes everything except token counts.
2. **Crate boundaries.** Only `mimir-providers` speaks HTTP. Only `mimir-runs` writes `.mimir/runs/`. Keep `scripts/check-gateway-boundary.sh` green.
3. **Schema-first.** schema → example → `crates/mimir-schemas/src/generated.rs` → consumers → regenerate SDK (`npm --prefix packages/sdk run generate && … check:schema-drift && … build`).
4. **`mimir code` stays `--editable`-gated and fail-closed.** The retrieve tool (H) must not become a file-read escape hatch.
5. **Cap gate stays green** (`crates/mimir-eval/tests/cap_compliance.rs`).
6. **Work one crate at a time, schema-first; keep `cargo test -p <crate>` green between steps.** Don't edit+compile two crates in one loop.

**Reusable assets you already have (verified):**
- `call_provider_with_request(packet, request, …)` — `crates/mimir-cli/src/main.rs` ~L1583. The canonical dispatch helper: validates the packet, selects the provider adapter (anthropic / openai-compatible / glm), honours `*_BASE_URL` overrides, returns the `ProviderResponse`. **The `eval answer` handler lives in `mimir-cli`, so it can call this directly — no extraction, no boundary issue.**
- `mimir_session::packet::provider_request_from_packet(workspace_root, packet, stream)` — builds the replayable provider request (with the compressed prompt) from a packet.
- `answer_provider_key_present(provider)` — `main.rs` (added in F's prep); env-key presence check per provider.
- Mock-provider test harness — `crates/mimir-cli/tests/journey_ask_code.rs`: `start_mock_provider(content)` spawns a localhost TCP server; the command is wired via `GLM_BASE_URL` + synthetic `GLM_API_KEY`. **This is how you test dispatch and the tool loop deterministically with no real key.**
- `run_context_expand(run_id, target, json)` — `main.rs` ~L894: hash-verified, fail-closed artifact retrieval. Factor its core out for H.

---

## Workstream F — Live answer grading for `mimir eval answer`

**Goal:** make `mimir eval answer` actually dispatch both arms to a provider, grade the answers, and report the verbatim-vs-compressed **accuracy delta** alongside the token delta — completing Workstream D's headline claim ("fewer tokens, *same answers*").

**Current state:** `crates/mimir-eval/src/answer_eval.rs` already has `build_answer_packets`, `grade_answer`, `summarize`, `AnswerQualityRun`, `AnswerQualitySummary`. `token_savings_report` (offline) is wired into the CLI. The dispatch + grading loop is the missing piece.

### Steps
1. **Keep dispatch in the CLI handler, grading/aggregation in the library.** In `main.rs` `EvalCmd::Answer`, after the existing offline `token_savings_report`:
   - If `!answer_provider_key_present(&provider)` → print the existing skip note and return (unchanged, CI-safe).
   - If a key is present: for each case, call `mimir_eval::answer_eval::build_answer_packets(case, &provider, &model)` → `(verbatim, compressed)`. For each arm: `provider_request_from_packet(workspace_root, &packet, false)?` then `call_provider_with_request(&packet, request, …).await?`, extract answer text from `ProviderResponse.content` (concatenate `ResponseBlock::Text` blocks), and build an `AnswerQualityRun { case_id, arm, answer, correct: grade_answer(&answer, &case.gold_answer, &case.grading), tokens_in: response.usage.input_tokens, provider, model }`.
   - Collect into `AnswerQualitySummary { dataset_id, arm_results }`, call `summarize(&summary)`, print the JSON (per-arm accuracy + mean tokens-in + the delta).
2. **`workspace_root`** for `provider_request_from_packet` is `case.repo_path` (the same root the packet was built against). Resolve to an absolute `Utf8Path`.
3. **No schema churn needed** — `EvalResult` already has `tokens_in` / `answer_correct` / `arm`. If you want to *persist* per-run results under `.mimir/evals/`, write schema-valid `EvalResult`s (match the existing context-eval writer in `mimir-eval`).
4. **Emit a `tokens_saved` + `accuracy_delta` summary line** so the win condition ("compressed matches verbatim accuracy at fewer tokens") is one glance.

### Tests
- **Mock-provider integration test** (`crates/mimir-cli/tests/`): reuse the `journey_ask_code.rs` harness. Start a mock that returns a fixed answer; point a 1-case dataset's `repo_path` at a temp repo; run `mimir eval answer --provider glm --dataset <tmp>.yaml` with `GLM_BASE_URL`+synthetic key; assert the JSON reports an `arms` block with `accuracy` and `mean_tokens_in` for both `verbatim` and `compressed`. Mark `#[ignore]`-free only if it uses the mock (no real network); it must not need a real key.
- **Unit**: `grade_answer` already covered; add one asserting `AnswerQualityRun.correct` flows into `summarize` accuracy.
- Keep the **no-key path** exercised by an existing/added test that asserts a clean skip with exit 0.

### Risks / DoD
- Async: the handler is already in tokio context — `call_provider_with_request` is `async`; `.await` it.
- **DoD:** with a mock provider, `mimir eval answer` prints per-arm accuracy + token deltas; with no key it cleanly reports offline savings only; `cargo test --workspace` stays green and key-free.

---

## Workstream G — Larger answer-quality fixture set (50+ cases)

**Goal:** a fixture set big and varied enough that F's accuracy comparison is meaningful, and that actually exercises compression.

### Steps
1. **Use small dedicated fixture repos, not `repo_path: .`.** Building the whole Mimir index ×50 ×2 arms is slow and non-hermetic. Create `fixtures/repos/<case-repo>/` with a handful of files each — including at least one **large file (>2048 tokens)** per compression-relevant case so the compressed arm differs from verbatim.
2. **Author `fixtures/answer-quality-v2.yaml`** (keep v1 as the smoke seed) with 50+ cases spanning: pure code-gen (no repo dependence), repo-retrieval-dependent Q&A (answer lives in a fixture file that only appears under compression), factual/format questions, and a few distractor-heavy cases. Vary `grading` across `exact_match` / `contains` / `contains_ci`.
3. **Determinism + safety:** no secrets, no network, stable gold answers. Run `mimir check --ci` mentality — fixtures must not trip the redactor.
4. **Loader already supports it** (`load_answer_dataset`); no code change unless you add fields (then schema-first).

### Tests / DoD
- A `load_answer_dataset` test that parses v2 and asserts ≥50 cases and that every case has non-empty `task` + `gold_answer`.
- `token_savings_report` over v2 (offline, in a test using the fixture repos) reports `tokens_saved > 0` in aggregate.
- **DoD:** v2 exists, parses, and demonstrably exercises compression on multiple cases.

---

## Workstream H — Model-callable `retrieve` tool (bounded, ≤3 per run)

**Goal:** let the model pull the verbatim original of a compressed/omitted candidate mid-run via a `retrieve` tool, bounded and replayable. This is the **largest** item: there is **no model-driven tool-call loop today** (`ResponseBlock::ToolUse { .. } => None` at `main.rs:1694`), so you are building one — keep it minimal and fail-closed.

### Design constraints (do not violate)
- **Bounded:** hard cap of **3 retrievals per run**; on the 4th request, stop the loop and fail closed (do not silently ignore).
- **Sandboxed:** `retrieve` may only return originals for candidates **already in this run's packet** (included-with-compression or omitted-with-stored-original). It is **not** a general file reader. Reuse `run_context_expand`'s resolution + hash-verification + secret-check — factor its core into a shared `fn resolve_run_original(run_dir, packet, target) -> Result<String>` and call it from both the CLI command and the tool handler.
- **Replayable:** the multi-turn exchange must be persisted so the run replays byte-identically. Today `provider_request.redacted.json` captures a single turn — H must persist the **turn sequence** (e.g. `provider_request.redacted.turn-N.json` / `response.turn-N.json`, mirroring the existing repair-loop artifact naming in `mimir-edit`). Replay (`mimir-session/packet.rs`) must reconstruct from the turn sequence.
- **Redaction:** run the tool-result text through `mimir_security::redact_secrets` before feeding it back; refuse (fail closed) if it would leak.

### Steps (phased)
1. **Shared resolver:** extract `resolve_run_original` from `run_context_expand` (`main.rs` ~L894-983); have the CLI command call it. No behaviour change — land + test this first.
2. **Tool schema:** register a `retrieve` `ToolSchema` (`crates/mimir-providers/src/types.rs::ToolSchema`) with params `{ "target": string }` (path or source_hash). Attach it to `tools` on the provider request for `plan`/`code` (and optionally `ask`) — gate behind a flag (e.g. `--enable-retrieve` or policy) so default behaviour is unchanged for v1.1.
3. **Bounded loop:** replace the `ResponseBlock::ToolUse { .. } => None` arm with a loop: when the response contains `ToolUse{name:"retrieve", input}`, call `resolve_run_original`, build a `tool_result` follow-up message, persist turn-N artifacts, re-dispatch via `call_provider_with_request`, incrementing a counter; break + fail-closed at 3. Non-`retrieve` tool names → fail closed (unknown tool).
4. **Run events:** append a redacted `retrieve_requested` / `retrieve_served` event to `events.jsonl` per retrieval (reuse the override-audit event pattern in `mimir-review`).

### Tests (mock provider)
- Mock returns a `ToolUse{retrieve, target}` on turn 1, then a final text answer on turn 2 → assert the original was served, one turn-1/turn-2 artifact pair exists, and the run replays.
- Mock requests `retrieve` 4× → assert the loop stops at 3 and the run exits non-zero (fail-closed).
- Mock requests `retrieve` for a path **not** in the packet → assert refusal.
- Replay test: `packet replay`/`context_prompt` reconstructs the multi-turn exchange.

### Risks / DoD
- This touches the replay/artifact model — the **biggest regression risk** in v1.1. Land the resolver (step 1) and schema (step 2) as separate green commits before the loop.
- If the multi-turn replay persistence proves too large for this pass, **ship steps 1–2 (resolver + registered-but-unused tool) and defer the loop**, documenting it — a half-wired loop is worse than none.
- **DoD:** bounded loop works against the mock, fail-closes at the cap and on out-of-packet targets, every retrieval is a recorded event, and the run replays. Default runs (no flag) are byte-identical to pre-H.

---

## Workstream I — Tree-sitter `CodeSkeleton` (optional; mind the caveat)

**Goal:** richer, more correct AST-aware elision than the current regex skeletonizer (handles multi-line signatures, macros, nested fns; extends cleanly to more languages).

### ⚠️ Replayability caveat — read first
Replay re-derives compressed text by re-running `compress_body` (`mimir-session/packet.rs::included_item_content`). The packet stores `compression.algorithm = "code_skeleton"` but **not** the implementation version. If you change what `code_skeleton` emits, **packets built before the upgrade will replay with different prompt text than was originally sent.** To preserve replayability you MUST version the algorithm:
- Introduce a **new** algorithm variant (e.g. `code_skeleton_ts`) rather than mutating `code_skeleton`, OR add a `compressor_version` field to `CompressionInfo` and have replay pick the matching implementation.
- This is a **schema change** (new enum value or new field) → schema-first, regenerate types/SDK/examples. Old packets keep using the old code path.

### Steps
1. Add `tree-sitter` + grammars (`tree-sitter-rust`, `-typescript`, `-python`, `-javascript`) to `crates/mimir-compress/Cargo.toml`. These pull native (C, `cc`-built) deps — **verify `cargo audit` + `cargo deny check` stay green** (they run in `validate-production.sh`); update `deny.toml` allow-lists only with justification.
2. Implement a `skeletonize_ts(content, language)` that parses to AST, keeps top-level + nested declaration signature lines + imports + doc comments, replaces body blocks with the elision marker. **Deterministic** (tree-sitter parsing is deterministic — invariant preserved).
3. Route the new variant via `select_algorithm` / a policy flag; keep the regex `code_skeleton` as the default + fallback for build-environments without the native grammars.
4. Keep `compress_body`'s signature and `CompressedBody` shape stable.

### Tests / DoD
- Determinism (same input → byte-identical output, twice).
- Signature preservation on multi-line-signature / macro / nested-fn fixtures the regex version mishandles.
- A replay test proving an **old** `code_skeleton` packet still reconstructs with the old algorithm after the new one is added.
- **DoD:** tree-sitter variant available + versioned, `cargo audit`/`deny` green, old packets still replay byte-identically.

> If the dependency weight or packaging impact (cargo-dist cross-compile of native grammars) is unacceptable, **defer I** and leave the regex skeletonizer — it is correct for the 4 supported languages. Note the decision in the roadmap.

---

## Workstream J — Close the exit gate

1. **Run the full gate** and fix anything it surfaces:
   ```bash
   ./scripts/validate-production.sh
   ```
   It runs: fmt, `clippy --workspace --all-targets -D warnings`, `test --workspace --all-targets`, doctests, gateway-boundary, **`cargo audit`**, **`cargo deny check`**, SDK `generate`/`check:schema-drift`/`build`, and `validate:examples`. (Workspace test + clippy + fmt are already green as of `30a0550`; this is the end-to-end confirmation incl. audit/deny/SDK.)
2. **Confirm v1.0 release machinery untouched:** no diffs under `dist-workspace.toml`, `HomebrewFormula/`, `packages/cli/`, cargo-dist config, or release scripts. (Workstreams F–I have no reason to touch these; verify with `git diff --stat 6a49148..HEAD -- dist-workspace.toml HomebrewFormula packages/cli`.)
3. Tick the two open exit-gate boxes in `V1.1-ROADMAP.md`.

---

## Suggested order

1. **F** (answer-grading dispatch) — completes D's headline; reuses `call_provider_with_request`.
2. **G** (fixtures) — makes F meaningful; do alongside F.
3. **J** (run the gate) once F+G land — checkpoint green.
4. **H** (retrieve tool) — phased: resolver → schema → bounded loop → replay; commit each green. Defer the loop if replay-persistence overruns.
5. **I** (tree-sitter) — only if dep/packaging weight is acceptable; honour the versioning caveat. Otherwise defer with a note.
6. **J** again — final full gate before review.

## Review checklist (what Opus verifies on return)
- [ ] `mimir eval answer` dispatches + grades against the mock provider; no-key path still a clean skip; CI stays key-free.
- [ ] v2 fixture set ≥50 cases, parses, exercises compression on multiple cases.
- [ ] `retrieve` tool: hard cap 3, fail-closed on cap + out-of-packet target, every retrieval a recorded event, run replays; **default runs byte-identical to pre-H**.
- [ ] If tree-sitter landed: algorithm **versioned**, old packets replay unchanged, `cargo audit`/`deny` green.
- [ ] Determinism preserved everywhere; only `mimir-providers` does HTTP; only `mimir-runs` writes runs; schema-first respected.
- [ ] `./scripts/validate-production.sh` green; v1.0 release machinery untouched; roadmap boxes ticked.

## Out of scope
- Anything in the v1.0 release machinery (tags, cargo-dist, Homebrew, Node packaging).
- Hosted/multi-tenant features; a general agentic tool loop beyond the bounded `retrieve`.
