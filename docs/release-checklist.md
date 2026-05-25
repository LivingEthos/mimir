# Mimir Release Checklist

Use this checklist after `./scripts/validate-production.sh` passes on the release commit.

## Local Gates

```bash
./scripts/validate-production.sh
./target/release/mimir eval context --dataset fixtures/context-recall-v1.yaml
```

The eval command writes schema-valid `EvalResult` entries under `.mimir/evals/`.

## Distribution Staging

Current release stance: Mimir is source-visible under the Mimir Public Source
License, with the canonical repository at `LivingEthos/mimir`. cargo-dist
archives exist in `target/distrib/` for all five configured targets, all five
native binaries are staged into private Node platform packages, Homebrew
macOS/Linux checksums match the local archives, and all Node package pack
dry-runs pass. npm registry publication is disabled by policy.

1. Verify release metadata alignment before staging binaries:

```bash
node scripts/verify-platform-package.mjs --all
```

2. Run cargo-dist for the release targets, collect the five archives, and unpack the native binaries:
   - `aarch64-apple-darwin`
   - `x86_64-apple-darwin`
   - `aarch64-unknown-linux-gnu`
   - `x86_64-unknown-linux-gnu`
   - `x86_64-pc-windows-msvc`
3. Stage each unpacked binary into its private Node platform package. Do not pass a `.tar.xz` archive to `--binary`; the staging script checks for an unpacked native executable:

```bash
node scripts/stage-npm-platform-package.mjs --platform darwin-arm64 --binary path/to/mimir
node scripts/stage-npm-platform-package.mjs --platform darwin-x64 --binary path/to/mimir
node scripts/stage-npm-platform-package.mjs --platform linux-arm64 --binary path/to/mimir
node scripts/stage-npm-platform-package.mjs --platform linux-x64 --binary path/to/mimir
node scripts/stage-npm-platform-package.mjs --platform win32-x64 --binary path/to/mimir.exe
```

4. Dry-run every private Node package as a packaging smoke test:

```bash
(cd packages/cli-darwin-arm64 && npm pack --dry-run)
(cd packages/cli-darwin-x64 && npm pack --dry-run)
(cd packages/cli-linux-arm64 && npm pack --dry-run)
(cd packages/cli-linux-x64 && npm pack --dry-run)
(cd packages/cli-win32-x64 && npm pack --dry-run)
(cd packages/sdk && npm pack --dry-run)
(cd packages/cli && npm pack --dry-run)
```

The root `@mimir/cli` prepack runs `verify-platform-package.mjs --all --require-platform-binaries`, so it should fail until every native platform package has been staged.

5. Replace the placeholder SHA256 values in `HomebrewFormula/mimir.rb` from the final archives:

```bash
node scripts/update-homebrew-checksums.mjs --artifacts-dir target/distrib
```

For a local macOS-only artifact set, this partial verification is expected to
pass while still reporting missing Linux archives:

```bash
node scripts/update-homebrew-checksums.mjs --check --allow-missing --artifacts-dir target/distrib
```

6. Verify the final Homebrew checksums are real SHA-256 values and match the cargo-dist archives:

```bash
node scripts/update-homebrew-checksums.mjs --check --artifacts-dir target/distrib
node scripts/verify-platform-package.mjs --all --require-homebrew-sha256 --homebrew-artifacts-dir target/distrib
node scripts/verify-platform-package.mjs --all --require-platform-binaries
```

## External Release Runbook

Do not create release tags or GitHub releases from a dirty, unvalidated, or
read-only-authenticated workspace. Do not expose a public release/tag whose
source archive contains superseded permissive license metadata. npm registry
publication is not part of the release process; the packages under `packages/`
are private and are kept only for local pack/install smoke tests.

1. Confirm credentials and exact release state:

```bash
gh auth status
git remote -v
git status --short --branch
git rev-parse HEAD
git rev-parse v1.0.0 || true
git push --dry-run origin HEAD
gh release view v1.0.0 --repo LivingEthos/mimir || true
```

Proceed only when GitHub push dry-run succeeds for `LivingEthos/mimir`, the release
commit is clean, and CI is green for that exact commit. If a local or remote
`v1.0.0` tag already exists at another commit, stop and get explicit retagging
approval before deleting or moving it.

2. Re-run focused release checks immediately before release creation:

```bash
git diff --check
node --check scripts/stage-npm-platform-package.mjs
node --check scripts/update-homebrew-checksums.mjs
node --check scripts/verify-platform-package.mjs
node scripts/verify-platform-package.mjs --all --require-platform-binaries
node scripts/verify-platform-package.mjs --all --require-homebrew-sha256 --homebrew-artifacts-dir target/distrib
node scripts/update-homebrew-checksums.mjs --check --artifacts-dir target/distrib
```

3. Re-run private Node package pack smoke tests:

```bash
(cd packages/cli-darwin-arm64 && npm pack --dry-run)
(cd packages/cli-darwin-x64 && npm pack --dry-run)
(cd packages/cli-linux-arm64 && npm pack --dry-run)
(cd packages/cli-linux-x64 && npm pack --dry-run)
(cd packages/cli-win32-x64 && npm pack --dry-run)
(cd packages/sdk && npm pack --dry-run)
(cd packages/cli && npm pack --dry-run)
```

Do not run `npm publish` for these packages.

4. Push the validated commit and tag only from an account with write access:

```bash
git push origin HEAD
git tag -a v1.0.0 -m "Mimir v1.0.0"
git push origin v1.0.0
```

If `v1.0.0` already exists locally at the wrong commit, do not run the tag
commands until the release owner has approved the exact retagging plan.

5. Create the GitHub release from the pushed tag and upload the cargo-dist
archives plus checksum sidecars:

```bash
gh release create v1.0.0 \
  target/distrib/mimir-cli-aarch64-apple-darwin.tar.xz \
  target/distrib/mimir-cli-aarch64-apple-darwin.tar.xz.sha256 \
  target/distrib/mimir-cli-x86_64-apple-darwin.tar.xz \
  target/distrib/mimir-cli-x86_64-apple-darwin.tar.xz.sha256 \
  target/distrib/mimir-cli-aarch64-unknown-linux-gnu.tar.xz \
  target/distrib/mimir-cli-aarch64-unknown-linux-gnu.tar.xz.sha256 \
  target/distrib/mimir-cli-x86_64-unknown-linux-gnu.tar.xz \
  target/distrib/mimir-cli-x86_64-unknown-linux-gnu.tar.xz.sha256 \
  target/distrib/mimir-cli-x86_64-pc-windows-msvc.zip \
  target/distrib/mimir-cli-x86_64-pc-windows-msvc.zip.sha256 \
  --repo LivingEthos/mimir \
  --title "Mimir v1.0.0" \
  --notes-file docs/release-notes-v1.0.0.md
```

6. Verify public package and release visibility:

```bash
gh release view v1.0.0 --repo LivingEthos/mimir
```

7. Run Homebrew formula audit/install smoke tests only after the GitHub release
asset URLs are live. The formula URLs are expected to 404 before step 5.
