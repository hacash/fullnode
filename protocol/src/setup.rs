use base::*;
use std::sync::Arc;
use sys::{Rerr, Ret, errf};

use crate::codec::action::*;
use crate::codec::block::create_std_block;
use crate::codec::tx::*;

#[cfg(feature = "execute")]
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

fn register_host_def(reg: &mut dyn ExecRegistry, def: VmHostActionDef) -> Rerr {
    reg.register_vm_host_def(def)
}

/// Transfer EXTACTION hosts: the wire id is the kind itself (`id = kind`), so a
/// kind larger than `0xff` can never be silently truncated into the u8 opcode id.
fn register_action_def(
    reg: &mut dyn ExecRegistry,
    kind: u16,
    name: &'static str,
    argc: usize,
) -> Rerr {
    if kind > 0xff {
        return errf!(
            "VM ACTION host {} kind {:#06x} cannot fit the u8 opcode id",
            name,
            kind
        );
    }
    // Transfer EXTACTION is Main+depth0 only (vm ensure_act_allowed); metadata matches.
    register_host_def(
        reg,
        VmHostActionDef {
            id: kind as u8,
            name,
            kind: VmHostCallKind::Action,
            ret: VmValueType::Nil,
            argc,
            allowed_policy: VmHostAllowedPolicy::TopOnly,
        },
    )
}

/// ACTENV hosts live in the 0x07xx opcode space; the opcode id is the low byte.
fn register_env_def(
    reg: &mut dyn ExecRegistry,
    kind: u16,
    name: &'static str,
    ret: VmValueType,
) -> Rerr {
    if kind >> 8 != 0x07 {
        return errf!(
            "VM ACTENV host {} kind {:#06x} must be in the 0x07xx opcode space",
            name,
            kind
        );
    }
    register_host_def(
        reg,
        VmHostActionDef {
            id: kind as u8,
            name,
            kind: VmHostCallKind::Env,
            ret,
            argc: 0,
            allowed_policy: VmHostAllowedPolicy::Any,
        },
    )
}

/// ACTVIEW hosts live in the 0x06xx opcode space; the opcode id is the low byte.
fn register_view_def(
    reg: &mut dyn ExecRegistry,
    kind: u16,
    name: &'static str,
    ret: VmValueType,
    argc: usize,
) -> Rerr {
    if kind >> 8 != 0x06 {
        return errf!(
            "VM ACTVIEW host {} kind {:#06x} must be in the 0x06xx opcode space",
            name,
            kind
        );
    }
    register_host_def(
        reg,
        VmHostActionDef {
            id: kind as u8,
            name,
            kind: VmHostCallKind::View,
            ret,
            argc,
            allowed_policy: VmHostAllowedPolicy::ViewOnly,
        },
    )
}

/// Host-definition registration sugar. The action type is the single
/// declaration point of `KIND`/`NAME`, so each batch entry only repeats the ABI
/// data that actually varies (source arity / return value type). The return
/// variant is auto-qualified (`U64` expands to `VmValueType::U64`). Expands
/// into the validating `register_*_def` helpers above.
macro_rules! register_vm_hosts {
    ($reg:expr, action; $( $ty:ty = $argc:expr ),+ $(,)?) => {{
        $(
            register_action_def($reg, <$ty>::KIND, <$ty>::NAME, $argc)?;
        )+
        Ok::<(), sys::Error>(())
    }};
    ($reg:expr, env; $( $ty:ty = $ret:ident ),+ $(,)?) => {{
        $(
            register_env_def($reg, <$ty>::KIND, <$ty>::NAME, VmValueType::$ret)?;
        )+
        Ok::<(), sys::Error>(())
    }};
    ($reg:expr, view; $( $ty:ty = ($ret:ident, $argc:expr) ),+ $(,)?) => {{
        $(
            register_view_def($reg, <$ty>::KIND, <$ty>::NAME, VmValueType::$ret, $argc)?;
        )+
        Ok::<(), sys::Error>(())
    }};
}

/// VM host capability surface for the standard Hacash protocol.
/// Host defs are split across crates into the same Registry:
/// - protocol owns transfer / env / view defs (this function)
/// - mint owns its additional inscription host definitions
fn register_vm_host_defs(reg: &mut dyn ExecRegistry) -> Rerr {
    register_vm_hosts!(reg, action;
        HacToTrs = 2,
        HacFromTrs = 2,
        HacFromToTrs = 3,
        SatToTrs = 2,
        SatFromTrs = 2,
        SatFromToTrs = 3,
        DiaSingleTrs = 2,
        DiaToTrs = 2,
        DiaFromTrs = 2,
        DiaFromToTrs = 3,
        AssetToTrs = 2,
        AssetFromTrs = 2,
        AssetFromToTrs = 3,
    )?;

    // Host ids = KIND low byte (mainnet-compatible ACTENV / ACTVIEW idx).
    register_vm_hosts!(reg, env;
        EnvHeight = U64,
        EnvMainAddr = Address,
        EnvBlockAuthorAddr = Address,
    )?;

    register_vm_hosts!(reg, view;
        ViewBalance = (Bytes, 1),
        ViewAssetBalance = (U64, 2),
        ViewCheckSign = (Bool, 1),
        ViewDiaInscNum = (U8, 1),
        ViewDiaInscGet = (Bytes, 2),
        ViewDiaNameList = (Bytes, 3),
        ViewDiaOwnerAddrs = (Bytes, 1),
    )?;
    Ok(())
}

/// Install the standard wire codec set: transaction codecs and every action
/// codec (binary + JSON + schema + friendly family). Execution-only
/// registrations (profile, VM params, block creator, context creator, VM host
/// defs) live in `register_exec`; the SDK/wasm path calls only this function,
/// so execution never enters its dependency graph.
pub fn register_wire(reg: &mut dyn WireRegistry) -> Rerr {
    // Type 0 is CoinbaseTx (registered by mint); do not register DefaultPreludeTx.
    reg.register_tx(TransactionType1::TYPE, create_transaction_type1)?;
    reg.register_tx(TransactionType2::TYPE, create_transaction_type2)?;
    reg.register_tx(TransactionType3::TYPE, create_transaction_type3)?;
    base::register_regular_actions!(
        reg,
        "hac_transfer", create_hac_transfer => [HacToTrs, HacFromTrs, HacFromToTrs],
        "sat_transfer", create_sat_transfer => [SatToTrs, SatFromTrs, SatFromToTrs],
        "asset_transfer", create_asset_transfer => [AssetToTrs, AssetFromTrs, AssetFromToTrs],
        // tx_message / tx_blob are separate friendly variants, so the shared
        // blob decoder is registered as two single-kind groups.
        "tx_message", create_blob_action => [TxMessage],
        "tx_blob", create_blob_action => [TxBlob],
        // chain_allow / height_scope have friendly forms; balance_floor does
        // not ("" skips the family registration).
        "chain_allow", create_chain_guard_action => [ChainAllow],
        "height_scope", create_chain_guard_action => [HeightScope],
        "", create_chain_guard_action => [BalanceFloor],
        "", create_envfunc_action => [
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
        "hacd_transfer",
        create_diamond_transfer,
        decode_diamond_transfer_json => [DiaSingleTrs, DiaFromToTrs, DiaToTrs, DiaFromTrs],
    )?;
    base::register_custom_actions!(
        reg,
        "req_sign_list",
        create_chain_guard_action,
        decode_req_sign_list_json => [ReqSignList],
    )?;
    base::register_custom_actions!(reg, "", create_ast_select, decode_ast_select_json => [AstSelect])?;
    base::register_custom_actions!(reg, "", create_ast_if, decode_ast_if_json => [AstIf])?;
    base::register_custom_actions!(
        reg,
        "",
        create_tex_cell_act,
        decode_tex_cell_act_json => [TexCellAct],
    )?;
    Ok(())
}

/// Install the standard execution services on top of `register_wire`'s codec
/// set: protocol profile, VM params, block creator, context creator and VM
/// host capability metadata. Called by the full node composition root only
/// (its context creator lives in the gated `exec` module).
#[cfg(feature = "execute")]
pub fn register_exec(reg: &mut dyn ExecRegistry, params: &'static crate::ProtocolParams) -> Rerr {
    reg.set_execution_profile(params)?;
    reg.set_vm_params(params.vm)?;
    reg.set_block_creator(create_std_block)?;
    reg.set_context_creator(create_context, base::DEFAULT_GAS_BUDGET)?;
    register_vm_host_defs(reg)?;
    Ok(())
}

/// Full standard registration (wire + exec) for composition roots that want
/// the whole surface from one entry.
#[cfg(feature = "execute")]
pub fn register_standard(
    reg: &mut dyn base::RegistryWriter,
    params: &'static crate::ProtocolParams,
) -> Rerr {
    register_wire(reg)?;
    register_exec(reg, params)?;
    Ok(())
}

/////////////////////////////////

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Minimal `RegistryWriter` capturing only VM host defs; the other
    /// registration surfaces are unused by `register_vm_host_defs` and error
    /// out if touched.
    struct TestReg {
        host_defs: HashMap<(VmHostCallKind, u8), VmHostActionDef>,
    }

    impl TestReg {
        fn host(&self, kind: VmHostCallKind, id: u8) -> Option<&VmHostActionDef> {
            self.host_defs.get(&(kind, id))
        }
    }

    impl ExecRegistry for TestReg {
        fn set_block_creator(&mut self, _f: base::BlockCreateFn) -> Rerr {
            errf!("unexpected set_block_creator")
        }
        fn set_block_sizer(&mut self, _f: base::BlockSizeFn) -> Rerr {
            errf!("unexpected set_block_sizer")
        }
        fn set_vm_assigner(&mut self, _f: base::VmAssignFn) -> Rerr {
            errf!("unexpected set_vm_assigner")
        }
        fn register_vm_host_def(&mut self, def: VmHostActionDef) -> Rerr {
            def.validate_opcode_abi()?;
            let key = (def.kind, def.id);
            if self.host_defs.insert(key, def).is_some() {
                return errf!("vm host {:?}/{} already registered", key.0, key.1);
            }
            Ok(())
        }
        fn set_context_creator(&mut self, _f: base::ContextCreateFn, _gas_budget: i64) -> Rerr {
            errf!("unexpected set_context_creator")
        }
        fn set_vm_params(&mut self, _params: base::VmExecutionParams) -> Rerr {
            errf!("unexpected set_vm_params")
        }
        fn set_execution_profile(
            &mut self,
            _profile: &'static (dyn std::any::Any + Send + Sync),
        ) -> Rerr {
            errf!("unexpected set_execution_profile")
        }
    }

    fn registered() -> TestReg {
        let mut reg = TestReg {
            host_defs: HashMap::new(),
        };
        register_vm_host_defs(&mut reg).expect("register vm host defs");
        reg
    }

    /// Every registered host name equals the owning action type's `NAME`, and
    /// the registered opcode id is the full KIND low byte.
    #[test]
    fn host_names_match_the_action_name_constants() {
        let reg = registered();
        for (kind, name, argc) in [
            (HacToTrs::KIND, HacToTrs::NAME, 2),
            (HacFromTrs::KIND, HacFromTrs::NAME, 2),
            (HacFromToTrs::KIND, HacFromToTrs::NAME, 3),
            (SatToTrs::KIND, SatToTrs::NAME, 2),
            (SatFromTrs::KIND, SatFromTrs::NAME, 2),
            (SatFromToTrs::KIND, SatFromToTrs::NAME, 3),
            (DiaSingleTrs::KIND, DiaSingleTrs::NAME, 2),
            (DiaToTrs::KIND, DiaToTrs::NAME, 2),
            (DiaFromTrs::KIND, DiaFromTrs::NAME, 2),
            (DiaFromToTrs::KIND, DiaFromToTrs::NAME, 3),
            (AssetToTrs::KIND, AssetToTrs::NAME, 2),
            (AssetFromTrs::KIND, AssetFromTrs::NAME, 2),
            (AssetFromToTrs::KIND, AssetFromToTrs::NAME, 3),
        ] {
            let def = reg.host(VmHostCallKind::Action, kind as u8).unwrap();
            assert_eq!(def.name, name);
            assert_eq!(def.argc, argc);
        }
        for (kind, name, ret) in [
            (EnvHeight::KIND, EnvHeight::NAME, VmValueType::U64),
            (EnvMainAddr::KIND, EnvMainAddr::NAME, VmValueType::Address),
            (
                EnvBlockAuthorAddr::KIND,
                EnvBlockAuthorAddr::NAME,
                VmValueType::Address,
            ),
        ] {
            let def = reg.host(VmHostCallKind::Env, kind as u8).unwrap();
            assert_eq!(def.name, name);
            assert_eq!(def.ret, ret);
        }
        for (kind, name, ret, argc) in [
            (ViewBalance::KIND, ViewBalance::NAME, VmValueType::Bytes, 1),
            (
                ViewAssetBalance::KIND,
                ViewAssetBalance::NAME,
                VmValueType::U64,
                2,
            ),
            (
                ViewCheckSign::KIND,
                ViewCheckSign::NAME,
                VmValueType::Bool,
                1,
            ),
            (
                ViewDiaInscNum::KIND,
                ViewDiaInscNum::NAME,
                VmValueType::U8,
                1,
            ),
            (
                ViewDiaInscGet::KIND,
                ViewDiaInscGet::NAME,
                VmValueType::Bytes,
                2,
            ),
            (
                ViewDiaNameList::KIND,
                ViewDiaNameList::NAME,
                VmValueType::Bytes,
                3,
            ),
            (
                ViewDiaOwnerAddrs::KIND,
                ViewDiaOwnerAddrs::NAME,
                VmValueType::Bytes,
                1,
            ),
        ] {
            let def = reg.host(VmHostCallKind::View, kind as u8).unwrap();
            assert_eq!(def.name, name);
            assert_eq!(def.ret, ret);
            assert_eq!(def.argc, argc);
        }
    }

    /// ENV / VIEW full KIND high byte is the ACTENV / ACTVIEW opcode prefix.
    #[test]
    fn env_view_full_kind_matches_the_opcode_prefix() {
        for kind in [EnvHeight::KIND, EnvMainAddr::KIND, EnvBlockAuthorAddr::KIND] {
            assert_eq!(kind >> 8, 0x07);
        }
        for kind in [
            ViewBalance::KIND,
            ViewAssetBalance::KIND,
            ViewCheckSign::KIND,
            ViewDiaInscNum::KIND,
            ViewDiaInscGet::KIND,
            ViewDiaNameList::KIND,
            ViewDiaOwnerAddrs::KIND,
        ] {
            assert_eq!(kind >> 8, 0x06);
        }
    }

    /// An ACTION kind that does not fit the u8 opcode id is rejected instead of
    /// silently truncating.
    #[test]
    fn action_kind_is_rejected_when_it_would_truncate() {
        let mut reg = TestReg {
            host_defs: HashMap::new(),
        };
        assert!(register_action_def(&mut reg, 0x0100, "overflow", 0).is_err());
        // The 0xff boundary still fits an u8 id.
        register_action_def(&mut reg, 0x00ff, "max_kind", 0).expect("0xff action kind fits");
    }

    /// ENV / VIEW kinds outside their opcode space are rejected.
    #[test]
    fn env_view_kinds_are_rejected_outside_their_opcode_space() {
        let mut reg = TestReg {
            host_defs: HashMap::new(),
        };
        assert!(register_env_def(&mut reg, 0x0601, "view_kind_as_env", VmValueType::U64).is_err());
        assert!(
            register_view_def(&mut reg, 0x0701, "env_kind_as_view", VmValueType::U64, 0).is_err()
        );
    }

    /// The mainnet-compatible ACTENV / ACTVIEW id -> name mapping is unchanged.
    #[test]
    fn legacy_capability_id_name_mapping_is_preserved() {
        let reg = registered();
        let name = |k: VmHostCallKind, id: u8| reg.host(k, id).map(|d| d.name);
        assert_eq!(name(VmHostCallKind::Action, 1), Some(HacToTrs::NAME));
        assert_eq!(name(VmHostCallKind::Env, 1), Some("block_height"));
        assert_eq!(name(VmHostCallKind::Env, 2), Some("tx_main_addr"));
        assert_eq!(name(VmHostCallKind::Env, 3), Some("block_author_addr"));
        assert_eq!(name(VmHostCallKind::View, 1), Some("balance"));
        assert_eq!(name(VmHostCallKind::View, 2), Some("asset_balance"));
        assert_eq!(name(VmHostCallKind::View, 9), Some("check_signature"));
        assert_eq!(name(VmHostCallKind::View, 17), Some("hacd_insc_num"));
        assert_eq!(name(VmHostCallKind::View, 18), Some("hacd_insc_get"));
        assert_eq!(name(VmHostCallKind::View, 19), Some("hacd_name_list"));
        assert_eq!(name(VmHostCallKind::View, 20), Some("hacd_owner_addrs"));
    }
}
