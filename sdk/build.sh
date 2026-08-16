#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JSTARGET="${1:-nodejs}"
PROFILE="${WASM_PROFILE:-wasm-release}"

SDK_NAME="hacashsdk"
LIB_NAME="sdk"
RUST_TARGET="wasm32-unknown-unknown"
WASM_BINDGEN_VERSION="0.2.100"

if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    TARGET_DIR="$CARGO_TARGET_DIR"
else
    TARGET_DIR="$(cargo metadata --manifest-path "$SCRIPT_DIR/Cargo.toml" --format-version 1 --no-deps \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | head -n 1)"
fi

BINARY="$TARGET_DIR/$RUST_TARGET/$PROFILE/$LIB_NAME.wasm"
DIST_DIR="$SCRIPT_DIR/dist"

if ! rustup target list --installed | grep -q "^$RUST_TARGET$"; then
    rustup target add "$RUST_TARGET"
fi

if ! command -v wasm-bindgen >/dev/null 2>&1; then
    echo "wasm-bindgen CLI not found. Install with: cargo install -f wasm-bindgen-cli --version $WASM_BINDGEN_VERSION"
    exit 1
fi

WASM_BINDGEN_CLI_VERSION="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$WASM_BINDGEN_CLI_VERSION" != "$WASM_BINDGEN_VERSION" ]; then
    echo "wasm-bindgen CLI version mismatch: expected $WASM_BINDGEN_VERSION, got $WASM_BINDGEN_CLI_VERSION"
    echo "Install with: cargo install -f wasm-bindgen-cli --version $WASM_BINDGEN_VERSION"
    exit 1
fi

cargo build \
    --manifest-path "$SCRIPT_DIR/Cargo.toml" \
    --target "$RUST_TARGET" \
    --profile "$PROFILE" \
    --lib

mkdir -p "$DIST_DIR"
if [ ! -f "$BINARY" ]; then
    echo "build output not found: $BINARY"
    exit 1
fi

wasm-bindgen "$BINARY" \
    --out-name "$SDK_NAME" \
    --out-dir "$DIST_DIR" \
    --target "$JSTARGET" \
    --remove-name-section \
    --remove-producers-section

BG_WASM="$DIST_DIR/${SDK_NAME}_bg.wasm"
if command -v wasm-opt >/dev/null 2>&1; then
    TMP_WASM="$(mktemp)"
    # Explicit feature flags: recent rustc emits bulk-memory/sign-ext on
    # wasm32-unknown-unknown, which `--all-features` on older wasm-opt
    # versions miscompiles. Validate the output and fall back to the
    # (already valid) wasm-bindgen file when in doubt.
    if wasm-opt -Oz --enable-bulk-memory --enable-sign-ext --strip-debug --strip-dwarf \
            -o "$TMP_WASM" "$BG_WASM" \
        && (command -v wasm-validate >/dev/null 2>&1 && wasm-validate "$TMP_WASM" >/dev/null 2>&1); then
        mv "$TMP_WASM" "$BG_WASM"
    else
        echo "[SDK wasm] wasm-opt output failed validation; keeping unoptimized wasm"
        rm -f "$TMP_WASM"
    fi
fi

RAW_SIZE="$(wc -c < "$BG_WASM" | tr -d ' ')"
GZIP_SIZE="$(gzip -c "$BG_WASM" | wc -c | tr -d ' ')"
RAW_MB="$(awk -v bytes="$RAW_SIZE" 'BEGIN { printf "%.3f", bytes / 1024 / 1024 }')"
GZIP_MB="$(awk -v bytes="$GZIP_SIZE" 'BEGIN { printf "%.3f", bytes / 1024 / 1024 }')"
echo "[SDK wasm] target=$JSTARGET profile=$PROFILE raw=${RAW_MB}MB gzip=${GZIP_MB}MB"
