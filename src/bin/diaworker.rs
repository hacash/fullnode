
use app::*;

fn main() {
    println!("[Version] Diamond miner worker v{}, build time: {}.", 
        HACASH_NODE_VERSION, HACASH_NODE_BUILD_TIME
    );
    app::diaworker::diaworker()
}