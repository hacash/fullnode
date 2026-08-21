#!/usr/bin/env bash
# Run the cheap architectural boundary checks used by the SDK/fullnode split.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$WORKSPACE"

echo "[check-architecture] checking codec-only parameter graph"
cargo check -p hacash-params -p protocol --no-default-features
cargo check -p mint-core --no-default-features

MINT_TREE="$(cargo tree -p mint-core --no-default-features --edges normal)"
for forbidden in protocol x16rs num-bigint; do
    if grep -qE "(^|[ /])${forbidden} v" <<< "$MINT_TREE"; then
        echo "[check-architecture] FAILED: mint-core wire graph contains $forbidden" >&2
        exit 1
    fi
done

MINT_EXEC_TREE="$(cargo tree -p mint-core --features execute --edges normal)"
if grep -qE "(^|[ /])protocol v" <<< "$MINT_EXEC_TREE"; then
    echo "[check-architecture] FAILED: mint-core execute graph contains protocol" >&2
    exit 1
fi

echo "[check-architecture] checking full composition and SDK"
cargo check -p app
cargo check -p sdk

echo "[check-architecture] checking SDK catalog selection"
cargo test -p sdk --lib codec::

APP_TREE="$(cargo tree -p app --edges normal)"
if grep -qE '(^|[ /])sdk v' <<< "$APP_TREE"; then
    echo "[check-architecture] FAILED: app production graph contains sdk" >&2
    exit 1
fi
SDK_TREE="$(cargo tree -p sdk --edges normal)"
for forbidden in app mint chain node server api; do
    if grep -qE "(^|[ /])${forbidden} v" <<< "$SDK_TREE"; then
        echo "[check-architecture] FAILED: SDK production graph contains $forbidden" >&2
        exit 1
    fi
done

echo "[check-architecture] checking the production graphs carry no serde"
# The JSON engine is the single hand-written implementation; serde_json may
# exist only as a dev-dependency test oracle, and serde only as a transitive
# third-party residue (libsecp256k1 / axum / reqwest / serde_urlencoded).
for pkg in field sys base mint app vm; do
    TREE="$(cargo tree -p "$pkg" --edges normal || true)"
    if grep -qE "(^| )serde_json v" <<< "$TREE"; then
        echo "[check-architecture] FAILED: $pkg production graph contains serde_json" >&2
        exit 1
    fi
    if grep -qE "(^| )serde v" <<< "$TREE"; then
        # serde may only enter through the allowed third-party residue
        # (libsecp256k1 / axum / reqwest / serde_urlencoded).
        INVERTED="$(cargo tree -p "$pkg" --edges normal -i serde || true)"
        if ! grep -qE "(libsecp256k1|axum|reqwest|serde_urlencoded) v" <<< "$INVERTED"; then
            echo "[check-architecture] FAILED: $pkg production graph pulls serde outside the allowed third-party residue" >&2
            exit 1
        fi
    fi
done

echo "[check-architecture] OK"
