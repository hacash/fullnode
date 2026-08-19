//! mint  Registry `mint::setup::register`
//!
//! All actions (inscription 32-36, channel 2/3, asset 16, diamond mint 4) have
//! moved to `mint-core`; the full action/tx codec surface is assembled once by
//! `chain-codec::register_standard` (used by the full node registry, the SDK
//! and `codec-schema-gen`). Here we only register this crate's own CoinbaseTx
//! (tx type 0), which is a block-level transaction, not a wallet-signable one.

use base::WireRegistry;
use sys::Rerr;

use crate::tx_coinbase::{CoinbaseTx, create_coinbase};

pub fn register(reg: &mut dyn WireRegistry) -> Rerr {
    reg.register_tx(CoinbaseTx::TYPE, create_coinbase)?;
    Ok(())
}
