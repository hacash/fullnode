
## Settings
SDKNAME=hacashsdk
LIBNAME=wasmsdk
TARGET=wasm32-unknown-unknown
BINARY=target/$TARGET/release/$LIBNAME.wasm

## Build WASM
RUSTFLAGS="$RUSTFLAGS -A dead_code -A unused_imports -A unused_variables" \
cargo build --target $TARGET --release --lib

## Reduce size (remove panic exception handling, etc.)
wasm-snip --snip-rust-fmt-code \
          --snip-rust-panicking-code \
          -o $BINARY $BINARY

## Reduce size (remove all debugging information)
wasm-strip $BINARY

## Further reduce size
mkdir -p dist
wasm-opt -o dist/$SDKNAME.wasm -Oz $BINARY
