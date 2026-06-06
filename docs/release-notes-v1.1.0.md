# Mimir v1.1.0 Release Notes

Mimir 1.1.0 ships reversible context compression for context packets, answer-quality eval coverage, and stronger repair-loop safety around patch application.

Highlights:

- Adds deterministic, replayable context compression metadata with recall guards and prompt-stability coverage.
- Adds the answer-quality v2 eval fixture set and token-savings report path for compression validation.
- Hardens `mimir code` repair application by binding mutations to run-owned editable files and rejecting out-of-order unified diff hunks.
- Keeps Studio on the local-first session view while deferring the disabled mode switch until a supported endpoint is ready.
- Aligns v1.1 release metadata with the canonical `LivingEthos/mimir` repository and `v1.1.0` package versions.

Deferred follow-up work remains tracked in `docs/HANDOFF-v1.1-followups.md`, including bounded retrieve-loop refinement and optional tree-sitter compression improvements.
