#!/usr/bin/env bash
# Verify the wasm32 dependency graph of the SDK stays inside the whitelist
# (plan 14 §1, §7): execution/consensus crates (x16rs/chain/db/node/...) and
# the big-integer engine must never enter the wasm graph, and `base` must be
# compiled WITHOUT the `execute` feature — the type-level ActionRef/TxRef cut
# is what guarantees execution code cannot reach the wasm binary.
#
# serde_json is deliberately NOT forbidden anymore: since the vm `full`/
# `codec-only` split was removed, the VM depends on serde_json unconditionally
# and it compiles into the wasm graph; nothing reachable from the wasm exports
# calls it, so wasm-ld dead-strips it from the artifact (the SDK uses field's
# handwritten JSON engine). The remaining graph checks below are the ones that
# still matter for binary correctness.
#
# Run from anywhere:  ./sdk/check-wasm-graph.sh
# Requires the wasm32-unknown-unknown target (rustup target add
# wasm32-unknown-unknown).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET="wasm32-unknown-unknown"

if ! rustup target list --installed 2>/dev/null | grep -q "^$TARGET$" \
    && [ ! -d "$(rustc --print sysroot)/lib/rustlib/$TARGET" ]; then
    echo "[check-wasm-graph] wasm32 target not installed — install with: rustup target add $TARGET"
    exit 1
fi

TREE="$(cd "$WORKSPACE" && cargo tree -p sdk --target "$TARGET" --edges normal -e features 2>/dev/null)"

# Crates that must never appear: execution/consensus state machines and the
# fullnode crates. x16rs is a hard compile constraint (its C code cannot build
# for wasm32 — no libc headers); num-bigint is the fullnode Amount engine that
# the SDK must never link (field uses the base-256 path instead).
FORBIDDEN=(
    x16rs x16rs-sys
    chain db node server api app mint
    num-bigint num-integer
    sled ocl
)

FAILED=0
for name in "${FORBIDDEN[@]}"; do
    if grep -qE "(^|[ /])${name} v|^├── ${name} v|^└── ${name} v" <<< "$TREE"; then
        echo "[check-wasm-graph] FAILED: forbidden crate in sdk wasm graph: $name"
        FAILED=1
    fi
done

# base must be compiled without the `execute` feature (the `-e features` tree
# lists "base feature \"execute\"" when it is enabled).
if grep -q 'base feature "execute"' <<< "$TREE"; then
    echo "[check-wasm-graph] FAILED: base is compiled with the execute feature in the wasm graph"
    FAILED=1
fi

# The expected codec surface is derived from chain-codec's own manifest
# dependencies (the crates its `register_standard` assembly calls into):
# adding a codec-hosting crate to chain-codec extends this list
# automatically instead of requiring a hand edit here. Guards against
# accidental feature flips that silently drop the codec set.
EXPECTED=($(awk '
    /^\[dependencies\]/ { in_deps = 1; next }
    /^\[/ { in_deps = 0 }
    in_deps && /^[A-Za-z0-9_-]+[[:space:]]*=/ { print $1 }
' "$WORKSPACE/chain-codec/Cargo.toml"))
for name in "${EXPECTED[@]}"; do
    if ! grep -qE "(^|[ /])${name} v" <<< "$TREE"; then
        echo "[check-wasm-graph] FAILED: expected crate missing from sdk wasm graph: $name"
        FAILED=1
    fi
done

if [ "$FAILED" -ne 0 ]; then
    echo "[check-wasm-graph] sdk wasm dependency graph violated the whitelist"
    exit 1
fi

echo "[check-wasm-graph] OK: sdk wasm graph is clean (base without execute, no execution/bigint crates)"
