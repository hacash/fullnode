//! `fullnode` binary — default full node without an external indexer.
use sys::Rerr;

fn main() -> Rerr {
    println!(
        "[Version] full node v{}, build time: {}, database version: {}, backend: {}.",
        app::HACASH_NODE_VERSION,
        app::HACASH_NODE_BUILD_TIME,
        app::DB_VERSION,
        db::backend_name()
    );
    app::run()
}
