fn main() {
    println!(
        "[Version] Diamond miner worker v{}, build time: {}.",
        app::HACASH_NODE_VERSION,
        app::HACASH_NODE_BUILD_TIME
    );
    if let Err(e) = app::worker::diamond::run() {
        eprintln!("[diaworker] {}", e);
        std::process::exit(1);
    }
}
