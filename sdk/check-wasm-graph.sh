#!/usr/bin/env bash
# Verify the wasm32 dependency graph of the SDK stays inside the whitelist
# (plan 14 §1, §7): execution/consensus crates (x16rs/chain/db/node/...) and
# the big-integer engine must never enter the wasm graph, and `base` must be
# compiled WITHOUT the `execute` feature so execution modules/bodies never
# reach the wasm binary (`ActionRef`/`TxRef` are the wire view in every build;
# execute is a separate trait reached only when this feature is on).
#
# serde_json, dyn-clone, blake2, and tiny-keccak are execute-only on `vm`
# (IR/machine engine + native hashes). sha2 stays in the wasm graph via
# libsecp256k1, not vm. `serde` must not enter via `base`. Other crates may
# still carry serde. The codec path uses a local JSON string escape and
# sha3/ripemd only for P2SH address hashing.
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
# for wasm32 — no libc headers); num-bigint is only a test oracle for field's
# base-256 Amount core and must never link into the SDK.
FORBIDDEN=(
    x16rs x16rs-sys
    chain db node server api app mint
    num-bigint num-integer
    sled ocl
    serde_json
    dyn-clone
    blake2
    tiny-keccak
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
if grep -q 'protocol feature "execute"' <<< "$TREE"; then
    echo "[check-wasm-graph] FAILED: protocol is compiled with the execute feature in the wasm graph"
    FAILED=1
fi
if grep -q 'vm feature "execute"' <<< "$TREE"; then
    echo "[check-wasm-graph] FAILED: vm is compiled with the execute feature in the wasm graph"
    FAILED=1
fi

# `base` must not pull serde on the codec-only graph.
BASE_TREE="$(cd "$WORKSPACE" && cargo tree -p base --no-default-features --target "$TARGET" --edges normal --depth 1 2>/dev/null)"
if grep -qE '^├── serde v|^└── serde v' <<< "$BASE_TREE"; then
    echo "[check-wasm-graph] FAILED: base pulls serde without the execute feature"
    FAILED=1
fi

# Workspace crates the SDK production graph must keep (crate-owned catalogs).
EXPECTED=($(awk '
    /^\[dependencies\]/ { in_deps = 1; next }
    /^\[/ { in_deps = 0 }
    in_deps && /^[A-Za-z0-9_-]+[[:space:]]*=/ { print $1 }
' "$WORKSPACE/sdk/Cargo.toml"))
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
