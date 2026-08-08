use base::{
    Context, Env, ExecutionServices, RegistryWriter, StateChunkRef, TxRef, VmHostActionDef,
    VmHostAllowedPolicy, VmHostCallKind, VmValueType,
};
use std::sync::Arc;
use sys::{Rerr, Ret};

use crate::codec::action::{
    AssetFromToTrs, AssetFromTrs, AssetToTrs, AstIf, AstSelect, BalanceFloor, ChainAllow,
    DiaFromToTrs, DiaFromTrs, DiaSingleTrs, DiaToTrs, EnvBlockAuthorAddr, EnvHeight, EnvMainAddr,
    HacFromToTrs, HacFromTrs, HacToTrs, HeightScope, ReqSignList, SatFromToTrs, SatFromTrs,
    SatToTrs, TexCellAct, TxBlob, TxMessage, ViewAssetBalance, ViewBalance, ViewCheckSign,
    ViewDiaInscGet, ViewDiaInscNum, ViewDiaNameList, ViewDiaOwnerAddrs, create_asset_transfer,
    create_ast_if, create_ast_select, create_blob_action, create_chain_guard_action,
    create_diamond_transfer, create_envfunc_action, create_hac_transfer, create_sat_transfer,
    create_tex_cell_act, decode_ast_if_json, decode_ast_select_json, decode_diamond_transfer_json,
    decode_req_sign_list_json, decode_tex_cell_act_json,
};
use crate::codec::block::create_std_block;
use crate::codec::tx::{
    TransactionType1, TransactionType2, TransactionType3, create_transaction_type1,
    create_transaction_type2, create_transaction_type3,
};

fn create_context(
    env: Env,
    registry: Arc<dyn ExecutionServices>,
    chunk: StateChunkRef,
    tx: TxRef,
    _gas_budget: i64,
) -> Ret<Box<dyn Context>> {
    Ok(Box::new(crate::exec::context::ContextInst::new(
        env, registry, chunk, tx,
    )?))
}

fn register_host_def(reg: &mut dyn RegistryWriter, def: VmHostActionDef) -> Rerr {
    reg.register_vm_host_def(def)
}

fn register_action_def(
    reg: &mut dyn RegistryWriter,
    id: u8,
    name: &'static str,
    argc: usize,
) -> Rerr {
    // Transfer EXTACTION is Main+depth0 only (vm ensure_act_allowed); metadata matches.
    register_host_def(
        reg,
        VmHostActionDef {
            id,
            name,
            kind: VmHostCallKind::Action,
            ret: VmValueType::Nil,
            argc,
            pass_body: true,
            have_retv: false,
            allowed_policy: VmHostAllowedPolicy::TopOnly,
        },
    )
}

fn register_env_def(
    reg: &mut dyn RegistryWriter,
    id: u8,
    name: &'static str,
    ret: VmValueType,
) -> Rerr {
    register_host_def(
        reg,
        VmHostActionDef {
            id,
            name,
            kind: VmHostCallKind::Env,
            ret,
            argc: 0,
            pass_body: false,
            have_retv: true,
            allowed_policy: VmHostAllowedPolicy::Any,
        },
    )
}

fn register_view_def(
    reg: &mut dyn RegistryWriter,
    id: u8,
    name: &'static str,
    ret: VmValueType,
    argc: usize,
) -> Rerr {
    register_host_def(
        reg,
        VmHostActionDef {
            id,
            name,
            kind: VmHostCallKind::View,
            ret,
            argc,
            pass_body: true,
            have_retv: true,
            allowed_policy: VmHostAllowedPolicy::ViewOnly,
        },
    )
}

/// VM host capability surface for the standard Hacash protocol.
/// Host defs are split across crates into the same Registry:
/// - protocol owns transfer / env / view defs (this function)
/// - mint owns its additional inscription host definitions
fn register_vm_host_defs(reg: &mut dyn RegistryWriter) -> Rerr {
    register_action_def(reg, HacToTrs::KIND as u8, "transfer_hac_to", 2)?;
    register_action_def(reg, HacFromTrs::KIND as u8, "transfer_hac_from", 2)?;
    register_action_def(reg, HacFromToTrs::KIND as u8, "transfer_hac_from_to", 3)?;
    register_action_def(reg, SatToTrs::KIND as u8, "transfer_sat_to", 2)?;
    register_action_def(reg, SatFromTrs::KIND as u8, "transfer_sat_from", 2)?;
    register_action_def(reg, SatFromToTrs::KIND as u8, "transfer_sat_from_to", 3)?;
    register_action_def(reg, DiaSingleTrs::KIND as u8, "transfer_hacd_single_to", 2)?;
    register_action_def(reg, DiaToTrs::KIND as u8, "transfer_hacd_to", 2)?;
    register_action_def(reg, DiaFromTrs::KIND as u8, "transfer_hacd_from", 2)?;
    register_action_def(reg, DiaFromToTrs::KIND as u8, "transfer_hacd_from_to", 3)?;
    register_action_def(reg, AssetToTrs::KIND as u8, "transfer_asset_to", 2)?;
    register_action_def(reg, AssetFromTrs::KIND as u8, "transfer_asset_from", 2)?;
    register_action_def(reg, AssetFromToTrs::KIND as u8, "transfer_asset_from_to", 3)?;

    // Host ids = KIND % 256 (mainnet-compatible ACTENV / ACTVIEW idx).
    register_env_def(reg, 1, "block_height", VmValueType::U64)?; // 0x0701
    register_env_def(reg, 2, "tx_main_addr", VmValueType::Address)?; // 0x0702
    register_env_def(reg, 3, "block_author_addr", VmValueType::Address)?; // 0x0703

    register_view_def(reg, 1, "balance", VmValueType::Bytes, 1)?; // 0x0601
    register_view_def(reg, 2, "asset_balance", VmValueType::U64, 2)?; // 0x0602
    register_view_def(reg, 9, "check_signature", VmValueType::Bool, 1)?; // 0x0609
    register_view_def(reg, 17, "hacd_insc_num", VmValueType::U8, 1)?; // 0x0611
    register_view_def(reg, 18, "hacd_insc_get", VmValueType::Bytes, 2)?; // 0x0612
    register_view_def(reg, 19, "hacd_name_list", VmValueType::Bytes, 3)?; // 0x0613
    register_view_def(reg, 20, "hacd_owner_addrs", VmValueType::Bytes, 1)?; // 0x0614
    Ok(())
}

pub fn register_standard(
    reg: &mut dyn RegistryWriter,
    params: &'static crate::ProtocolParams,
) -> Rerr {
    reg.set_execution_profile(params)?;
    reg.set_vm_params(params.vm)?;
    // Type 0 is CoinbaseTx (registered by mint); do not register DefaultPreludeTx.
    reg.register_tx(TransactionType1::TYPE, create_transaction_type1)?;
    reg.register_tx(TransactionType2::TYPE, create_transaction_type2)?;
    reg.register_tx(TransactionType3::TYPE, create_transaction_type3)?;
    base::register_regular_actions!(
        reg,
        create_hac_transfer => [HacToTrs, HacFromTrs, HacFromToTrs],
        create_sat_transfer => [SatToTrs, SatFromTrs, SatFromToTrs],
        create_asset_transfer => [AssetToTrs, AssetFromTrs, AssetFromToTrs],
        create_blob_action => [TxMessage, TxBlob],
        create_chain_guard_action => [ChainAllow, HeightScope, BalanceFloor],
        create_envfunc_action => [
            EnvHeight,
            EnvMainAddr,
            EnvBlockAuthorAddr,
            ViewBalance,
            ViewAssetBalance,
            ViewCheckSign,
            ViewDiaInscNum,
            ViewDiaInscGet,
            ViewDiaNameList,
            ViewDiaOwnerAddrs,
        ],
    )?;
    base::register_custom_actions!(
        reg,
        create_diamond_transfer,
        decode_diamond_transfer_json => [DiaSingleTrs, DiaFromToTrs, DiaToTrs, DiaFromTrs],
    )?;
    base::register_custom_actions!(
        reg,
        create_chain_guard_action,
        decode_req_sign_list_json => [ReqSignList],
    )?;
    base::register_custom_actions!(
        reg,
        create_ast_select,
        decode_ast_select_json => [AstSelect],
    )?;
    base::register_custom_actions!(reg, create_ast_if, decode_ast_if_json => [AstIf])?;
    base::register_custom_actions!(
        reg,
        create_tex_cell_act,
        decode_tex_cell_act_json => [TexCellAct],
    )?;
    reg.set_block_creator(create_std_block)?;
    reg.set_context_creator(create_context, base::DEFAULT_GAS_BUDGET)?;
    register_vm_host_defs(reg)?;
    Ok(())
}
