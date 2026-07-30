#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST_DIR="$SCRIPT_DIR/dist"
JS_DIR="$SCRIPT_DIR/js"

"$SCRIPT_DIR/build.sh" nodejs
mkdir -p "$DIST_DIR/nodejs"
mv "$DIST_DIR/hacashsdk.js" "$DIST_DIR/nodejs"
mv "$DIST_DIR/hacashsdk_bg.wasm" "$DIST_DIR/nodejs"
if [ -f "$DIST_DIR/hacashsdk.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk.d.ts" "$DIST_DIR/nodejs"
fi
if [ -f "$DIST_DIR/hacashsdk_bg.wasm.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk_bg.wasm.d.ts" "$DIST_DIR/nodejs"
fi

"$SCRIPT_DIR/build.sh" web
mkdir -p "$DIST_DIR/web"
mv "$DIST_DIR/hacashsdk.js" "$DIST_DIR/web"
mv "$DIST_DIR/hacashsdk_bg.wasm" "$DIST_DIR/web"
if [ -f "$DIST_DIR/hacashsdk.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk.d.ts" "$DIST_DIR/web"
fi
if [ -f "$DIST_DIR/hacashsdk_bg.wasm.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk_bg.wasm.d.ts" "$DIST_DIR/web"
fi

"$SCRIPT_DIR/build.sh" no-modules
node "$SCRIPT_DIR/pack.js"
mkdir -p "$DIST_DIR/page"
mv "$DIST_DIR/hacashsdk_bg.js" "$DIST_DIR/page"
if [ -f "$DIST_DIR/hacashsdk.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk.d.ts" "$DIST_DIR/page"
fi
if [ -f "$DIST_DIR/hacashsdk_bg.wasm.d.ts" ]; then
    mv "$DIST_DIR/hacashsdk_bg.wasm.d.ts" "$DIST_DIR/page"
fi
cp "$SCRIPT_DIR/tests/friendly_test.html" "$DIST_DIR/page/friendly_test.html"

rm -f "$DIST_DIR"/*.js "$DIST_DIR"/*.wasm

mkdir -p "$DIST_DIR/js"
cp "$JS_DIR/hacashsdk.mjs" "$DIST_DIR/js/hacashsdk.mjs"
cp "$JS_DIR/hacashsdk.cjs" "$DIST_DIR/js/hacashsdk.cjs"
cp "$JS_DIR/hacashsdk.global.js" "$DIST_DIR/js/hacashsdk.global.js"
