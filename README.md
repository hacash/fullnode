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
cp target/release/fullnode   ./hacash_fullnode_ubuntu
cp target/release/poworker   ./hacash_poworker_ubuntu
cp target/release/diaworker ./hacash_diaworker_ubuntu


# or static linked
# edit chain/Cargo.toml and protocol/Cargo.toml, change "db-leveldb-sys" to "db-sled"
rustup target add x86_64-unknown-linux-musl
RUSTFLAGS="-C target-feature=+crt-static" RUST_BACKTRACE="full" cargo build --release --target=x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/fullnode   ./hacash_fullnode_ubuntu_18.0
cp target/x86_64-unknown-linux-musl/release/poworker   ./hacash_poworker_ubuntu_18.0
cp target/x86_64-unknown-linux-musl/release/diaworker ./hacash_diaworker_ubuntu_18.0


# cross build for windows
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
rustup toolchain install stable-x86_64-pc-windows-gnu
RUSTFLAGS="-C target-feature=+crt-static" RUST_BACKTRACE="full" cargo build --release --target x86_64-pc-windows-gnu
cp target/x86_64-pc-windows-gnu/release/fullnode.exe   ./hacash_fullnode_windows.exe
cp target/x86_64-pc-windows-gnu/release/poworker.exe   ./hacash_poworker_windows.exe
cp target/x86_64-pc-windows-gnu/release/diaworker.exe ./hacash_diaworker_windows.exe



# cross build for macos
# https://wapl.es/rust/2019/02/17/rust-cross-compile-linux-to-macos.html/
sudo apt install clang gcc g++ zlib1g-dev libmpc-dev libmpfr-dev libgmp-dev openssl libssl-dev
# install build osxcross
git clone https://github.com/tpoechtrager/osxcross
cd osxcross
wget -nc https://s3.dockerproject.org/darwin/v2/MacOSX10.10.sdk.tar.xz
mv MacOSX10.10.sdk.tar.xz tarballs/
UNATTENDED=yes OSX_VERSION_MIN=10.6 ./build.sh
# build
rustup target add x86_64-apple-darwin
rustup toolchain install stable-x86_64-apple-darwin
./build_macos.sh


```

Windows:

```powershell

## gnu
# download and install: https://cmake.org/download/
# download and install: https://www.msys2.org/
pacman -Sy && pacman -Syu
pacman -S mingw-w64-x86_64-toolchain

rustup target add x86_64-pc-windows-gnu
rustup toolchain install stable-x86_64-pc-windows-gnu
set RUSTFLAGS='-C target-feature=+crt-static'; set RUST_BACKTRACE='full'; cargo build --release --target x86_64-pc-windows-gnu;
cp target/x86_64-pc-windows-gnu/release/fullnode.exe   ./hacash_fullnode_windows.exe
cp target/x86_64-pc-windows-gnu/release/poworker.exe   ./hacash_poworker_windows.exe
cp target/x86_64-pc-windows-gnu/release/diaworker.exe ./hacash_diaworker_windows.exe

## or msvc
rustup target add x86_64-pc-windows-msvc
rustup toolchain install stable-x86_64-pc-windows-msvc
set RUSTFLAGS='-C target-feature=+crt-static'; set RUST_BACKTRACE='full'; cargo build --release --target x86_64-pc-windows-msvc;
cp target/x86_64-pc-windows-msvc/release/fullnode.exe   ./hacash_fullnode_windows.exe
cp target/x86_64-pc-windows-msvc/release/poworker.exe   ./hacash_poworker_windows.exe
cp target/x86_64-pc-windows-msvc/release/diaworker.exe ./hacash_diaworker_windows.exe

# dumpbin /dependents  ./hacash_fullnode_windows.exe

```

MacOS:

```sh

               
RUSTFLAGS='-C target-feature=+crt-static' RUST_BACKTRACE='full' cargo build --release --target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/fullnode   ./hacash_fullnode_macos 
cp target/x86_64-apple-darwin/release/poworker   ./hacash_poworker_macos
cp target/x86_64-apple-darwin/release/diaworker ./hacash_diaworker_macos



```