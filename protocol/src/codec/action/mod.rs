//! HAC Action
//!
//! Standard transfer / guard / blob / AST / envfunc / tex action codecs registered by
//! `protocol::setup::register_standard`. Split into submodules for maintainability;
//! all public items are re-exported here so `protocol::action_std::*` and
//! `crate::codec::action::*` keep working.

mod ast;
mod blob;
mod common;
mod envfunc;
mod guard;
mod tex;
mod transfer;

pub use ast::{ActionListW1, AstIf, AstSelect, create_ast_if, create_ast_select};
pub use blob::{TxBlob, TxMessage, create_blob_action};
pub use envfunc::{
    EnvBlockAuthorAddr, EnvHeight, EnvMainAddr, ViewAssetBalance, ViewBalance, ViewCheckSign,
    ViewDiaInscGet, ViewDiaInscNum, ViewDiaNameList, ViewDiaOwnerAddrs, create_envfunc_action,
};
pub use guard::{BalanceFloor, ChainAllow, HeightScope, ReqSignList, create_chain_guard_action};
pub use tex::{TexCellAct, create_tex_cell_act};
pub use transfer::{
    AssetFromToTrs, AssetFromTrs, AssetToTrs, DiaFromToTrs, DiaFromTrs, DiaSingleTrs, DiaToTrs,
    HacFromToTrs, HacFromTrs, HacToTrs, HacTransfer, SatFromToTrs, SatFromTrs, SatToTrs,
    create_asset_transfer, create_diamond_transfer, create_hac_transfer, create_sat_transfer,
};
