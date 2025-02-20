/*

RUSTFLAGS="-C target-feature=-crt-static" cargo build

cp hacash.config.ini ./target/debug/ && rm -rf ./target/debug/hacash_mainnet_data/ && RUST_BACKTRACE=all cargo run --bin fullnode

*/




/*
* main fullnode
*/
include!{"./bin/fullnode.rs"}


include!("devtest.rs");

