
### Prepare the compilation environment on Ubuntu:

```sh

## dependencies
rustup target add wasm32-unknown-unknown
cargo install wasm-snip
cargo install wasm-opt
cargo install wasm-pack
cargo install wasm-bindgen-cli
sudo apt install wabt

```


### Build WASM:

```sh

## build with wasm-bindgen --target nodejs or web or something else
./build.sh nodejs
# or 
./build.sh web


```

### Test in web browser:

```sh




./build.sh no-modules

# pack wasm code to one js file
node pack.js

cp ./tests/test.html ./dist/test.html

```

Open `./dist/test.html` in web browser and check devtools.