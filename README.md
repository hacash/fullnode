# fullnode
Hacash Fullnode Software and SDK



Ubuntu:

```sh


sudo apt install -y openssl libssl-dev libudev-dev cmake llvm clang musl-tools build-essential
sudo ln -s /usr/bin/g++ /usr/bin/musl-g++
 
# rustup target add x86_64-pc-windows-gnu
rustup target add x86_64-unknown-linux-musl

cargo build --target x86_64-unknown-linux-musl

# or
RUSTFLAGS="-C target-feature=-crt-static" RUST_BACKTRACE="full" cargo build --release
cp target/release/fullnode  ./hacash_fullnode_ubuntu
cp target/release/poworker  ./hacash_poworker_ubuntu
cp target/release/diaworker ./hacash_diaworker_ubuntu

# or static 
RUSTFLAGS="-C target-feature=+crt-static" RUST_BACKTRACE="full" cargo build --release --target=x86_64-unknown-linux-musl

```

Windows:

```powershell


rustup target add x86_64-pc-windows-gnu

set RUSTFLAGS='-C target-feature=+crt-static'; set RUST_BACKTRACE='full'; cargo build --release --target x86_64-pc-windows-gnu;

cp target/x86_64-pc-windows-gnu/release/fullnode.exe  ./hacash_fullnode_windows.exe
cp target/x86_64-pc-windows-gnu/release/poworker.exe  ./hacash_poworker_windows.exe
cp target/x86_64-pc-windows-gnu/release/diaworker.exe ./hacash_diaworker_windows.exe


```
