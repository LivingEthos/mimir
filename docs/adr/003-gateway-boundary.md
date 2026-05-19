# ADR-003: Provider Gateway Boundary

## Status
Accepted

## Context
Only one crate should speak HTTP to AI providers. This prevents secret leakage, enables centralized retry/caching, and ensures cap compliance.

## Decision
All provider HTTP traffic flows through `mimir-providers`. No other crate may import HTTP clients or provider SDKs. A CI script (`cargo deny` + custom check) enforces this.

## Rationale
- Centralized secret redaction before outbound calls
- Single point for retry, backoff, and circuit breaker logic
- Token counting and cap enforcement at the boundary
- Audit logging of all provider interactions

## Consequences
Positive: Security, observability, compliance
Negative: All provider features must be added to gateway first, indirection overhead
