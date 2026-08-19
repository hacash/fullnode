#!/usr/bin/env bash
# Regenerate the TS/JS codec and verify the on-disk copy in sdk/js/generated is
# exactly what the Rust schema produces (pack.sh regenerates it on every build,
# so a failure here means the checked-out copy was hand-edited or is stale).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE="$(cd "$SCRIPT_DIR/.." && pwd)"
GENERATED="$WORKSPACE/sdk/js/generated/codec.ts"
GENERATED_MJS="$WORKSPACE/sdk/js/generated/codec.mjs"

for f in "$GENERATED" "$GENERATED_MJS"; do
    if [ ! -f "$f" ]; then
        echo "[check-schema] missing $f — regenerate via 'cargo run -p codec-schema-gen' or './sdk/pack.sh'"
        exit 1
    fi
done

cp "$GENERATED" "$GENERATED.bak"
cp "$GENERATED_MJS" "$GENERATED_MJS.bak"
(cd "$WORKSPACE" && cargo run -q -p codec-schema-gen >/dev/null)
if ! cmp -s "$GENERATED" "$GENERATED.bak" || ! cmp -s "$GENERATED_MJS" "$GENERATED_MJS.bak"; then
    rm -f "$GENERATED.bak" "$GENERATED_MJS.bak"
    echo "[check-schema] FAILED: codec.ts/codec.mjs drifted from Rust schema. Regenerate and commit:"
    echo "  cargo run -p codec-schema-gen"
    exit 1
fi
rm -f "$GENERATED.bak" "$GENERATED_MJS.bak"
HASH="$(sed -n 's/^export const SCHEMA_HASH = "\([0-9a-f]*\)";/\1/p' "$GENERATED")"
echo "[check-schema] OK: codec.ts/codec.mjs match Rust schema (hash $HASH)"
