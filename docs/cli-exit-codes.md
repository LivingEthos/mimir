# CLI Exit Codes

## Exit codes emitted today

The `mimir` binary currently emits only three exit codes. `main()` returns
`anyhow::Result<()>`, so a successful run yields `0` and any returned `Err`
yields `1`. A handful of error paths short-circuit with an explicit
`std::process::exit(1)`. Argument parsing is handled by clap, which exits the
process with `2` on usage/parse errors before `main` runs.

| Code | Meaning | How it is produced |
|------|---------|--------------------|
| 0 | Success | `main()` returns `Ok(())` |
| 1 | General error | Any `Err` propagated out of `main()` (anyhow), **or** an explicit `std::process::exit(1)` on a handled error path |
| 2 | Usage error | clap rejects arguments/flags (invalid value, missing required arg, unknown subcommand, `--help`/`--version` are also clap-driven) |

### Where the explicit `exit(1)` paths live

All explicit process exits in `crates/mimir-cli/src/main.rs` use code `1`:

- `mimir check --ci` when source-controlled checks report an error/critical finding (`!result.passed`)
- `mimir memory {list,show,why,search,publish,forget}` when the memory DB cannot be opened (`Memory DB not initialized`)
- `mimir packet share` (and `--packet-only`) when no packet exists for the run (`No packet found for run …`)
- `mimir trace export` when no trace can be read for the run (`No trace found for run …`)

Every other failure (config load, provider/network errors, schema/validation
failures, cap-over-limit rejections, secret-risk bails, etc.) surfaces as an
`anyhow` error returned from `main()` and therefore also exits with code `1` —
it is **not** mapped to a distinct numeric code.

## Reserved / planned scheme (NOT emitted today)

The table below is an **aspirational** exit-code scheme. The current binary
does **not** emit any of these codes; failures that would map here all exit `1`
today. Treat this as a design target for future refinement, not as current
behavior. Tooling that needs to distinguish failure classes today must parse
stderr/stdout, not the exit code.

| Code | Intended meaning |
|------|------------------|
| 3 | Config error (invalid or missing `.mimir/config.yaml`) |
| 4 | Provider error (provider API failure, authentication error) |
| 5 | Cap exceeded (packet exceeds configured token cap) |
| 6 | Network error (cannot reach provider or server) |
| 7 | File not found (file, packet, or run ID does not exist) |
| 8 | Permission denied (insufficient permissions for operation) |
| 9 | Validation error (schema validation failed) |
| 10 | Override required (operation requires cap override approval) |
| 11 | Review blocked (review found blocking issues) |
| 12 | Test failure (code execution produced failing tests) |
| 13 | Memory error (memory DB not initialized or corrupted) |
| 14 | Index error (repo index missing or stale) |
| 15 | Gateway boundary violation (direct provider import detected) |
| 16 | Prompt injection detected (potential prompt injection in repo content) |
| 64 | Usage error (sysexits `EX_USAGE`) |
| 70 | Internal software error (sysexits `EX_SOFTWARE`) |
| 77 | Permission denied (sysexits `EX_NOPERM`) |
| 126 | Command not executable |
| 127 | Command not found |

> Note: clap's usage-error code is `2`, which is what the binary actually
> emits — not the sysexits `64` shown in the reserved table above.

`mimir check --ci` currently exits with code `1` when source-controlled checks
report error or critical findings. A future refinement could map blocking check
failures to a dedicated code (e.g. `11`) once the broader review/check policy is
unified, but until then the binary emits only `0`, `1`, and `2`.
