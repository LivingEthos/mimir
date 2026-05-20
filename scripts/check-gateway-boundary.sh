#!/usr/bin/env bash
set -euo pipefail

echo "Checking gateway boundary..."

# Only mimir-providers may import reqwest
for crate in crates/*/; do
    name=$(basename "$crate")
    if [ "$name" = "mimir-providers" ]; then
        continue
    fi
    if matches=$(grep -R -n -E '^[[:space:]]*(use|extern[[:space:]]+crate)[[:space:]]+reqwest([[:space:];:{]|$)' "$crate/src" 2>/dev/null); then
        echo "$matches"
        echo "ERROR: $name imports reqwest (only mimir-providers may do this)"
        exit 1
    fi
done

# Non-provider crates must not invoke provider adapter dispatch directly.
for crate in crates/*/; do
    name=$(basename "$crate")
    if [ "$name" = "mimir-providers" ]; then
        continue
    fi
    if matches=$(grep -R -n -E '\.call[[:space:]]*\(' "$crate/src" "$crate/tests" 2>/dev/null); then
        echo "$matches"
        echo "ERROR: $name invokes .call() directly; provider dispatch must go through ProviderGateway"
        exit 1
    fi
done

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/mimir-gateway-boundary.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT
cat >"$tmp_dir/Cargo.toml" <<EOF
[package]
name = "mimir-gateway-boundary-check"
version = "0.0.0"
edition = "2021"

[dependencies]
mimir-providers = { path = "$PWD/crates/mimir-providers" }
EOF
mkdir -p "$tmp_dir/src"
cat >"$tmp_dir/src/main.rs" <<'EOF'
use mimir_providers::{
    OpenAiCompatibleAdapter, ProviderAdapter, ProviderMessage, ProviderRequest,
};

fn main() {
    let adapter =
        OpenAiCompatibleAdapter::generic_from_env("openai-compatible", "boundary-model").unwrap();
    let request = ProviderRequest {
        model: "boundary-model".to_string(),
        system: None,
        messages: vec![ProviderMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        }],
        tools: None,
        max_tokens: Some(1),
        temperature: None,
        stream: None,
        stop_sequences: None,
        extra: None,
    };
    let _future = adapter.call(request);
}
EOF
if cargo check --manifest-path "$tmp_dir/Cargo.toml" --quiet >"$tmp_dir/stdout" 2>"$tmp_dir/stderr"; then
    echo "ERROR: external crate can directly call provider adapter .call()"
    exit 1
fi
if ! grep -q -E 'no method named `call`|method not found' "$tmp_dir/stderr"; then
    cat "$tmp_dir/stderr"
    echo "ERROR: external adapter .call() check failed for an unexpected reason"
    exit 1
fi

# Only mimir-runs may write under .mimir/runs/
write_api_pattern='(fs::write|std::fs::write|File::create|std::fs::File::create|OpenOptions|create_dir|remove_file|remove_dir|rename|copy)'
for crate in crates/*/; do
    name=$(basename "$crate")
    if [ "$name" = "mimir-runs" ]; then
        continue
    fi
    if matches=$(grep -R -n -E '\.mimir/runs' "$crate/src" 2>/dev/null \
        | grep -v -E '^[^:]+:[0-9]+:[[:space:]]*(//|//!|///)' \
        | grep -E "$write_api_pattern"); then
        echo "$matches"
        echo "ERROR: $name writes under .mimir/runs (only mimir-runs may do this)"
        exit 1
    fi
done

echo "Gateway boundary OK"
