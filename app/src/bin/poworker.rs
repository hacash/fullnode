fn main() {
    println!(
        "[Version] HAC miner worker v{}, build time: {}.",
        app::HACASH_NODE_VERSION,
        app::HACASH_NODE_BUILD_TIME
    );
    if let Err(e) = app::worker::pow::run() {
        eprintln!("[poworker] {}", e);
        std::process::exit(1);
    }
}
