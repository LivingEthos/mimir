# @mimir/cli

NPM wrapper for the Mimir CLI. Installs the platform-native binary via optional dependencies.

## Install

```bash
npm install -g @mimir/cli
```

## Usage

```bash
mimir --help
mimir init
mimir ask "How does session refresh work?"
mimir code "Fix the session refresh bug"
```

## Platforms

- macOS (ARM64, x86_64)
- Linux (ARM64, x86_64)
- Windows (x86_64)

If your platform is not supported, install from source:
```bash
cargo install mimir-cli
```
