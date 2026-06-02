# ADR-007: Override Auto-Grant After Repeated Failures

## Status
Accepted

## Context
Mimir enforces caps (notably the context token cap) that occasionally block legitimate work. A user who keeps hitting a cap needs an escape hatch, but a fully manual approval gate stalls automated workflows, and a fully automatic one defeats the cap. The system needs a bounded, auditable way to grant an above-default cap only after the user has genuinely been blocked, with every decision recorded.

## Decision
`mimir override request` records an override request, counts how many times the run already failed against a cap or safety boundary, and auto-grants the override once that count reaches the threshold. Every step writes a redacted audit event and a schema-validated artifact.

1. **Command surface.** `mimir override request --cap <tokens> --reason <text> [--auto-grant-after <N>] [--run-id <id>]` (`crates/mimir-cli/src/main.rs`, `OverrideCmd::Request`). `--auto-grant-after` defaults to `3`.
2. **Prior-failure counting.** When `--run-id` attaches to an existing run, `count_override_failed_attempts` scans that run's `events.jsonl` and counts lines whose `event_type` is in `OVERRIDE_FAILURE_EVENT_TYPES`: `cost_cap_aborted`, `repair_cost_cap_preflight_exceeded`, `patch_rejected`, `repair_patch_rejected`, `patch_tests_failed`, and `override_attempt_failed`. With no `--run-id`, a fresh run is created and prior failures are `0`.
3. **Grant decision.** The count drives `mimir_review::override_req::OverrideManager` (`crates/mimir-review/src/override_req.rs`), whose `default_threshold` is set from `--auto-grant-after`. `request_with_failures` auto-grants (`auto_granted = true`, `approved = Some(true)`) and appends an `OverrideAuditEntry` only when `prior_failures >= default_threshold`; otherwise the request stays pending (`approved = None`).
4. **Audit events.** An `override_requested` event is always appended to `events.jsonl` (carrying `requested_cap`, `reason`, `auto_grant_after`, `prior_failures`, `auto_granted`). On auto-grant, an `override_granted` event is also appended. Both go through `append_redacted_event`, which runs `mimir_security::redact_json_value` before write.
5. **Artifacts.** The request is persisted to `override_request.json`; on grant, an `OverrideGrant` is persisted to `override_grant.json` with `granted_by: "auto_after_failures"`, `granted_cap`, `prior_failures`, and `auto_grant_after`. `OverrideGrant` is a schema type (`crates/mimir-schemas/src/generated.rs`) backed by `schemas/OverrideGrant.schema.json`, whose `granted_by` is constrained to the enum `["auto_after_failures", "user", "policy"]` — an unknown value is rejected by the schema.

## Rationale
- Counting concrete failure events (not a self-reported tally) ties the auto-grant to evidence already in the run's append-only log, so the escape hatch only opens after the user was actually blocked
- A configurable, default-3 threshold balances "don't stall automation" against "don't trivially bypass the cap"
- Distinguishing `granted_by: auto_after_failures` from `user`/`policy` in a schema-validated artifact makes after-the-fact auditing unambiguous
- Routing every override event through the redactor keeps secrets out of the audit trail even when reasons or context echo sensitive text

## Consequences
Positive: Caps stay enforced by default but self-relax under demonstrated, logged pressure; every request and grant is auditable via `events.jsonl` and the `OverrideGrant` artifact; the grant provenance is machine-checkable
Negative: Auto-grant depends on the integrity of the failure-event log — a run that does not emit those event types never reaches the threshold; the threshold is per-invocation, not a global policy, so callers must pass a consistent `--auto-grant-after`
