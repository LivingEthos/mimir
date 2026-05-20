#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PARENT_ROOT=$(cd "$ROOT/.." && pwd)

run() {
    echo
    echo "==> $*"
    "$@"
}

run_node_check_if_present() {
    local path=$1
    if [ -f "$ROOT/$path" ]; then
        run node --check "$ROOT/$path"
    fi
}

cd "$ROOT"

run cargo fmt --all -- --check
run git diff --check
run cargo clippy --workspace --all-targets -- -D warnings
run cargo test --workspace --all-targets
run cargo test -p mimir-context -p mimir-providers -p mimir-schemas --doc
run cargo build --release
run "$ROOT/scripts/check-gateway-boundary.sh"
run cargo audit
run cargo deny check

if [ -f "$ROOT/packages/sdk/package.json" ]; then
    run npm --prefix "$ROOT/packages/sdk" run generate
    run npm --prefix "$ROOT/packages/sdk" run check:schema-drift
    run npm --prefix "$ROOT/packages/sdk" run build

    if [ -d "$ROOT/packages/sdk/scripts" ]; then
        while IFS= read -r script; do
            run node --check "$script"
        done < <(find "$ROOT/packages/sdk/scripts" -maxdepth 1 -type f -name '*.mjs' | sort)
    fi
fi

run_node_check_if_present "packages/cli/bin/mimir"
run_node_check_if_present "packages/cli/install.js"

if [ -f "$PARENT_ROOT/package.json" ] && [ -f "$PARENT_ROOT/scripts/validate-examples.mjs" ]; then
    run node --check "$PARENT_ROOT/scripts/validate-examples.mjs"
    run npm --prefix "$PARENT_ROOT" run validate:examples
else
    echo
    echo "==> skipping parent example validation; no parent package validator found"
fi

echo
echo "Production validation passed."
