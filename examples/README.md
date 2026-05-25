# Example Artifacts

Conforming sample artifacts for each schema. These are illustrative; the schemas in `../schemas/` are normative.

Hermes uses these to:

- Verify the schema validator works end-to-end.
- Bootstrap CLI golden-output tests.
- Have a reference for what a "real" packet/ledger/card looks like.

## Files

| File | Validates against |
|---|---|
| `context-packet.example.json` | `ContextPacket.schema.json` |
| `budget-ledger.example.json` | `BudgetLedger.schema.json` |
| `provider-capabilities.example.json` | `ProviderCapabilities.schema.json` |
| `provider-capabilities-list.example.json` | `ProviderCapabilitiesList.schema.json` |
| `context-candidate.example.json` | `ContextCandidate.schema.json` |
| `omitted-candidate.example.json` | `OmittedCandidate.schema.json` |
| `candidate-manifest.example.json` | `CandidateManifest.schema.json` |
| `context-plan.example.json` | `ContextPlan.schema.json` |
| `tool-result-card.example.json` | `ToolResultCard.schema.json` |
| `test-card.example.json` | `TestCard.schema.json` |
| `patch-plan.example.json` | `PatchPlan.schema.json` |
| `executable-patch-plan.example.json` | `ExecutablePatchPlan.schema.json` |
| `plan-artifact.example.json` | `PlanArtifact.schema.json` |
| `override-request.example.json` | `OverrideRequest.schema.json` |
| `eval-case.example.yaml` | `EvalCase.schema.json` (YAML form) |
| `audit-event.example.json` | `AuditEvent.schema.json` |
| `error.example.json` | `Error.schema.json` |
| `eval-result.example.json` | `EvalResult.schema.json` |
| `review-result.example.json` | `ReviewResult.schema.json` |
| `evidence-summary.example.json` | `EvidenceSummary.schema.json` |
| `memory-entry.example.json` | `MemoryEntry.schema.json` |
| `drift-report.example.json` | `DriftReport.schema.json` |
| `retry-policy.example.json` | `RetryPolicy.schema.json` |
| `trace-span.example.json` | `TraceSpan.schema.json` |

## Validation

`npm run validate:examples` runs `scripts/validate-examples.mjs`, which loads every schema into Ajv 2020 with `ajv-formats`, validates every example, and runs semantic consistency checks for budget math, editable targets, omitted-candidate rejection, error-code references, and TOML snippet duplicate keys. CI runs it on every PR.

`provider-capabilities.example.json` shows one `ProviderCapabilities` document, which is the same single-provider shape local YAML files must use. `provider-capabilities-list.example.json` shows the plural `ProviderCapabilitiesList` response returned by provider-list APIs.
