# fullnode
Hacash Fullnode Software and SDK



Ubuntu:

```sh
sudo apt update
sudo apt install build-essential cmake musl-tools 

#s udo apt install -y openssl libssl-dev libudev-dev cmake llvm clang musl-tools build-essential
# sudo ln -s /usr/bin/g++ /usr/bin/musl-g++
 
# rustup target add x86_64-pc-windows-gnu

cargo build --target x86_64-unknown-linux-musl

# or
RUSTFLAGS="-C target-feature=-crt-static" RUST_BACKTRACE="full" cargo build --release
cp target/release/fullnode  ./hacash_fullnode_ubuntu
cp target/release/poworker  ./hacash_poworker_ubuntu
cp target/release/diaworker ./hacash_diaworker_ubuntu

# or static linked
# edit chain/Cargo.toml and protocol/Cargo.toml, change "db-leveldb-sys" to "db-sled"
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="-C target-feature=+crt-static" RUST_BACKTRACE="full" cargo build --release --target=x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/fullnode  ./hacash_fullnode_ubuntu_18.0
cp target/x86_64-unknown-linux-musl/release/poworker  ./hacash_poworker_ubuntu_18.0
cp target/x86_64-unknown-linux-musl/release/diaworker ./hacash_diaworker_ubuntu_18.0

```

Windows:

```powershell


rustup target add x86_64-pc-windows-gnu

set RUSTFLAGS='-C target-feature=+crt-static'; set RUST_BACKTRACE='full'; cargo build --release --target x86_64-pc-windows-gnu;

cp target/x86_64-pc-windows-gnu/release/fullnode.exe  ./hacash_fullnode_windows.exe
cp target/x86_64-pc-windows-gnu/release/poworker.exe  ./hacash_poworker_windows.exe
cp target/x86_64-pc-windows-gnu/release/diaworker.exe ./hacash_diaworker_windows.exe


```

MacOS:

```sh

               
RUSTFLAGS='-C target-feature=+crt-static' RUST_BACKTRACE='full' cargo build --release --target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/fullnode  ./hacash_fullnode_macos 
cp target/x86_64-apple-darwin/release/poworker  ./hacash_poworker_macos
cp target/x86_64-apple-darwin/release/diaworker ./hacash_diaworker_macos



```