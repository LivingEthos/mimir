# CLI Exit Codes

| Code | Meaning | When It Occurs |
|------|---------|----------------|
| 0 | Success | Command completed normally |
| 1 | General error | Unhandled error, invalid input, or unexpected failure |
| 2 | Misuse of command | Invalid arguments, missing required flags |
| 3 | Config error | Invalid or missing `.mimir/config.yaml` |
| 4 | Provider error | Provider API failure, authentication error |
| 5 | Cap exceeded | Packet exceeds configured token cap |
| 6 | Network error | Cannot reach provider or server |
| 7 | File not found | Requested file, packet, or run ID does not exist |
| 8 | Permission denied | Insufficient permissions for operation |
| 9 | Validation error | Schema validation failed |
| 10 | Override required | Operation requires cap override approval |
| 11 | Review blocked | Review found blocking issues |
| 12 | Test failure | Code execution produced failing tests |
| 13 | Memory error | Memory DB not initialized or corrupted |
| 14 | Index error | Repo index missing or stale |
| 15 | Gateway boundary violation | Direct provider import detected |
| 16 | Prompt injection detected | Potential prompt injection in repo content |
| 64 | Usage error | CLI usage error (clap default) |
| 70 | Internal software error | Panic or unrecoverable internal error |
| 77 | Permission denied (OS) | OS-level permission denied |
| 126 | Command not executable | Binary permissions issue |
| 127 | Command not found | Required external tool not found |
