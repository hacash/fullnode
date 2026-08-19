#!/usr/bin/env bash
# Unified SDK packaging — the single build entry for the JS SDK.
#
#   ./sdk/pack.sh            # readable JS, all artifacts under sdk/dist/
#   ./sdk/pack.sh --release  # minified JS (esbuild → terser → npx esbuild;
#                            # skips with a warning when no minifier exists)
#
# One command covers the whole pipeline: regenerate the TS/JS codec from the
# Rust action schemas (sdk/js/generated is a build product, never committed),
# build the three wasm targets (nodejs / web / no-modules → base64 page),
# assemble dist/, and optionally minify the shipped JS.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$SCRIPT_DIR/dist"
JS_DIR="$SCRIPT_DIR/js"

RELEASE=0
case "${1:-}" in
    "") ;;
    --release|-r) RELEASE=1 ;;
    *) echo "usage: pack.sh [--release|-r]" >&2; exit 2 ;;
esac

# 1. Regenerate the codec from the Rust schemas so the shipped copy can never
#    drift from the Rust side, then regenerate the action-spec adapter,
#    op/error tables and golden vectors from the Rust single sources.
(cd "$WORKSPACE" && cargo run -q -p codec-schema-gen)
(cd "$WORKSPACE" && cargo run -q -p sdk --bin sdk_codegen)
SCHEMA_HASH="$(sed -n 's/^export const SCHEMA_HASH = "\([0-9a-f]*\)";/\1/p' "$JS_DIR/generated/codec.ts")"
echo "[pack] codec regenerated (schema hash $SCHEMA_HASH)"

# 2. wasm targets → dist/{nodejs,web,page}
"$SCRIPT_DIR/build.sh" nodejs
mkdir -p "$DIST_DIR/nodejs"
mv "$DIST_DIR/hacashsdk.js" "$DIST_DIR/nodejs/"
mv "$DIST_DIR/hacashsdk_bg.wasm" "$DIST_DIR/nodejs/"
if [ -f "$DIST_DIR/hacashsdk.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk.d.ts" "$DIST_DIR/nodejs/"
fi
if [ -f "$DIST_DIR/hacashsdk_bg.wasm.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk_bg.wasm.d.ts" "$DIST_DIR/nodejs/"
fi

"$SCRIPT_DIR/build.sh" web
mkdir -p "$DIST_DIR/web"
mv "$DIST_DIR/hacashsdk.js" "$DIST_DIR/web/"
mv "$DIST_DIR/hacashsdk_bg.wasm" "$DIST_DIR/web/"
if [ -f "$DIST_DIR/hacashsdk.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk.d.ts" "$DIST_DIR/web/"
fi
if [ -f "$DIST_DIR/hacashsdk_bg.wasm.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk_bg.wasm.d.ts" "$DIST_DIR/web/"
fi

"$SCRIPT_DIR/build.sh" no-modules
node "$SCRIPT_DIR/pack.js"
mkdir -p "$DIST_DIR/page"
mv "$DIST_DIR/hacashsdk_bg.js" "$DIST_DIR/page/"
if [ -f "$DIST_DIR/hacashsdk.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk.d.ts" "$DIST_DIR/page/"
fi
if [ -f "$DIST_DIR/hacashsdk_bg.wasm.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk_bg.wasm.d.ts" "$DIST_DIR/page/"
fi
cp "$SCRIPT_DIR/tests/friendly_test.html" "$DIST_DIR/page/friendly_test.html"

rm -f "$DIST_DIR"/*.js "$DIST_DIR"/*.wasm "$DIST_DIR"/*.d.ts

# 3. JS facade + codec. Recreate the target dir instead of copying into it:
#    cp -r into an existing dir nests (dist/js/generated/generated/…) and
#    leaves stale files behind.
rm -rf "$DIST_DIR/js"
mkdir -p "$DIST_DIR/js"
cp "$JS_DIR/hacashsdk.mjs" "$DIST_DIR/js/hacashsdk.mjs"
cp -r "$JS_DIR/generated" "$DIST_DIR/js/generated"

# 4. --release: minify the shipped JS (facade, codec, wasm-bindgen glue, page
#    bundle). Minifier resolution: $SDK_MINIFIER → esbuild → terser → npx
#    esbuild (on-demand download, cached after the first run).
MINIFY_CMD=""
if [ -n "${SDK_MINIFIER:-}" ] && command -v "$SDK_MINIFIER" >/dev/null 2>&1; then
    MINIFY_CMD="$SDK_MINIFIER"
elif command -v esbuild >/dev/null 2>&1; then
    MINIFY_CMD="esbuild"
elif command -v terser >/dev/null 2>&1; then
    MINIFY_CMD="terser"
elif timeout 30 npx --yes esbuild --version >/dev/null 2>&1; then
    MINIFY_CMD="npx esbuild"
fi

minify_js() {
    local file="$1" tmp="$1.tmp"
    case "$MINIFY_CMD" in
        esbuild|npx\ esbuild)
            if ! $MINIFY_CMD "$file" --minify --target=esnext --outfile="$tmp" >/dev/null 2>&1; then
                rm -f "$tmp"
                return 1
            fi ;;
        terser)
            local mod=""
            case "$file" in
                *.mjs|*/nodejs/*|*/web/*) mod="--module" ;;
            esac
            if ! terser "$file" -c -m $mod -o "$tmp" >/dev/null 2>&1; then
                rm -f "$tmp"
                return 1
            fi ;;
        *) return 1 ;;
    esac
    mv "$tmp" "$file"
}

if [ "$RELEASE" -eq 1 ]; then
    if [ -z "$MINIFY_CMD" ]; then
        echo "[pack] WARNING: --release given but no JS minifier found (esbuild/terser/npx); shipping readable JS"
    else
        echo "[pack] minifying JS with: $MINIFY_CMD"
        for f in \
            "$DIST_DIR/js/hacashsdk.mjs" \
            "$DIST_DIR/js/generated/codec.mjs" \
            "$DIST_DIR/nodejs/hacashsdk.js" \
            "$DIST_DIR/web/hacashsdk.js" \
            "$DIST_DIR/page/hacashsdk_bg.js"
        do
            if minify_js "$f"; then
                echo "[pack]   $(basename "$f"): $(wc -c < "$f" | tr -d ' ') bytes"
            else
                echo "[pack]   WARNING: minification failed for $(basename "$f"); keeping original"
            fi
        done
    fi
fi

echo "[pack] done — SDK artifacts under $DIST_DIR (entry: js/hacashsdk.mjs)"
