use std::sync::Arc;
use std::thread;
use std::time::Duration;

use base::{ChainId, Node, PkgOrigin, PkgSource, Transaction, TransactionBuild, TxPkg, TxPool};
use field::{Address, Amount, AmtCpr, Encode};
use mint::MinerConf;
use mint::action_diamond::DiamondMint;
use protocol::tx_std::TransactionType2;
use sys::Waiter;

const TX_POOL_GROUP_DIAMOND_MINT: base::TxGroupId = base::TxGroupId::new(1);

pub(super) fn start(
    conf: MinerConf,
    node: Arc<dyn Node>,
    chain_id: ChainId,
    waiter: Waiter,
) -> Option<thread::JoinHandle<()>> {
    if !conf.diamond_enable || !chain_id.is_mainnet() {
        return None;
    }
    let min_step = Amount::coin(1, 244);
    if conf.diamond_bid_step < min_step {
        eprintln!(
            "[diabider] bid step {} is lower than minimum {}",
            conf.diamond_bid_step, min_step
        );
    }
    if conf.diamond_bid_max < conf.diamond_bid_min {
        eprintln!(
            "[diabider] max bid fee {} cannot be less than min fee {}",
            conf.diamond_bid_max, conf.diamond_bid_min
        );
        return None;
    }

    println!(
        "[diabider] auto bidding account={} min={} max={}",
        conf.diamond_bid_account.readable(),
        conf.diamond_bid_min,
        conf.diamond_bid_max
    );

    Some(thread::spawn(move || {
        if waiter.sleep_or_quit(Duration::from_secs(15)) {
            return;
        }
        let mut current_number = 0u32;
        loop {
            if waiter.is_shutdown() {
                break;
            }
            let pending_height = node.engine().latest_height() + 1;
            check_bidding_step(node.clone(), &conf, pending_height, &mut current_number);
            if waiter.sleep_or_quit(Duration::from_millis(77)) {
                break;
            }
        }
    }))
}

fn check_bidding_step(
    node: Arc<dyn Node>,
    conf: &MinerConf,
    pending_height: u64,
    bidding_number: &mut u32,
) {
    if pending_height % 5 == 0 {
        return;
    }

    let my_addr = Address::from(*conf.diamond_bid_account.address());
    let mut bid_step = conf.diamond_bid_step.clone();
    let min_step = Amount::coin(1, 244);
    if bid_step < min_step {
        bid_step = min_step;
    }

    macro_rules! retry {
        ($ms:expr) => {{
            thread::sleep(Duration::from_millis($ms));
            return;
        }};
    }

    let txpool = node.txpool();
    let (Some(first_bid), Some(my_bid)) = pick_first_and_my_bid_tx(txpool.as_ref(), &my_addr)
    else {
        retry!(3);
    };
    if first_bid.tx().main() == my_addr {
        retry!(1);
    }
    let first_bid_fee = first_bid.tx().fee().clone();
    if first_bid_fee > conf.diamond_bid_max {
        retry!(10);
    }
    let Ok(first_bid_fee) = first_bid_fee.compress(2, AmtCpr::Grow) else {
        eprintln!(
            "[diabider] cannot compress fee {} to 4 length",
            first_bid_fee
        );
        retry!(10);
    };
    if my_bid.tx().main() == first_bid.tx().main() {
        retry!(1);
    }
    if my_bid.tx().fee() >= &conf.diamond_bid_max {
        retry!(5);
    }

    let Ok(new_bid_fee) = first_bid_fee.add_mode_u64(&bid_step) else {
        eprintln!(
            "[diabider] cannot add fee {} with {}",
            first_bid_fee, bid_step
        );
        retry!(10);
    };
    let Ok(mut new_bid_fee) = new_bid_fee.compress(2, AmtCpr::Grow) else {
        eprintln!("[diabider] cannot compress fee {} to 4 length", new_bid_fee);
        retry!(10);
    };
    if new_bid_fee > conf.diamond_bid_max {
        new_bid_fee = conf.diamond_bid_max.clone();
    }
    if new_bid_fee <= first_bid_fee {
        retry!(10);
    }

    if let Some(mint) = pick_diamond_mint_action(my_bid.tx()) {
        let number = mint.d.number.uint();
        if *bidding_number != number {
            *bidding_number = number;
            println!(
                "[diabider] diamond {}({}) raise fee to {}",
                mint.d.diamond.to_readable(),
                number,
                new_bid_fee.to_fin_string()
            );
        }
    }

    let Some(mut tx) = my_bid
        .tx()
        .as_any()
        .downcast_ref::<TransactionType2>()
        .cloned()
    else {
        return;
    };
    tx.set_fee(new_bid_fee);
    if let Err(e) = tx.fill_sign_account(&conf.diamond_bid_account) {
        eprintln!("[diabider] fill sign failed: {}", e);
        retry!(3);
    }
    let pkg = match TxPkg::from_bytes(
        node.engine().services().as_ref(),
        tx.encode(),
        PkgSource::new(PkgOrigin::Local),
    ) {
        Ok(pkg) => pkg,
        Err(e) => {
            eprintln!("[diabider] create tx package failed: {}", e);
            retry!(3);
        }
    };
    if let Err(e) = node.submit_transaction(&pkg, false, false) {
        eprintln!("[diabider] submit tx error: {}", e);
        retry!(3);
    }
}

fn pick_first_and_my_bid_tx(
    txpool: &dyn TxPool,
    my_addr: &Address,
) -> (Option<TxPkg>, Option<TxPkg>) {
    let mut first = None;
    let mut mine = None;
    let mut pick = |pkg: &TxPkg| {
        if first.is_none() {
            first = Some(pkg.clone());
        }
        if mine.is_none() && pkg.tx().main() == *my_addr {
            mine = Some(pkg.clone());
        }
        !(first.is_some() && mine.is_some())
    };
    let _ = txpool.iter(TX_POOL_GROUP_DIAMOND_MINT, &mut pick);
    (first, mine)
}

fn pick_diamond_mint_action(tx: &dyn Transaction) -> Option<&DiamondMint> {
    tx.actions()
        .iter()
        .find_map(|act| act.as_any().downcast_ref::<DiamondMint>())
}
