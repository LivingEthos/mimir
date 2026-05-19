# ADR-002: JSON Schemas as Source of Truth

## Status
Accepted

## Context
Mimir exchanges structured data between Rust core, TypeScript SDK, provider APIs, and local artifacts. We need a single source of truth for all data contracts.

## Decision
Use JSON Schema (draft 2020-12) as the canonical contract. Generate Rust types via `schemars` and TypeScript types via `json-schema-to-typescript`.

## Rationale
- Language-agnostic contracts
- Validation at runtime and compile time
- Versioning via `schema_version` field
- Easy to share with external tools and documentation

## Consequences
Positive: Single source of truth, cross-language consistency, runtime validation
Negative: Schema evolution requires migration scripts, build-time generation complexity
