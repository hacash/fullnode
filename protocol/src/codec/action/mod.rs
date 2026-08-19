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

pub use ast::{
    ActionListW1, AstIf, AstSelect, create_ast_if, create_ast_select, decode_ast_if_json,
    decode_ast_select_json,
};
pub use blob::{TxBlob, TxMessage, create_blob_action};
pub use envfunc::{
    EnvBlockAuthorAddr, EnvHeight, EnvMainAddr, ViewAssetBalance, ViewBalance, ViewCheckSign,
    ViewDiaInscGet, ViewDiaInscNum, ViewDiaNameList, ViewDiaOwnerAddrs, create_envfunc_action,
};
pub use guard::{
    BalanceFloor, ChainAllow, GuardFacts, HeightScope, ReqSignList, create_chain_guard_action,
    decode_req_sign_list_json, guard_facts,
};
pub use tex::{TexCellAct, create_tex_cell_act, decode_tex_cell_act_json, tex_cell_schema};
pub use transfer::{
    AssetFromToTrs, AssetFromTrs, AssetToTrs, DiaFromToTrs, DiaFromTrs, DiaSingleTrs, DiaToTrs,
    HacFromToTrs, HacFromTrs, HacToTrs, HacTransfer, SatFromToTrs, SatFromTrs, SatToTrs,
    create_asset_transfer, create_diamond_transfer, create_hac_transfer, create_sat_transfer,
    decode_diamond_transfer_json,
};
