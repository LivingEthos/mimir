# Mimir v1.0.0 Release Notes

Mimir 1.0.0 ships the local-first replayable context CLI: context packet construction, provider gateway validation, redacted provider request artifacts, safe plan/code flows, packet sharing/replay, memory import, live TUI/server refresh, SDK types, and a local context eval harness.

## Included

- Replayable `ContextPacket` artifacts with stable packet hashes.
- Provider gateway and capability registry for Anthropic and OpenAI-compatible providers.
- `ask`, `plan`, and `code` command paths that persist redacted run artifacts.
- Safe patch application with editable-set enforcement, dirty-target checks, bounded repair, and test output redaction.
- `mimir context suggest` for provider-free starting context, guidance files, likely files, and risky omissions.
- `mimir check` for source-controlled `.mimir/checks/*.md` validation, including CI JSON output.
- `mimir explore` for read-only subagent evidence persisted under `.mimir/runs/`.
- `mimir init` workflow seeds for `.mimir/project-rules.md`, starter checks, and validation recipes.
- Portable packet share/replay bundles.
- SQLite-backed memory import and publication tools.
- JSON-RPC/LSP server transport and TUI packet inspection.
- TypeScript SDK generated from the JSON schemas.
- `mimir eval context --dataset fixtures/context-recall-v1.yaml` for local recall/cap validation.

## Deliberately Out

- Multi-user hosted service behavior.
- Cloud synchronization of local run artifacts.
- npm registry publishing; Node packages in this repo are private pack-only smoke tests.
- Automatic Homebrew publishing from a local developer machine.
- Provider credentials stored in config files; credentials remain environment-only.

## Release Requirements

Before tagging, run `./scripts/validate-production.sh`, run the context eval dataset, stage and pack-check the private Node platform packages, replace Homebrew checksums, and confirm green CI on the release commit.
