# Mimir JSON Schemas

These schemas are normative. The Rust and TypeScript types in `mimir-schemas` and `@mimir/sdk` are generated from these files.

## Conventions

- JSON Schema Draft 2020-12.
- `$id` is `https://mimir.dev/schemas/<Name>.schema.json`.
- Every schema includes `schema_version` as a `const`.
- Additive-only after Phase 0 freeze; breaking changes require an ADR and a bumped `schema_version`.
- All string enums use snake_case.
- All token and byte counts are unsigned integers.
- All durations are milliseconds (suffix `_ms`) or microseconds (`_us`) unless otherwise noted.
- All currency amounts are USD micros (`_micros`), where 1,000,000 = $1.00.
- All file paths are POSIX-style (`/`) even on Windows.
- All hashes are hex-encoded SHA-256 unless explicitly `blake3` or `sha512`.
- All timestamps are RFC 3339 with timezone.

## Files

| File | Schema title |
|---|---|
| `ContextPacket.schema.json` | ContextPacket |
| `BudgetLedger.schema.json` | BudgetLedger |
| `ProviderCapabilities.schema.json` | ProviderCapabilities |
| `ContextCandidate.schema.json` | ContextCandidate |
| `OmittedCandidate.schema.json` | OmittedCandidate |
| `CandidateManifest.schema.json` | CandidateManifest |
| `ContextPlan.schema.json` | ContextPlan |
| `ToolResultCard.schema.json` | ToolResultCard |
| `TestCard.schema.json` | TestCard |
| `PatchPlan.schema.json` | PatchPlan |
| `ReviewResult.schema.json` | ReviewResult |
| `EvidenceSummary.schema.json` | EvidenceSummary |
| `OverrideRequest.schema.json` | OverrideRequest |
| `MemoryEntry.schema.json` | MemoryEntry |
| `DriftReport.schema.json` | DriftReport |
| `EvalCase.schema.json` | EvalCase |
| `EvalResult.schema.json` | EvalResult |
| `Error.schema.json` | Error |
| `AuditEvent.schema.json` | AuditEvent |
| `TraceSpan.schema.json` | TraceSpan |
| `RetryPolicy.schema.json` | RetryPolicy |

## Validation

Validate examples and fixtures via:

```
npm run validate:examples
```

CI runs this on every PR.
