# Handoff — v1.1 Reversible Context Compression (RCC) + Quality Eval

**Author:** planning pass (Opus) · **Implementer:** Kimi K2.6 · **Reviewer:** Opus (on return)
**Repo root:** `/Users/frisson1/Downloads/Mimir-Hermes-Handoff/Mimir`
**Origin:** ideas adapted from `chopratejas/headroom` (compression layer), reframed to fit Mimir's replayability contract.

---

## 0. Read first / guardrails

This repo is at **v1.0.0 readiness**: 531 tests pass, fmt + workspace clippy `-D warnings` clean, `./scripts/validate-production.sh` green, every `18-DEFINITION-OF-DONE.md` exit gate ticked except the GitHub `v1.0.0` tag (needs write access + CI — **out of scope here, do not touch it**).

Everything below is **v1.1**. It must not regress v1.0. Before writing code, read `AGENTS.md` and `docs/HANDOFF.md`.

### Hard invariants (breaking any of these fails review)

1. **Deterministic, hashable, replayable.** `crates/mimir-context/src/hash.rs::hash_packet` hashes the whole packet except token-count fields. Every byte you add to a packet must be a **pure function of inputs**. No wall-clock, no RNG, no map iteration order, no ML inference in the packet path.
2. **No ML compressor.** Headroom's `Kompress-base` is explicitly rejected — a learned lossy text model is non-deterministic and opaque, which is the opposite of Mimir's wedge. Only deterministic, rule-based compressors (code skeleton, JSON crush).
3. **Reversible, never destructive.** Compression must preserve the original. The original is written under `.mimir/runs/<run-id>/artifacts/` and is retrievable. The packet carries the original's `source_hash` so replay can verify.
4. **Schema-first.** Schemas in `schemas/*.schema.json` are load-bearing contracts (`additionalProperties:false` / `unevaluatedProperties:false`). Order of changes is always: **schema → example → `crates/mimir-schemas/src/generated.rs` → consuming code → regenerate SDK mirror**. Never edit generated types ahead of the schema.
5. **Crate boundaries.** Only `mimir-providers` speaks HTTP. Only `mimir-runs` writes under `.mimir/runs/`. Keep `./scripts/check-gateway-boundary.sh` green.
6. **Cap gate stays green.** `crates/mimir-eval/tests/cap_compliance.rs` asserts every packet ≤ 64000 tokens across modes 0,2,3,4,5. Compression should make this *easier*, never break it.
7. **`mimir code` stays fail-closed** and `--editable`-gated. Nothing here changes edit safety.

### Branch & sequencing

- Branch from the current clean tip (`6a49148`): `git checkout -b v1.1/reversible-compression`.
- **Work one crate at a time, schema-first, and run the focused check after each crate before moving on.** Do not edit + compile two crates in the same loop — sequence the schema/type change, get it green, then move to consumers. (Parallel same-tree edits race; keep it linear.)
- Commit per workstream using the repo's conventional-commit style (see `24-COMMIT-AND-PR-CONVENTIONS.md`). Keep the dirty buckets from `docs/HANDOFF.md` §Cleanup separate — don't fold this work into them.

### Validation (run the smallest that covers your change while iterating)

```bash
. "$HOME/.cargo/env"
cargo fmt --all -- --check
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate> --all-targets
# schema/SDK sync after any schema change:
npm --prefix packages/sdk run generate && npm --prefix packages/sdk run check:schema-drift && npm --prefix packages/sdk run build
# full gate before handing back:
cargo test --workspace --all-targets && ./scripts/validate-production.sh
```

---

## Workstream A — Deterministic body compressors + reversible store  *(core; highest value)*

**Goal:** When a candidate's full body would be omitted for `budget_overflow` (or exceeds a size threshold), compress its body deterministically and include the compressed form **instead of dropping it**, while preserving the original for retrieval. This is the direct adaptation of Headroom's SmartCrusher/CodeCompressor, made replayable.

### A1. New crate `mimir-compress`

Create `crates/mimir-compress` (add to workspace `Cargo.toml` members; mirror an existing small crate's manifest, e.g. `mimir-security`). Pure library, **no I/O, no network, no provider deps** — so it's trivially deterministic and unit-testable.

Public API:

```rust
pub enum CompressionAlgorithm { None, CodeSkeleton, JsonCrush }

pub struct CompressedBody {
    pub algorithm: CompressionAlgorithm,
    pub text: String,            // the compressed rendering that enters the packet
    pub original_hash: String,   // sha256 hex of the ORIGINAL bytes
    pub original_tokens: u32,
    pub compressed_tokens: u32,
}

/// Pure function of (content, language, target). Deterministic.
pub fn compress_body(content: &str, language: &str, target_tokens: u32) -> CompressedBody;
```

Algorithms (all deterministic, rule-based):

- **`CodeSkeleton`** (Rust/TS/JS/Python — same languages `mimir-index` already handles): keep module-level doc lines, imports, and every function/struct/enum/class **signature line**; replace each body block with a single elision marker `// … <N> lines elided …` (use the language's comment token). Reuse the regex signature patterns that already exist in `crates/mimir-index/src/lib.rs` (`RUST_PUB_RE`, `TS_EXPORT_RE`, `PY_DEF_RE`, `PY_CLASS_RE`, lines ~165–196) so the definition of "signature" stays consistent with retrieval. Do **not** add tree-sitter — regex/line-based skeletonization is deterministic and dependency-light; note tree-sitter as a future upgrade only.
- **`JsonCrush`**: for `.json`/JSON tool output that is an array of homogeneous objects, emit a header row of keys + compact rows (CSV-ish) instead of repeated key names; for nested/heterogeneous JSON, fall back to key-sorted pretty-print with long string values truncated to a cap and a `"…(+N chars)"` marker. Must round-trip-describe, not round-trip-reconstruct (lossy bodies, lossless originals on disk).
- **`None`**: identity (used when compression wouldn't help or language unknown).

Token counts via `mimir_providers::count::count_local` is a provider dep — to keep `mimir-compress` dependency-light, take `original_tokens`/`compressed_tokens` as computed by the **caller** (builder), or accept a `tok: &dyn Fn(&str)->u32` closure. Prefer the closure so the crate stays pure.

**Tests (in-crate):** for each algorithm, assert (a) byte-identical output across two calls on the same input (determinism), (b) `compressed_tokens < original_tokens` on a representative fixture, (c) signatures are preserved for code, (d) unknown language ⇒ `None`/identity.

### A2. Schema change — `IncludedItem` gains compression metadata

In `schemas/ContextPacket.schema.json`, the `included[]` items currently carry `path`, `ranges`, `tokens`, `source_hash`, etc. Add an **optional** object:

```jsonc
"compression": {
  "type": ["object", "null"],
  "required": ["algorithm", "original_tokens", "compressed_tokens", "original_hash", "original_artifact_path"],
  "properties": {
    "algorithm": { "type": "string", "enum": ["none", "code_skeleton", "json_crush"] },
    "original_tokens": { "type": "integer", "minimum": 0 },
    "compressed_tokens": { "type": "integer", "minimum": 0 },
    "original_hash": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
    "original_artifact_path": { "type": "string", "description": "Path under .mimir/runs/<run_id>/artifacts/ holding the verbatim original body." }
  },
  "additionalProperties": false
}
```

Then: update `examples/ContextPacket.example.json` (add one compressed item so `crates/mimir-schemas/tests/p0_schemas.rs` validates it), regenerate `crates/mimir-schemas/src/generated.rs`, then regenerate the SDK mirror. `schema_version` on `ContextPacket` stays the same only if the field is additive+optional and all existing examples still validate — confirm `p0_schemas.rs` passes; if it forces a bump, bump and update the backward-read note per `05-SCHEMAS.md`.

### A3. Wire into the builder

`crates/mimir-context/src/builder.rs`, in `build_retrieved_context()` around the body-load site (lines ~234–305):

1. After reading the file (`fs::read_to_string`, ~line 261) and counting tokens (~line 265), if the candidate is `full_file`/large and either exceeds a configurable `compress_threshold_tokens` **or** the greedy pack flagged it as `budget_overflow`, call `mimir_compress::compress_body`.
2. If `compressed_tokens` meaningfully beats `original_tokens` (e.g. ≥25% reduction) **and** the compressed form fits the remaining budget: include the compressed `text` as the item body, set `IncludedItem.compression`, and write the **original** bytes to `.mimir/runs/<run-id>/artifacts/<original_hash>.orig` **via a `mimir-runs` writer** (add one if needed — only `mimir-runs` may write there). `source_hash` stays the hash of the original (so replay/verify is unchanged); `tokens` reflects the compressed body actually sent.
3. Re-run the budget accounting so a previously-omitted candidate can now be promoted to included. Keep the omission path for anything that still doesn't fit (`reason_for_omission: budget_overflow`).

**Determinism check:** compression decisions must depend only on packet inputs (content, language, budget, threshold), never on iteration order. Add a test that builds the same packet twice and asserts identical `packet_hash` (extend `crates/mimir-context` tests; there's prior art in `hash.rs` tests).

### A4. Policy knob

Add `compress_threshold_tokens` and `compression_enabled` (default on) to the context policy (`crates/mimir-context/src/policy.rs`) so it can be disabled for a fully-verbatim packet — useful as a determinism escape hatch and for A/B in the eval (Workstream D).

**Definition of done (A):** large files that used to be omitted now appear compressed; original retrievable on disk; `packet_hash` stable across rebuilds; cap gate still green; new `mimir-compress` unit tests + a builder determinism test pass.

---

## Workstream B — Reversible retrieval: `mimir context expand` (+ gated model tool)  *(CCR)*

**Goal:** let a consumer (and, as a stretch, the model) pull the verbatim original of a compressed-or-omitted candidate on demand. This is Headroom's CCR, but Mimir-flavored: deterministic and replayable.

### B1. CLI command (ship this — provider-free)

Add `mimir context expand <run-id> <path|source_hash>` in `crates/mimir-cli/src/main.rs`:
- Resolve the run's packet, find the matching `included[].compression.original_artifact_path` (or an `omitted_candidates[]` entry), read the original from `.mimir/runs/<run-id>/artifacts/`, print it (respect `--json`).
- Verify the on-disk bytes hash to the recorded `original_hash`; **fail closed** with a clear error if they don't.
- Add `crates/mimir-cli/tests/context_expand.rs` covering: expand a compressed item, hash-mismatch rejection, unknown id error, redaction preserved (never print secrets the packet wouldn't).

### B2. Model-callable `retrieve` tool — **STRETCH, gate it**

There is currently **no model-driven tool-call loop**: `ResponseBlock::ToolUse` is recognized but mapped to `None` (`crates/mimir-cli/src/main.rs` ~line 1555). A full agentic loop is out of scope for v1.1. If — and only if — time allows after A, C, D are solid:

- Register a `retrieve` `ToolSchema` (`crates/mimir-providers/src/types.rs::ToolSchema`) describing `{ source_hash | path }`.
- Add it to the `tools` on `plan`/`code` provider requests, and implement a **bounded** handler: when the model returns `ToolUse{name:"retrieve"}`, read the artifact, append a `tool_result`, and re-call — capped at **max 3 retrievals per run**, fail-closed on cap. Every retrieval is itself a recorded run event so the run stays replayable.
- If you don't get here, **leave it unregistered** and document it as planned. Do not ship a half-wired tool loop.

**Definition of done (B):** `mimir context expand` works from a fresh dir against a shared/replayed packet, hash-verified, with tests. Model tool either fully bounded+tested or absent.

---

## Workstream C — Real ToolResultCard / TestCard summarization  *(fix a latent schema gap)*

**Goal:** make tool/test output compression real, and reconcile a struct that currently violates its own schema.

### C1. Reconcile `ToolResultCard` struct ↔ schema

The in-code struct `crates/mimir-tools/src/lib.rs` (lines ~12–23) has only `{schema_version, tool_name, stdout, stderr, exit_code}`, but `schemas/ToolResultCard.schema.json` mandates `card_id, command, cwd, safety_class, timeout_ms, duration_ms, stdout_preview, stderr_preview, estimated_tokens, inclusion_policy` plus optional `*_artifact_path`, `*_original_size_bytes`, `detected_file_refs`, `detected_test_refs`, `filters_applied`. Bring the struct (and `run_command`, ~lines 31–57) into line with the schema:
- Produce **capped previews** (`stdout_preview`/`stderr_preview`) and spill the **full** output to `.mimir/runs/<run-id>/artifacts/` (via `mimir-runs`), recording `*_artifact_path` and `*_original_size_bytes`.
- Populate `safety_class` from the existing `mimir_security::classify_command`, set `inclusion_policy` (`preview_only` by default, `summary_only` for large output), and run lightweight `detected_file_refs`/`detected_test_refs` extraction (regex for path-like / test-id tokens).

### C2. Real `TestCard` summarization

`crates/mimir-review/src/test_runner.rs::summarize_test_result` (lines ~8–20) is a stub that just formats a status line. Replace with failure-focused summarization off `TestRunResult` (`crates/mimir-edit/src/test_runner.rs`): keep failing test names + first assertion/error line per failure, drop passing-test noise, cap the preview, spill full logs to artifacts. Deterministic ordering (sort failures by name). Add tests with a multi-failure fixture.

**Definition of done (C):** `ToolResultCard` round-trips through its schema (add/extend a schema-validation test); test summaries are failure-focused and deterministic; full outputs are on disk, previews in the packet.

---

## Workstream D — Provider-backed answer-quality eval tier  *(proves the wedge)*

**Goal:** demonstrate "fewer tokens, same answers" with numbers, the way Headroom validates against GSM8K/SQuAD — but as an **opt-in, key-gated** tier that never runs in CI.

The current eval (`crates/mimir-eval`) is **provider-free** (recall/precision/cap only; `result_from_packet` stubs out `tokens_out_total`/`cost_usd_total`). Keep that gate **exactly as-is** (it's green). Add a new tier alongside it:

- New subcommand `mimir eval answer --provider <p> --dataset <yaml> [--compare verbatim,compressed]`. Requires an API key in env; if absent, exits with a clear "skipped: no key" status (so it's safe to invoke anywhere).
- For each case: build the packet **twice** (compression on vs `compression_enabled=false` from Workstream A4), send both to the provider, grade the answer against `gold` (exact-match / contains / a simple rubric), and report: task-success rate per arm, mean tokens-in, and the success **delta**. The win condition is "compressed arm matches verbatim arm on success while spending fewer input tokens."
- Extend `EvalMetrics`/`EvalResult` schemas with optional `answer_correct: bool|null`, `tokens_in: int`, `arm: string`. Schema-first as always.
- Seed a **small** fixture set (5–10 cases) under `fixtures/answer-quality-v1.yaml`; design the loader so the set can grow. Do not vendor a giant benchmark — scope is "harness + seed", not "full GSM8K".
- Mark the tier `#[ignore]`/feature-gated in test code so `cargo test --workspace` stays provider-free and deterministic.

**Definition of done (D):** `mimir eval answer` runs against a real provider when a key is present, prints per-arm success + token deltas, writes schema-valid `EvalResult`s under `.mimir/evals/`, and is invisible to CI/default test runs.

---

## Workstream E — Cross-cutting tasks to do at the same time

These are cheap, related, and best done in the same branch:

1. **CacheAligner check (cost defense).** `26-FIRST-PROVIDER-SPEC.md:116` already adds `cache_control` to stable prefixes (system prompt, repo map, project memory), targeting >50% prompt-cache hit. Add a test in `mimir-providers` (or `mimir-context`) asserting the cache-control prefix segment is **byte-stable across two builds of the same packet** (compression must not destabilize the cached prefix — keep compressed bodies *after* the cached prefix region). No rebuild of caching logic; just verify + guard.
2. **ADR-009** in `docs/adr/` documenting reversible context compression (why deterministic-only, why originals-on-disk, replay semantics). Brings the named-ADR set to six.
3. **Docs:** update `docs/context-packets.md` (compression + expand lifecycle), `CHANGELOG.md` Unreleased, and add a `v1.1` section stub to `V1.0-ROADMAP.md` (or a new `V1.1-ROADMAP.md`).
4. **Do not** touch the v1.0 release machinery (tags, cargo-dist, Homebrew, Node packages). If a schema bump changes the SDK, regenerate it but keep it on this branch.

---

## Suggested order (linear, schema-first)

1. **C1** (struct↔schema reconcile) — smallest, unblocks the artifact-spill pattern you'll reuse. → green.
2. **A1** `mimir-compress` crate + unit tests (pure, isolated). → green.
3. **A2** schema + example + generated types + SDK regen. → `p0_schemas.rs` + drift check green.
4. **A3/A4** builder wiring + policy knob + determinism test + cap gate. → green.
5. **B1** `mimir context expand` + tests. → green.
6. **C2** TestCard summarization. → green.
7. **D** eval tier (schema + harness + seed fixtures). → provider-free tests green; manual key run captured.
8. **E** cache-stability test, ADR-009, docs, CHANGELOG.
9. **B2** model `retrieve` tool — only if time remains; otherwise documented-as-planned.
10. Full gate: `cargo test --workspace --all-targets` + `./scripts/validate-production.sh` + SDK regen/drift/build + `check-gateway-boundary.sh`.

---

## Review checklist (what Opus will verify on return)

- [ ] `packet_hash` is stable across two builds of the same input with compression on (determinism preserved).
- [ ] No ML / network / RNG / clock in the packet path; `mimir-compress` is pure.
- [ ] Originals are preserved on disk and `mimir context expand` round-trips, hash-verified, fail-closed on mismatch.
- [ ] Only `mimir-providers` does HTTP; only `mimir-runs` writes `.mimir/runs/`; gateway-boundary script green.
- [ ] Schema-first respected: every artifact-shape change went schema → example → generated → SDK; `p0_schemas.rs` + drift check green.
- [ ] `cap_compliance.rs` still green; compression reduced omissions on at least one fixture (show before/after).
- [ ] `ToolResultCard` struct now satisfies its schema; TestCard summaries are failure-focused + deterministic.
- [ ] `mimir eval answer` is fully opt-in and invisible to CI; verbatim-vs-compressed deltas captured in the PR description.
- [ ] 531 prior tests still pass; new tests added per workstream; fmt + workspace clippy `-D warnings` clean; `validate-production.sh` green.
- [ ] v1.0 release machinery untouched.

## Explicitly out of scope / do NOT do

- ML/learned compression (Kompress-base) — rejected on determinism grounds.
- A general agentic tool-call loop beyond the bounded `retrieve` stretch.
- Headroom-style proxy/middleware/MCP packaging (Mimir is a standalone CLI for now — post-1.1 strategic question).
- The `v1.0.0` GitHub tag / CI / release-asset work — that's a separate, access-gated task.
