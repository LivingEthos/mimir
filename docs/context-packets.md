# Context Packets

Mimir's context packet is the portable record of what the Context Governor selected for a run. It is hash-checked, capability-snapshot checked, and tied to a run id so later tools can prove they are using the same context.

## Build And Inspect

```bash
mimir context build
mimir context inspect .mimir/runs/<run-id>/context_packet.json
```

`context_packet.json` stores packet metadata, included paths, source hashes, omitted candidates, provider/model, and token estimates. It does not embed provider credentials.

Context packets include safe repository guidance files when present. `.mimir/project-rules.md`, `AGENTS.md`, and `CLAUDE.md` are treated as manifest references. Documentation-oriented tasks may also include `README.md` and `docs/HANDOFF.md`. Secret-like or oversized guidance files are omitted and recorded in `omitted_candidates`.

For task triage without a provider call:

```bash
mimir context suggest "fix server refresh"
```

`context suggest` writes a normal packet under `.mimir/runs/<run-id>/context_packet.json` and reports likely starting files, guidance files, risky omissions, and loaded source-controlled checks.

## Call A Provider

```bash
mimir context call .mimir/runs/<run-id>/context_packet.json
```

`context call`, `plan`, `code`, and `ask` write a redacted provider request artifact at:

```text
.mimir/runs/<run-id>/provider_request.redacted.json
```

That artifact is the canonical request replay uses when it exists.

## Share

```bash
mimir packet share <run-id> --output shared-packet.json
```

`packet share` writes a portable redacted replay bundle by default. The bundle contains:

- the schema-valid `ContextPacket`
- packet hash, provider, model, and run id metadata
- the redacted provider request used by the original run, when available
- SHA-256 checksums for the redacted request and user prompt

Before export, Mimir verifies the packet hash, run id, provider capability snapshot, and included-file source hashes. If packet metadata contains secret-like text, export fails instead of redacting hash-covered packet fields.

For metadata-only export:

```bash
mimir packet share <run-id> --packet-only --output context-packet.json
```

`--packet-only` is not self-contained. It requires an identical checkout for prompt reconstruction.

## Replay

```bash
mimir packet replay <run-id>
mimir packet replay <run-id> --request-json
mimir packet replay shared-packet.json
mimir packet replay shared-packet.json --request-json
```

Local run replay verifies the saved packet and included source hashes. Bundle replay works from a fresh directory because the redacted provider request is embedded in the bundle. `--request-json` prints the byte-identical redacted provider request JSON, suitable for audit diffs against `.mimir/runs/<run-id>/provider_request.redacted.json`.

Replay does not dispatch to a provider by itself. To make a new provider call from local artifacts, use:

```bash
mimir context call .mimir/runs/<run-id>/context_packet.json
```

## Common Failures

- `context packet hash mismatch`: packet fields changed after build.
- `packet run_id mismatch`: packet JSON does not belong to the run directory being used.
- `capability snapshot mismatch`: provider/model capability metadata changed.
- `source_hash mismatch`: an included source file changed since packet creation.
- `contains secret-like text`: packet metadata or a saved redacted request is not safe to share.
