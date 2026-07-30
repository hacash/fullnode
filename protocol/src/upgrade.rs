//! Protocol activation compatibility hooks.
//!
//! The next node is deployed after the historical online activation height,
//! so the old `765432` runtime gate is intentionally not enforced here.
//! These functions remain as stable call sites for the execution layers and
//! always accept. Asset issuance keeps its independent historical start height
//! in `mint::action::asset::ASSET_ALIVE_HEIGHT`.

use field::Address;
use sys::Rerr;

pub const MAINNET_CHAIN_ID: u32 = 0;

#[inline]
pub fn is_online_upgrade_open(_height: u64) -> bool {
    true
}

#[inline]
pub fn check_gated_tx(_chain_id: base::ChainId, _height: u64, _tx_type: u8) -> Rerr {
    Ok(())
}

#[inline]
pub fn check_gated_action(_chain_id: base::ChainId, _height: u64, _kind: u16) -> Rerr {
    Ok(())
}

#[inline]
pub fn check_transfer_addr_online_open(
    _chain_id: base::ChainId,
    _height: u64,
    _from: &Address,
    _to: &Address,
) -> Rerr {
    Ok(())
}
