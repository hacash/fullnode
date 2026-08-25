#!/usr/bin/env bash
# Build the native Ubuntu release binaries and the distributable WASM SDK.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

APP_FEATURES=(--no-default-features --features db-leveldb-sys)
BINARIES=(fullnode poworker diaworker fitshc)

echo "[release] building native binaries with db-leveldb-sys"
for binary in "${BINARIES[@]}"; do
    cargo build --release -p app --bin "$binary" "${APP_FEATURES[@]}"
    cp "target/release/$binary" "hacash_${binary}_ubuntu"
    echo "[release] published hacash_${binary}_ubuntu"
done

echo "[release] building static musl fullnode with db-sled"
cargo build --release -p app --bin fullnode \
    --no-default-features --features db-sled \
    --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/fullnode hacash_fullnode_ubuntu_16.04
echo "[release] published hacash_fullnode_ubuntu_16.04"

echo "[release] building SDK distribution"
# SDK's execute-off graph has known, intentionally retained Rust warnings.
# Suppress them for this packaging command only; preserve any caller flags.
SDK_RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-Awarnings"
RUSTFLAGS="$SDK_RUSTFLAGS" ./sdk/pack.sh --release

(cd sdk && zip -q -FSr ../hacash_wasm_sdk.zip README.md dist)
echo "[release] published hacash_wasm_sdk.zip"
