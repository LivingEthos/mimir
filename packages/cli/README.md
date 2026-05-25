# @mimir/cli

Private Node wrapper for local Mimir CLI package smoke tests. It installs the
platform-native binary via optional dependencies when packed and installed from
the local workspace.

## Install

```bash
cargo install mimir-cli
```

For release builds, prefer the GitHub release archives or Homebrew formula. This
package is intentionally private and is not published to the npm registry.

## Usage

```bash
mimir --help
mimir init
mimir plan --editable src/session.ts "Plan the session refresh fix"
mimir code --editable src/session.ts --dry-run "Fix the session refresh bug"
mimir code --editable src/session.ts "Fix the session refresh bug"
mimir packet share <run-id> --output shared-packet.json
mimir packet replay shared-packet.json --request-json
mimir eval context --dataset fixtures/context-recall-v1.yaml
```

`mimir code` refuses to run without `--editable` paths. Provider credentials are read from environment variables such as `GLM_API_KEY`, `ZAI_API_KEY`, or `OPENAI_API_KEY`; do not put keys in prompts or files.

Packet sharing writes a portable redacted replay bundle by default. Use `mimir packet share <run-id> --packet-only` only when you need the metadata-only `ContextPacket` JSON.

## Platforms

- macOS (ARM64, x86_64)
- Linux (ARM64, x86_64)
- Windows (x86_64)

If your platform is not supported, install from source:
```bash
cargo install mimir-cli
```

Release maintainers must stage native binaries into the platform packages before
running pack smoke tests. Empty platform packages fail `npm pack` through their
`prepack` guard.
