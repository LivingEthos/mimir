# Phase 2 Implementation Plan

## Goal
Implement repo map generation, ranked retrieval, and recall guard per 09-RETRIEVAL-PIPELINE.md and 15-PHASES.md.

## Architecture

### mimir-index (Repo Map Generation)
- File tree walker with gitignore support
- Language detection by extension + shebang + content sniff
- Import/export extraction for Rust, TS/JS, Python (regex-based for P2, tree-sitter in P5)
- Content hash for incremental indexing
- Index cache keyed by content-hash + repo-hash

### mimir-retrieval (Ranked Retrieval)
- Stage 1: High-recall cheap scan (filename match, ripgrep on symbols)
- Stage 2: Structural expansion (import closure, callers/callees)
- Stage 4: Candidate manifest emission with feature vectors
- Stage 5: Greedy budget packer
- Stage 6: Sufficiency check
- Stage 7: Recall guard

### mimir-context (Recall Guard + Integration)
- Recall guard: flag high-risk omissions
- `mimir context why <path> <run_id>` command
- Integration with builder for full pipeline

## Exit Gates (from 15-PHASES.md)
- Indexing 10k-file repo <30s cold, <2s incremental
- Modes 2/3 outperform mode 1 on recall metrics
- Regression case catches known indirect dependency omission
- `mimir context why` correctly cites reason code
- Cap sweep produces table for 16k/32k/64k/96k/128k
- All eval cases have cap_compliance: 100%

## Implementation Order
1. mimir-index: file tree + language detection + import extraction
2. mimir-retrieval: pipeline stages + ranking + packing
3. mimir-context: recall guard + why command
4. Tests for all crates
5. CLI integration
6. Multi-model review
