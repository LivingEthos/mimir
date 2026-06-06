# Release Readiness — v1.1 (Reversible Context Compression)

Status as of 2026-06-05. **This is an assessment + checklist. The actual
push/tag/release is intentionally NOT automated** — it is outward-facing and
hard to reverse, and the git/remote state below needs a human decision first.

## Code readiness — GREEN

- Branch `v1.1/reversible-compression` (tip after F/G/H-groundwork/eval work).
- `./scripts/validate-production.sh` passes end-to-end (fmt, `clippy --workspace
  -D warnings`, full test suite, doctests, gateway-boundary, `cargo audit`,
  `cargo deny`, SDK generate/drift/build, validate:examples).
- v1.0 release machinery untouched by v1.1 (`dist-workspace.toml`,
  `HomebrewFormula/`, `packages/cli/` unchanged).
- RCC thesis validated on a real provider (MiniMax-M2.7): compression preserves
  answer quality while cutting input tokens ~54% — see
  [`eval-results-rcc-v1.1.md`](eval-results-rcc-v1.1.md).

## Git / remote topology — NEEDS A DECISION

| Ref | Commit | Notes |
|-----|--------|-------|
| `HEAD` (v1.1) | tip | 50+ commits of v1.1 work |
| local `main` | `6263004` | **ancestor of HEAD** — v1.1 → local main is a clean fast-forward |
| `origin/main` | `ab22d48` | **diverged** from the v1.1 line at baseline `6a49148` |
| tag `v1.0.0` | exists | already present locally (+ "LivingEthos public release" prep commits on local main) |
| `origin` | `https://github.com/LivingEthos/mimir.git` | but earlier handoff docs call `MisterWonderful/mimir` canonical |
| `gh` auth | `MisterWonderful` | active account; push protocol https |

**Blocking questions (answer before any push/tag):**
1. **Which repo is canonical** — `LivingEthos/mimir` (current `origin`) or
   `MisterWonderful/mimir` (per handoff docs)? They disagree.
2. **Is `v1.0.0` already publicly released?** A `v1.0.0` tag and release-prep
   commits already exist. If so, **v1.1 should ship as `v1.1.0`, not a re-tag of
   1.0.0.**
3. **How to reconcile `origin/main`'s divergence** — `origin/main` (ab22d48) has
   commits not in v1.1, and v1.1 has 50 commits not in it. Merge v1.1 into it,
   rebase v1.1 onto it, or is `origin/main` stale and local `main` authoritative?

## Recommended release path (once the above are answered)

Assuming canonical repo confirmed, `origin/main` reconciled, and shipping as
`v1.1.0`:

1. `git fetch origin` and reconcile `main` with `origin/main` (merge or reset to
   the agreed authoritative state).
2. Open a PR `v1.1/reversible-compression → main` (or fast-forward if local main
   is authoritative): `gh pr create` — body below.
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
> Full `validate-production.sh` green; v1.0 release machinery untouched. Deferred:
> live retrieve loop (needs multi-turn replay), tree-sitter skeletons. See
> `docs/HANDOFF-v1.1-followups.md` and `docs/eval-results-rcc-v1.1.md`.

## Not done here (deliberately)

- No `git push`, no tag, no GitHub release, no Homebrew change. These wait on the
  three blocking questions and explicit go-ahead.
