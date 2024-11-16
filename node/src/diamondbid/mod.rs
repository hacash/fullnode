use std::sync::{ Arc };
use std::time::*;
use std::thread;


use crate::sys::*;
use crate::config::EngineConf;
use crate::core::field::*;
use crate::core::component::TxPackage;
use crate::mint::action as mint_action;

use crate::interface::field::*;
use crate::interface::protocol::*;
use crate::interface::node::*;
use crate::interface::chain::*;

use super::memtxpool::TXPOOL_GROUP_DIAMOND_MINT;


include!("bidding.rs");


pub fn start_diamond_auto_bidding(hnode: Arc<dyn HNode>) -> RetErr {
    
    // check config
    let eng = hnode.engine();
    let cnf = eng.config();
    let bidmin = cnf.dmer_bid_min.clone();
    let bidmax = cnf.dmer_bid_max.clone();
    let mut bidstep = cnf.dmer_bid_step.clone();
    let minstep = Amount::new_small(1, 244);

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

    if bidstep.less_than(&minstep) {
        printerr!("bid step amount cannot less than {} HAC",
            &minstep.to_fin_string()
        );
        bidstep = minstep;
    }

    if bidmax.less_than(&bidmin) {
        printerr!("max bid fee {} cannot less than min fee {}", 
            &bidmax.to_fin_string(), &bidmin.to_fin_string()
        );
        panic!("");
    }

    println!("[Diamond Auto Bidding] Start with account {} min fee {} and max fee {}.",
        &cnf.dmer_bid_account.readable(), &bidmin.to_fin_string(), &bidmax.to_fin_string()
    );
    
    // thread loop 
    let engcnf = cnf.clone();
    thread::spawn(move || {
        thread::sleep( Duration::from_secs(15) );
        let mut current_number: u32 = 0;
        loop {
            let pending_height = eng.latest_block().objc().height().uint() + 1;
            check_bidding_step(hnode.clone(), &engcnf, pending_height, &mut current_number);
            // sleep 0.3 secs
            thread::sleep( Duration::from_millis(77) );
        }
    });

    Ok(())
}



