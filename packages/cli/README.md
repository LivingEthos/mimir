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
mimir plan --editable src/session.ts "Plan the session refresh fix"
mimir code --editable src/session.ts --dry-run "Fix the session refresh bug"
mimir code --editable src/session.ts "Fix the session refresh bug"
```

`mimir code` refuses to run without `--editable` paths. Provider credentials are read from environment variables such as `GLM_API_KEY`, `ZAI_API_KEY`, or `OPENAI_API_KEY`; do not put keys in prompts or files.

## Platforms

- macOS (ARM64, x86_64)
- Linux (ARM64, x86_64)
- Windows (x86_64)

If your platform is not supported, install from source:
```bash
cargo install mimir-cli
```
