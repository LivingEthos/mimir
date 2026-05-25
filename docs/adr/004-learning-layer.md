# ADR-004: Learning Layer Scope

## Status
Accepted

## Context
Mimir should improve over time by remembering successful patterns, failures, and fixes. However, unverified "lessons" can pollute context and degrade performance.

## Decision
Implement a verify-before-learn pattern:
1. Proposed memory entries are recorded under `.mimir/runs/<run_id>/proposed_memory.json`
2. Entries require 3 independent successes before promotion to `verified`
3. Memory retrieval only uses `verified` entries by default
4. Project fingerprint prevents cross-project pollution

## Rationale
- Prevents hallucinated lessons from entering the context loop
- Audit trail for every promoted memory entry
- User can manually promote or demote entries

## Consequences
Positive: Reliable memory, no pollution, auditable
Negative: Slower learning curve, requires success tracking infrastructure
