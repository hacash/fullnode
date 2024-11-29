# fullnode
Hacash Fullnode Software and SDK



Ubuntu:

```sh

RUSTFLAGS="-C target-feature=-crt-static" RUST_BACKTRACE="full" cargo build --release
cp target/release/fullnode  ./hacash_fullnode_ubuntu
cp target/release/poworker  ./hacash_poworker_ubuntu
cp target/release/diaworker ./hacash_diaworker_ubuntu

```

Windows:

```powershell


set RUSTFLAGS='-C target-feature=+crt-static'; set RUST_BACKTRACE='full'; cargo build --release --target x86_64-pc-windows-msvc;

cp target/x86_64-pc-windows-msvc/release/fullnode.exe  ./hacash_fullnode_windows.exe
cp target/x86_64-pc-windows-msvc/release/poworker.exe  ./hacash_poworker_windows.exe
cp target/x86_64-pc-windows-msvc/release/diaworker.exe ./hacash_diaworker_windows.exe


```
