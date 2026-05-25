# ADR-001: Rust + TypeScript Tech Stack

## Status
Accepted

## Context
Mimir needs a high-performance, safe, and distributable CLI tool. The tool must handle large codebases, provider API calls, and local file system operations reliably.

## Decision
Use Rust for the core CLI and TypeScript for SDK/types generation.

## Rationale
- **Rust**: Memory safety without GC, excellent performance for file I/O and text processing, strong type system, cross-compilation for distribution
- **TypeScript**: Schema-first development, generated types from JSON schemas, npm distribution for JS ecosystem integration
- **tokio**: Async runtime for concurrent provider calls and file operations
- **clap**: Derive-based CLI parsing with comprehensive help generation

## Consequences
Positive: Fast cold start, safe concurrency, small binary size, type-safe schema evolution
Negative: Rust compile times, smaller contributor pool than Python/JS
