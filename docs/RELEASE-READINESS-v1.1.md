# Release Readiness — v1.1 (Reversible Context Compression)

Status as of 2026-06-06. **Topology is now verified for PR prep.** Push, tag,
release, and Homebrew publication are still outward-facing steps and should stay
tied to green CI / release assets.

## Code readiness — GREEN

- Branch `v1.1/reversible-compression` (tip after F/G/H-groundwork/eval work,
  repair-patch hardening, and `origin/main` reconciliation).
- `./scripts/validate-production.sh` passes end-to-end on the final reconciled
  tip (fmt, `clippy --workspace -D warnings`, full test suite, doctests,
  release build, gateway-boundary, `cargo audit`, `cargo deny`, SDK
  generate/drift/build, CLI wrapper checks, and validate:examples).
- Release metadata is aligned to `v1.1.0` and canonical `LivingEthos/mimir`
  URLs (`crates/mimir-cli`, private npm package manifests, SDK metadata,
  Homebrew formula placeholders, and release scripts).
- RCC thesis validated on a real provider (MiniMax-M2.7): compression preserves
  answer quality while cutting input tokens ~54% — see
  [`eval-results-rcc-v1.1.md`](eval-results-rcc-v1.1.md).

## Git / remote topology — VERIFIED

| Ref | Commit | Notes |
|-----|--------|-------|
| `HEAD` (v1.1) | tip | 18 commits ahead / 0 behind `origin/main` after local reconciliation |
| local `main` | `6263004` | stale local branch, 0 ahead / 40 behind `origin/main`; do not use as release base |
| `origin/main` | `ab22d48` | public main; merge commit for `phase7/v1-exit-gates`; identical tree to `origin/phase7/v1-exit-gates` |
| tag `v1.0.0` | exists | published public GitHub release with cargo-dist assets on 2026-05-25 |
| `origin` | `https://github.com/LivingEthos/mimir.git` | canonical; `gh repo view MisterWonderful/mimir` currently resolves to `LivingEthos/mimir` |
| `gh` auth | `MisterWonderful` | active account; push protocol https |

**Decisions from current evidence:**

1. Canonical repo is `LivingEthos/mimir`.
2. `v1.0.0` is already public, so this release should ship as `v1.1.0`.
3. `origin/main` is authoritative. It has been merged into
   `v1.1/reversible-compression` locally; the remaining release path should use
   `origin/main` as the PR base.

## Recommended release path

1. Push `v1.1/reversible-compression` to `LivingEthos/mimir`.
2. Open a PR `v1.1/reversible-compression → main`: `gh pr create` — body below.
3. Green CI on the merge commit.
4. `git tag v1.1.0 <merge-commit>` and push the tag.
5. cargo-dist builds release artifacts; create the GitHub release; upload assets.
6. Update `HomebrewFormula` checksums to the live asset URLs; run the Homebrew
   install smoke check against the published URLs.
7. `@mimir/cli` npm publish remains disabled by policy (per handoff) unless that
   changed.

## Draft PR body

> **v1.1 — Reversible Context Compression + answer-quality eval**
>
> Adds deterministic, reversible context compression (CodeSkeleton + JsonCrush)
> so oversized candidates are compressed-and-included rather than omitted, with
> originals preserved on disk and retrievable via `mimir context expand`. Adds a
> provider-backed answer-quality eval tier and retrieve-tool groundwork.
>
> Validated on MiniMax-M2.7 (50-case set): compression preserves answer quality
> (delta within noise) while cutting provider-reported input tokens ~54%.
>
> Also hardens provider repair patch application: unified diffs are located by
> pre-image context instead of trusting loose line numbers, run-owned artifacts
> are checked before repair apply, and disabled Studio mode labels were removed.
>
> Full `validate-production.sh` is green on the final v1.1.0 release tip.
> Package and Homebrew metadata now target `LivingEthos/mimir` / `v1.1.0`;
> Homebrew checksums remain placeholders until cargo-dist release archives are
> staged. Deferred: live retrieve loop (needs multi-turn replay), tree-sitter
> skeletons. See
> `docs/HANDOFF-v1.1-followups.md` and `docs/eval-results-rcc-v1.1.md`.

## Not done yet

- No tag, no GitHub release, no Homebrew change yet. These wait on PR merge,
  green CI, and cargo-dist release assets.
