use base::Transaction;
use field::Amount;
use sys::{Rerr, errf};

use crate::minter::block_reward_number;
use crate::tx_coinbase::CoinbaseTx;

pub fn verify_coinbase(height: u64, tx: &dyn Transaction) -> Rerr {
    if tx.ty() != CoinbaseTx::TYPE {
        return errf!("mainnet prelude tx must be coinbase");
    }
    let Some(got) = tx.block_reward() else {
        return errf!("coinbase transaction missing block reward");
    };
    let need = Amount::mei(block_reward_number(height) as u64);
    if &need != got {
        return errf!("block coinbase reward expected {} but got {}", need, got);
    }
    Ok(())
}

pub fn verify_coinbase_privakey(tx: &dyn Transaction) -> Rerr {
    let Some(addr) = tx.author() else {
        return Ok(());
    };
    if !addr.is_privkey() {
        return errf!(
            "coinbase address {} must be PRIVAKEY type but got version {}",
            addr.to_readable(),
            addr.version()
        );
    }
    if addr.is_privkey_unknown() {
        return errf!(
            "coinbase address {} is a system address with unknown private key",
            addr.to_readable()
        );
    }
    Ok(())
}
