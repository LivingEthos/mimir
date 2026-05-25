# ADR-005: MCP Server as Thin Shim

## Status
Accepted

## Context
MCP (Model Context Protocol) is emerging as a standard for tool/context exchange between AI systems. Mimir should be interoperable without becoming an MCP server itself.

## Decision
Ship a thin MCP shim in `mimir-server` that exposes a subset of SDK operations via JSON-RPC. The shim is stateless and delegates to the Rust core.

## Rationale
- Interoperability with MCP clients without full server implementation
- Thin shim means less attack surface and maintenance burden
- Can be disabled without affecting core workflow

## Consequences
Positive: MCP compatibility, minimal code, optional
Negative: Limited MCP feature set, may need expansion post-1.0
