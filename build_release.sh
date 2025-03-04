
# db-leveldb-sys
RUSTFLAGS="-C target-feature=-crt-static" RUST_BACKTRACE="full" cargo build --release --bin fullnode --no-default-features --features "db-leveldb-sys"
cp target/release/fullnode   ./hacash_fullnode_ubuntu
cp target/release/poworker   ./hacash_poworker_ubuntu
cp target/release/diaworker ./hacash_diaworker_ubuntu


# db-rusty-leveldb
RUSTFLAGS="-C target-feature=+crt-static" RUST_BACKTRACE="full" cargo build --release --bin fullnode --target=x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/fullnode   ./hacash_fullnode_ubuntu_16.04
cp target/x86_64-unknown-linux-musl/release/poworker   ./hacash_poworker_ubuntu_16.04
cp target/x86_64-unknown-linux-musl/release/diaworker ./hacash_diaworker_ubuntu_16.04

# or for db-sled
RUSTFLAGS="-C target-feature=+crt-static" RUST_BACKTRACE="full" cargo build --release --bin fullnode --target=x86_64-unknown-linux-musl --no-default-features --features "db-sled"
cp target/x86_64-unknown-linux-musl/release/fullnode   ./hacash_fullnode_ubuntu_dbsled


# build hascan
cd ../hascan
RUSTFLAGS="$RUSTFLAGS -Awarnings" cargo build --release && cp ./target/release/hascan ./ && cp ./hascan ../fullnode/
cd ../fullnode

