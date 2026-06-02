# ADR-008: Secret Redaction on Artifacts and Outbound Traffic

## Status
Accepted

## Context
Mimir persists provider requests, responses, events, and traces to `.mimir/runs/` and can export and share them. These artifacts routinely contain code, environment fragments, and provider payloads that may carry credentials. A single leaked secret in a shared replay bundle or trace export is a serious failure. Redaction must be centralized, pattern-driven, and applied uniformly wherever data leaves memory.

## Decision
A single redactor in `crates/mimir-security/src/redactor.rs` scrubs secrets from strings and JSON, and every artifact-writing path routes through it. Redaction is structural (sensitive JSON keys) as well as value-based (regex patterns), with explicit carve-outs so token-accounting fields survive.

1. **Pattern set.** `PATTERNS` is an array of exactly **19** regex patterns. By category: cloud-provider keys — `AWS_KEY`, `GCP_KEY`, `AZURE_SAS`; AI-provider keys — `ANTHROPIC_KEY`, `OPENAI_KEY`; SaaS/VCS tokens — `STRIPE_KEY`, `GITHUB_TOKEN`, `GITHUB_PAT`, `SLACK_TOKEN`; generic auth/JWT — `BEARER_TOKEN`, `JWT`; private keys — `PRIVATE_KEY`; environment-assignment forms — `ENV_KEY`, `ENV_SECRET`, `ENV_TOKEN`, `PASSWORD`, `PASSWD`, `API_KEY`; and connection strings — `DB_URL`. `redact_secrets` compiles them once via `OnceLock` and replaces each match with `<REDACTED:NAME>`. A data-driven corpus test (`test_redactor_corpus_covers_every_pattern`) asserts the corpus length equals `PATTERNS.len()`, keeping coverage 1:1.
2. **Sensitive JSON-key detection.** `redact_json_value` recurses into arrays and objects; for objects it consults `is_sensitive_key`, which lowercases and alphanumeric-normalizes the key and matches `api_key`/`apikey`, `secret`, `credential`, `token` (exact, or `_`/`-`/`.`-suffixed, or normalized-suffixed), `password`, `authorization`, `auth`, and normalized `cookie`-suffixed keys. A matched key has its value replaced with `<REDACTED:SECRET_FIELD>`; a key that itself looks like a secret is renamed to `<REDACTED:SECRET_KEY>` via `redact_json_key`.
3. **Token-accounting carve-outs.** `is_token_accounting_key` exempts `tokens`, `*_tokens`/`*-tokens`/`*.tokens`, and `token_count(s)`/`token_usage`/`token_budget`/`token_reserve` so usage and budget numbers (e.g. `max_tokens`, `input_tokens`, `output_reserve_tokens`) are never clobbered. `is_sensitive_key` also explicitly returns `false` for `credential_detected` so that detection flag is preserved.
4. **Application points.** Outbound provider requests are written redacted to `.mimir/runs/<id>/provider_request.redacted.json` by `write_provider_request_artifact` → `write_redacted_json_artifact` (which calls `redact_json_value`), on the `ask`, `plan`, `code`, and `context call` paths (`crates/mimir-cli/src/main.rs`). Provider responses, events (`append_redacted_event`), patch reports, plans, and override artifacts all flow through the same redacted writers. Provider adapters (`crates/mimir-providers/src/adapters/anthropic.rs`, `openai_compatible.rs`) redact before logging. `mimir trace export --redact` runs `redact_json_value` on each event and then `mimir_runs::redact_trace_paths` to strip absolute workspace paths from the export. Replay/share bundles (`crates/mimir-session/src/packet.rs`) reload the byte-identical redacted request and re-assert it is safe to share.

## Rationale
- One redactor, one `PATTERNS` array, one set of carve-outs means the rules cannot drift between call sites
- Combining value regexes with structural key detection catches secrets whether they appear inline in text or as a labeled JSON field
- The corpus test pins the pattern count and forces a sample per pattern, so adding a pattern without coverage fails CI
- Carving out token-accounting and `credential_detected` keys keeps cost/usage telemetry and detection flags intact, avoiding redaction that would break replay accounting

## Consequences
Positive: Every persisted, exported, or shared artifact is scrubbed by the same code; replay bundles and trace exports are safe to hand off; token accounting survives redaction
Negative: Regex-based detection is best-effort — a novel secret format outside the 19 patterns and not under a sensitive key can slip through; over-broad key matching can redact a benign field whose name merely contains `token`/`secret`/`auth`
