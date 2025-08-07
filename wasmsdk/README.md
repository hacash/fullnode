
Prepare the compilation environment on Ubuntu:

```sh

## dependencies
rustup target add wasm32-unknown-unknown
cargo install wasm-snip
cargo install wasm-opt
cargo install wasm-pack
cargo install wasm-bindgen-cli
sudo apt install wabt

## build with wasm-bindgen --target nodejs or web or something else
./build.sh nodejs
# or 
./build.sh web


```
