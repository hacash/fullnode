

use app::*;


fn main() {
    println!("[Version] HAC miner worker v{}, build time: {}.", 
        HACASH_NODE_VERSION, HACASH_NODE_BUILD_TIME
    );
    app::poworker::poworker()
}


