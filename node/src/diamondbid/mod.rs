use std::sync::Arc;
use std::time::*;
use std::thread;


use sys::*;
use chain::engine::*;
use field::*;
use field::interface::*;
use protocol::*;
use protocol::transaction::*;

use super::interface::*;
use super::memtxpool::*;


include!("bidding.rs");


pub fn start_diamond_auto_bidding(hnode: Arc<dyn HNode>) -> Rerr {
    
    // check config
    let eng = hnode.engine();
    let cnf = eng.config();
    let bidmin = cnf.dmer_bid_min.clone();
    let bidmax = cnf.dmer_bid_max.clone();
    let bidstep = cnf.dmer_bid_step.clone();
    let minstep = Amount::coin(1, 244);

    if ! cnf.dmer_enable {
        return Ok(()) // not enable
    }

    macro_rules! printerr {
        ( $f: expr, $( $v: expr ),+ ) => {
            println!("\n\n{} {}\n\n", 
                "[Diamond Auto Bid Config Warning]",
                format!($f, $( $v ),+)
            );
        }
    }

    if bidstep < minstep {
        printerr!("bid step amount cannot less than {} HAC", &minstep );
    }

    if bidmax < bidmin {
        printerr!("max bid fee {} cannot less than min fee {}", &bidmax, &bidmin);
        panic!("");
    }

    println!("[Diamond Auto Bidding] Start with account {} min fee {} and max fee {}.",
        &cnf.dmer_bid_account.readable(), &bidmin, &bidmax
    );
    
    // thread loop 
    let engcnf = cnf.clone();
    thread::spawn(move || {
        thread::sleep( Duration::from_secs(15) );
        let mut current_number: u32 = 0;
        loop {
            let pending_height = eng.latest_block().height().uint() + 1;
            check_bidding_step(hnode.clone(), &engcnf, pending_height, &mut current_number);
            // sleep 0.3 secs
            thread::sleep( Duration::from_millis(77) );
        }
    });

    Ok(())
}



