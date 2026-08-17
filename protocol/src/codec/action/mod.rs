//! HAC Action
//!
//! Standard transfer / guard / blob / AST / envfunc / tex action codecs registered by
//! `protocol::setup::register_standard`. Split into submodules for maintainability;
//! all public items are re-exported here so `protocol::action_std::*` and
//! `crate::codec::action::*` keep working.

/// codec-only（SDK wasm）下所有标准 action 的 `Action::execute` 入口桩：直接
/// 返回错误，不进入完整执行函数，保证执行实现不进入 wasm 依赖闭包。
#[cfg(all(feature = "codec-only", target_arch = "wasm32"))]
pub(crate) fn execution_disabled() -> sys::Ret<Vec<u8>> {
    sys::errf!("protocol action execution is not included in the sdk (codec-only) build")
}

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
    BalanceFloor, ChainAllow, HeightScope, ReqSignList, create_chain_guard_action,
    decode_req_sign_list_json,
};
pub use tex::{TexCellAct, create_tex_cell_act, decode_tex_cell_act_json};
pub use transfer::{
    AssetFromToTrs, AssetFromTrs, AssetToTrs, DiaFromToTrs, DiaFromTrs, DiaSingleTrs, DiaToTrs,
    HacFromToTrs, HacFromTrs, HacToTrs, HacTransfer, SatFromToTrs, SatFromTrs, SatToTrs,
    create_asset_transfer, create_diamond_transfer, create_hac_transfer, create_sat_transfer,
    decode_diamond_transfer_json,
};
