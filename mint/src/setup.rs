//! mint  Registry `mint::setup::register`

use base::RegistryWriter;
use sys::Rerr;

use crate::action::asset::{AssetCreate, create_asset_create};
use crate::action::channel::{
    ChannelClose, ChannelOpen, create_channel_close, create_channel_open,
};
use crate::action::diamond::{DiamondMint, create_diamond_mint, decode_diamond_mint_json};
use crate::tx_coinbase::{CoinbaseTx, create_coinbase};

pub fn register(reg: &mut dyn RegistryWriter) -> Rerr {
    // 铭刻动作（32-36）注册已移至 mint-core（mint 与 sdk 共用同一入口）
    mint_core::setup::register(reg)?;
    reg.register_tx(CoinbaseTx::TYPE, create_coinbase)?;
    base::register_regular_actions!(
        reg,
        create_channel_open => [ChannelOpen],
        create_channel_close => [ChannelClose],
        create_asset_create => [AssetCreate],
    )?;
    base::register_custom_actions!(
        reg,
        create_diamond_mint,
        decode_diamond_mint_json => [DiamondMint],
    )?;
    Ok(())
}
