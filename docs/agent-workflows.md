# Agent Workflows

Mimir productizes a context-first workflow for large codebases: discover the right starting point, preserve repository guidance, bound edits, run source-controlled checks, and keep evidence replayable.

## Initialize A Repo

```bash
mimir init
```

`mimir init` creates the standard `.mimir/` layout plus workflow files:

- `.mimir/project-rules.md` for durable repository guidance and published validated memory.
- `.mimir/checks/no-provider-secrets.md` as a starter source-controlled check.
- `.mimir/commands/fast-check.md` as a focused validation recipe.
- `.mimir/commands/release-check.md` as a full handoff validation recipe.

Existing files are preserved; init only writes missing files.

## Suggest Starting Context

```bash
mimir context suggest "fix server refresh" --json
```

`context suggest` builds and persists a context packet without a provider call. The JSON output includes the run id, packet path, guidance files, likely starting files, risky omissions, loaded check count, and next steps.

Use this before `mimir plan` or `mimir code` when the task is ambiguous.

## Repository Guidance

When present and safe to send, context packets include these guidance files with `reason_code=manifest_reference`:

- `.mimir/project-rules.md`
- `AGENTS.md`
- `CLAUDE.md`

For documentation, onboarding, handoff, or workflow tasks, packets may also include:

- `README.md`
- `docs/HANDOFF.md`

Guidance files are omitted rather than sent if they contain secret-like material or exceed guidance size limits. Omitted guidance is recorded in `omitted_candidates` with the reason.

## Source-Controlled Checks

```bash
mimir check
mimir check --ci --json
```

`mimir check` loads `.mimir/checks/*.md` and runs the current source-controlled checks without a provider call. `--ci` exits non-zero for error or critical findings. `--json` emits a machine-readable summary.

## Code Recipes

```bash
mimir code --editable src/lib.rs --recipe focused --param target=src/lib.rs "implement the change"
```

Code-mode recipes in `.mimir/commands/<name>.md` can parameterize the task sent to the provider. Mimir validates recipe frontmatter, rejects unsafe names, symlinks, undeclared parameters, unknown tools, unsupported modes, and secret-like content, then writes `.mimir/runs/<run-id>/command_recipe.json` with the rendered recipe before the provider call.

Recipes do not loosen edit permissions. `mimir code` still requires explicit `--editable` paths, and patch validation continues to fail closed outside that target set.

## Read-Only Exploration

```bash
mimir explore "where is packet replay handled?" --json
```

`explore` runs a read-only search subagent and writes `.mimir/runs/<run-id>/explore_evidence.json`. Use it to gather evidence before planning or editing.

## Editing Still Requires A Bound

The read-only commands do not grant write permission. Use explicit edit targets for mutations:

```bash
mimir plan --editable src/lib.rs "plan the change"
mimir code --editable src/lib.rs --dry-run "implement the change"
```

This keeps the large-codebase workflow aligned with Mimir's core contract: context first, explicit editable sets, replayable evidence, and fail-closed validation.
