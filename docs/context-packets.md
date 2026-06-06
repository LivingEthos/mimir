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

## Compression and Expand Lifecycle

When a candidate's token count exceeds `compress_threshold_tokens` (default 2048), Mimir applies deterministic, rule-based compression instead of omitting it:

- **CodeSkeleton** (Rust/TypeScript/JavaScript/Python) keeps signatures, imports, and doc comments; elides body blocks.
- **JsonCrush** compacts homogeneous JSON arrays and truncates long strings.
- **None** — identity pass-through for unknown languages.

If compression achieves ≥25% reduction and the compressed form fits the remaining budget, the packet includes the compressed text and records metadata in `included[].compression`:

```json
{
  "algorithm": "code_skeleton",
  "original_tokens": 5200,
  "compressed_tokens": 1200,
  "original_hash": "e4f5a6...e4f5",
  "original_artifact_path": ".mimir/runs/<run-id>/artifacts/e4f5a6...e4f5.orig"
}
```

The original bytes are written to `.mimir/runs/<run-id>/artifacts/<hash>.orig` so they remain retrievable. `source_hash` is still the hash of the original (replay verification is unchanged).

To retrieve the verbatim original:

```bash
mimir context expand <run-id> <path>
mimir context expand <run-id> <source-hash>
```

`expand` verifies the on-disk bytes against the recorded hash and fails closed on mismatch. It respects `--json` for machine-readable output.

Plan and code requests can advertise an experimental `retrieve` tool schema with
`--enable-retrieve`. The live model-driven retrieval loop is intentionally still
deferred until replay supports multi-turn provider request artifacts; if a
provider returns a tool-use response today, Mimir records the response and fails
closed instead of silently ignoring it.

Compression can be disabled per-run for a fully-verbatim packet:

```bash
# Not yet exposed on the CLI; disable via policy in code/tests
TokenPolicy { compression_enabled: false, .. }
```

## Common Failures

- `context packet hash mismatch`: packet fields changed after build.
- `packet run_id mismatch`: packet JSON does not belong to the run directory being used.
- `capability snapshot mismatch`: provider/model capability metadata changed.
- `source_hash mismatch`: an included source file changed since packet creation.
- `contains secret-like text`: packet metadata or a saved redacted request is not safe to share.
