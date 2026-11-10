#!/usr/bin/env bash
# Build the native Ubuntu release binaries and the distributable WASM SDK.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "[release] building native binaries with db-leveldb-sys"
for binary in fullnode poworker diaworker; do
    features=(--no-default-features --features db-leveldb-sys)
    if [[ "$binary" == "fullnode" ]]; then
        features=(--no-default-features --features db-leveldb-sys,upgrade-guard)
    fi
    cargo build --release -p app --bin "$binary" "${features[@]}"
    cp "target/release/$binary" "hacash_${binary}_ubuntu"
    echo "[release] published hacash_${binary}_ubuntu"
done

# fitshc lives in its own package (no db/network deps), and its binary name
# differs from the package name, so it needs its own invocation.
cargo build --release -p fitshc-cli --bin fitshc
cp target/release/fitshc hacash_fitshc_ubuntu
echo "[release] published hacash_fitshc_ubuntu"

# db-sled keeps this target free of the C++ leveldb static library the host
# toolchain does not provide for x86_64-unknown-linux-musl.
echo "[release] building static musl fullnode with db-sled"
cargo build --release -p app --bin fullnode \
    --no-default-features --features db-sled,upgrade-guard \
    --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/fullnode hacash_fullnode_ubuntu_16.04
echo "[release] published hacash_fullnode_ubuntu_16.04"

echo "[release] building SDK distribution"
# SDK's execute-off graph has known, intentionally retained Rust warnings.
# Suppress them for this packaging command only; preserve any caller flags.
./sdk/pack.sh --release

# The zip is written from dist/ itself, so a stale top-level hacash_wasm_sdk.zip
# (possibly owned by another user from an earlier run) is replaced explicitly
# rather than left for zip to update in place.
rm -f hacash_wasm_sdk.zip
(cd sdk && zip -q -r ../hacash_wasm_sdk.zip README.md dist)
echo "[release] published hacash_wasm_sdk.zip"