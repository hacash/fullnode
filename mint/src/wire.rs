//! Mint-owned wire registration. Consensus actions live in `mint-core`; this
//! crate owns only CoinbaseTx (transaction type 0), block-level and not wallet-signable.

use base::{TxCodecBinding, WireRegistry};
use sys::Rerr;

use crate::tx_coinbase::{CoinbaseTx, create_coinbase};

pub const TX_CODECS: &[TxCodecBinding] = &[TxCodecBinding {
    ty: CoinbaseTx::TYPE,
    decode_wire: create_coinbase,
}];

pub fn register_wire(reg: &mut dyn WireRegistry) -> Rerr {
    for binding in TX_CODECS {
        reg.register_tx_codec(*binding)?;
    }
    Ok(())
}
