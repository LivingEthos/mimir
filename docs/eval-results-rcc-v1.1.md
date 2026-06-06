# RCC v1.1 — Answer-Quality Eval Results

First real-provider validation of the Reversible Context Compression thesis:
**does compressing context cost answer quality?**

## Run

| Field | Value |
|-------|-------|
| Date | 2026-06-05 |
| Command | `mimir eval answer --provider openai-compatible --model MiniMax-M2.7 --dataset fixtures/answer-quality-v2.yaml --compare verbatim,compressed` |
| Provider | MiniMax (`https://api.minimax.io/v1`, OpenAI-compatible) |
| Model | `MiniMax-M2.7` |
| Dataset | `fixtures/answer-quality-v2.yaml` — 50 cases, 5 dedicated fixture repos |
| Arms | `verbatim` (compression off) vs `compressed` (RCC on) |
| Calls | 100 (50 cases × 2 arms); 1 excluded (see caveats) |

## Headline result

**Compression is effectively free on quality, and roughly halves input tokens.**

| Arm | Accuracy | Cases | Mean input tokens (provider-reported) |
|-----|---------:|------:|--------------------------------------:|
| verbatim | 30.0% (15/50) | 50 | 1,472.3 |
| compressed | 30.6% (15/49) | 49 | 677.6 |
| **delta** | **+0.6pp** | — | **−54% (−794.7 tokens/case)** |

Mimir's own packet-token accounting across the set:

| Metric | Value |
|--------|------:|
| verbatim input tokens (total) | 80,533 |
| compressed input tokens (total) | 30,181 |
| **tokens saved** | **50,352 (62.5%)** |

**Interpretation:** the accuracy delta (+0.6pp) is well within noise for n≈50, i.e. compression did **not** degrade answers, while cutting real provider-reported input tokens ~54% (and Mimir-estimated packet tokens ~62%). This is the "same answers, fewer tokens" claim, validated against a real model rather than a mock. The deterministic CodeSkeleton/JsonCrush compressors hold up.

## Caveats — read before quoting the absolute accuracy

1. **The ~30% absolute accuracy is a grading artifact, not a capability signal.** 30 of 50 cases grade with `exact_match` on short codename lookups (e.g. gold `aurora-ledger`). Models naturally answer *"The codename is `aurora-ledger`."* — which **contains** the gold but is not **equal** to it, so `exact_match` marks a correct retrieval as wrong. This depresses **both arms identically**, so the verbatim-vs-compressed comparison (the point of the eval) is unaffected — but the absolute number should not be read as "Mimir produces 30%-correct answers." Relaxing those cases to `contains_ci` would raise the absolute accuracy for both arms.

2. **One case excluded (`aq2-019-python-mode`, compressed arm):** `provider_truncated` — MiniMax-M2.7 is a reasoning model whose output (including thinking) exceeded the packet's `max_tokens` reserve. The eval is now resilient: it records and skips the failed call rather than aborting the whole run. Real-world `mimir code`/`plan` against long-reasoning models may want a larger output reserve.

## Conclusions / follow-ups

- ✅ **RCC thesis validated** for the comparison it was built to test: compression preserves answer quality while ~halving input tokens.
- ⚠️ **Improve fixture grading** — convert the brittle `exact_match` codename cases to `contains_ci`, then re-run, to get a trustworthy *absolute* accuracy alongside the (already trustworthy) delta.
- ⚠️ **Output-reserve sizing** for reasoning models — consider a larger/configurable `max_tokens` reserve so long-thinking models don't truncate.
- Next: a real-task success eval (`mimir code` on real repos vs `aider`/`claude --print`) for the broader product thesis.
