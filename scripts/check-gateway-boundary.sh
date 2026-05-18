#!/usr/bin/env bash
set -euo pipefail

echo "Checking gateway boundary..."

# Only mimir-providers may import reqwest
for crate in crates/*/; do
    name=$(basename "$crate")
    if [ "$name" = "mimir-providers" ]; then
        continue
    fi
    if grep -r "use reqwest" "$crate/src" 2>/dev/null || grep -r "extern crate reqwest" "$crate/src" 2>/dev/null; then
        echo "ERROR: $name imports reqwest (only mimir-providers may do this)"
        exit 1
    fi
done

# Only mimir-runs may write under .mimir/runs/
for crate in crates/*/; do
    name=$(basename "$crate")
    if [ "$name" = "mimir-runs" ]; then
        continue
    fi
    if grep -r "\.mimir/runs" "$crate/src" 2>/dev/null; then
        echo "ERROR: $name references .mimir/runs (only mimir-runs may do this)"
        exit 1
    fi
done

echo "Gateway boundary OK"
