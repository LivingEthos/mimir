# Eval Fixtures

Generated from the Mimir workspace itself.

`context-recall-v1.yaml` is the production context-recall dataset used by:

```bash
mimir eval context --dataset fixtures/context-recall-v1.yaml
```

`answer-quality-v2.yaml` is the larger provider-answer fixture set used by:

```bash
mimir eval answer --dataset fixtures/answer-quality-v2.yaml
```

It points at small dedicated fixture repositories under `fixtures/repos/` so
answer-quality runs do not index the full Mimir workspace. Several of those
repos include large source files to exercise reversible compression in the
compressed arm.

The legacy `fixture-*.yaml` files remain as readable source cases; the dataset file
normalizes them into schema-shaped `EvalCase` entries.

| # | ID | Mode | Description |
|---|-----|------|-------------|
| 1 | mode0-simple-ask | 0 | Simple question about a single file |
| 2 | mode2-cross-crate-ref | 2 | Cross-crate reference requiring repo map |
| 3 | mode3-trace-replay | 3 | Replay a previous context build |
| 4 | mode4-subagent | 4 | Subagent execution with evidence |
| 5 | mode5-memory | 5 | Memory-augmented context |
| 6 | mode0-cap-compliance | 0 | Token cap enforcement |
| 7 | mode2-recall-guard | 2 | Recall guard for high-risk omissions |
| 8 | mode0-redactor | 0 | Secret redaction verification |
| 9 | mode2-provider-adapter | 2 | Provider adapter contract |
| 10 | mode4-edit-loop | 4 | Edit/test/repair loop |
| 11 | mode5-session-import | 5 | Session importer validation |
| 12 | mode0-server-transport | 0 | LSP server transport |
| 13 | mode2-telemetry | 2 | Telemetry event recording |
| 14 | mode3-packet-share | 3 | Packet sanitization and sharing |
| 15 | mode5-eval-harness | 5 | Eval harness with fixtures |
