# ADR-006: Fail-Closed Editing

## Status
Accepted

## Context
Mimir applies model-proposed patches to a real working tree. A model can hallucinate paths, propose edits outside the area a user expected to change, or emit a patch that breaks the build. Silently applying any of these is unacceptable. The edit engine must refuse to act whenever a target is unsafe, out of scope, or uncertain, rather than guess.

## Decision
Editing fails closed at every stage. `mimir code` requires the user to name the editable area explicitly, and `mimir-edit` rejects any patch step whose target is not provably safe and in-scope before a single byte is written.

1. **Explicit editable set is mandatory.** `mimir code` aborts when no `--editable` path is supplied: `if options.editable.is_empty() { bail!("mimir code requires at least one --editable path so patch safety can be enforced") }` (`crates/mimir-cli/src/main.rs`). The allowed paths are carried as a `mimir_edit::EditableSet` (`crates/mimir-edit/src/lib.rs`).
2. **In-set enforcement.** `verify_editable_set` (lib.rs) walks `patch_step_paths` for every `PatchStep` variant (`LineRange`, `UnifiedDiff`, `WholeFile`, `Create`, `Delete`, `Move`) and returns `EditError::FileNotEditable` if any target is absent from the set. Inside the apply engine, `resolve_target` re-checks membership via `ensure_editable` (`crates/mimir-edit/src/apply.rs`) so the boundary is enforced again at write time.
3. **Path-escape rejection.** `normalize_patch_path` (apply.rs) refuses empty paths, Windows prefixes, backslashes, absolute paths, and any `ParentDir`/`RootDir`/`Prefix` component — `..` traversal cannot escape the base. `ensure_path_within_base` canonicalizes symlink targets and rejects any that resolve outside `canonical_base`; `ensure_no_symlink_components` refuses paths routed through a symlinked component. The CLI mirrors this with its own `safe_relative_path` / `ensure_no_symlink_components` guards (main.rs) that `bail!("file_not_editable: ...")` on unsafe or symlink-backed editable paths.
4. **Uncertain plans fail closed.** The code prompt instructs the model that "If no safe patch is possible, return patch_plan metadata without patch_recipe so Mimir fails closed." Plan validation `bail!`s when `editable_target_set` includes a path outside the requested set, or when a recipe step path is outside the plan's declared set (main.rs).
5. **Failed tests block success.** The bounded repair loop (`run_repair_loop`, `crates/mimir-edit/src/repair.rs`) only reports `converged: true` with `stop_reason "tests_passed"`. Exhausting `max_repair_turns` yields `converged: false` / `max_repair_turns_reached`, and exceeding `cost_cap_dollars` yields `cost_cap_exceeded` — never a silent pass. `WorktreeStatus` (`crates/mimir-edit/src/git.rs`) surfaces dirty files so edits over uncommitted work can be refused with `EditError::DirtyWorktree`.

## Rationale
- A bounded, explicit editable set turns "what can the model touch?" into a user-controlled invariant instead of a model guess
- Defense in depth: membership and path safety are checked both before apply (`verify_editable_set`) and at write time (`resolve_target`), so a bug in one layer does not open the boundary
- Symlink and `..` rejection close the obvious sandbox-escape vectors before any filesystem mutation
- Tests gate success, so a "fix" that breaks the build is reported as a failure with a structured stop reason, not applied and forgotten

## Consequences
Positive: No out-of-scope or path-escaping edit can be applied; unsafe or unverifiable patches are refused rather than guessed; every refusal carries a typed `EditError`
Negative: Users must enumerate `--editable` paths up front (more friction for broad changes); legitimate edits to symlinked or generated trees are blocked and need manual handling
