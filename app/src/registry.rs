//! Application-owned component registry and standard Hacash assembly.

use std::collections::HashMap;
use std::sync::Arc;

use base::*;
use sys::{Ret, normalf};

pub struct Registry {
    block_hasher: BlockHasherFn,
    block_creator: Option<BlockCreateFn>,
    vm_assigner: Option<VmAssignFn>,
    wire_codecs: WireCodecTable,
    vm_host_defs: HashMap<(VmHostCallKind, u8), VmHostActionDef>,
    context_creator: Option<ContextCreateFn>,
    vm_params: Option<VmExecutionParams>,
    execution_profile: Option<&'static dyn ExecutionProfile>,
}

impl Registry {
    pub fn new(block_hasher: BlockHasherFn) -> Self {
        Self {
            block_hasher,
            block_creator: None,
            vm_assigner: None,
            wire_codecs: WireCodecTable::new(),
            vm_host_defs: HashMap::new(),
            context_creator: None,
            vm_params: None,
            execution_profile: None,
        }
    }
}

impl base::WireRegistry for Registry {
    fn register_tx_codec(&mut self, binding: TxCodecBinding) -> sys::Rerr {
        self.wire_codecs.add_tx(binding)
    }

    fn register_action_codec(&mut self, binding: ActionCodecBinding) -> sys::Rerr {
        self.wire_codecs.add_action(binding)
    }
}

impl base::ExecRegistry for Registry {
    fn set_block_creator(&mut self, f: BlockCreateFn) -> sys::Rerr {
        if self.block_creator.is_some() {
            return sys::errf!("block creator already registered");
        }
        self.block_creator = Some(f);
        Ok(())
    }

    fn set_vm_assigner(&mut self, f: VmAssignFn) -> sys::Rerr {
        if self.vm_assigner.is_some() {
            return sys::errf!("vm assigner already registered");
        }
        self.vm_assigner = Some(f);
        Ok(())
    }

    fn register_vm_host_def(&mut self, def: VmHostActionDef) -> sys::Rerr {
        def.validate_opcode_abi()?;
        let key = (def.kind, def.id);
        if self.vm_host_defs.contains_key(&key) {
            return sys::errf!("vm host {:?}/{} already registered", key.0, key.1);
        }
        self.vm_host_defs.insert(key, def);
        Ok(())
    }

    fn set_context_creator(&mut self, f: ContextCreateFn) -> sys::Rerr {
        if self.context_creator.is_some() {
            return sys::errf!("context creator already registered");
        }
        self.context_creator = Some(f);
        Ok(())
    }

    fn set_vm_params(&mut self, params: VmExecutionParams) -> sys::Rerr {
        if self.vm_params.is_some() {
            return sys::errf!("VM execution params already registered");
        }
        // A profile must not boot with an invalid contract storage discount table
        // (height/rate monotonicity, C = T×R, v1 curve, K_max > R).
        params.validate()?;
        self.vm_params = Some(params);
        Ok(())
    }

    fn set_execution_profile(&mut self, profile: &'static dyn ExecutionProfile) -> sys::Rerr {
        if self.execution_profile.is_some() {
            return sys::errf!("execution profile already registered");
        }
        self.execution_profile = Some(profile);
        Ok(())
    }
}

impl BinaryCodecs for Registry {
    fn decode_action(&self, buf: &[u8]) -> Ret<(ActionRef, usize)> {
        self.wire_codecs.decode_action(self, buf)
    }

    fn decode_transaction(&self, buf: &[u8]) -> Ret<(TxRef, usize)> {
        self.wire_codecs.decode_transaction(self, buf)
    }
    fn block_hash(&self, height: u64, stuff: &[u8]) -> [u8; HASH_SIZE] {
        (self.block_hasher)(height, stuff)
    }

    fn block_hasher_fn(&self) -> BlockHasherFn {
        self.block_hasher
    }
    fn decode_block(&self, buf: &[u8]) -> Ret<(BlockRef, usize)> {
        match self.block_creator {
            Some(creator) => creator(self, buf),
            None => normalf!("block creator not registered"),
        }
    }

    fn peek_block_size(&self, buf: &[u8]) -> Ret<usize> {
        self.decode_block(buf).map(|(_, used)| used)
    }
}

impl JsonCodecs for Registry {
    fn decode_action_json(&self, json: &str) -> Ret<ActionRef> {
        self.wire_codecs.decode_action_json(self, json)
    }
}

impl ExecutionServices for Registry {
    fn assign_vm(&self, height: u64) -> Option<Box<dyn Vm>> {
        self.vm_assigner.map(|assign| assign(self, height))
    }

    fn vm_host_def(&self, kind: VmHostCallKind, id: u8) -> Option<&VmHostActionDef> {
        self.vm_host_defs.get(&(kind, id))
    }

    fn vm_params(&self) -> Ret<&VmExecutionParams> {
        self.vm_params
            .as_ref()
            .ok_or_else(|| sys::Error::fault("VM execution params not registered"))
    }

    fn execution_profile(&self) -> Ret<&'static dyn ExecutionProfile> {
        self.execution_profile
            .ok_or_else(|| sys::Error::fault("execution profile not registered"))
    }
    fn create_context(
        self: Arc<Self>,
        env: Env,
        chunk: StateChunkRef,
        tx: TxRef,
    ) -> Ret<Box<dyn Context>> {
        chunk.validate_tx_identity(&tx.hash())?;
        match self.context_creator {
            Some(create) => create(env, self, chunk, tx),
            None => sys::errf!("context creator not registered"),
        }
    }
}

/// Standard mainnet assembly.
pub fn standard_registry() -> Ret<Registry> {
    standard_registry_with_params(&hacash_params::MAINNET_PARAMS)
}

/// Assembly with an explicit parameter profile. Side/test-chain nodes register
/// their own `HacashParams` (e.g. cheaper VM storage rent) and select the
/// matching consensus (`Consensus::mint_params`/reward curve read the
/// registered profile, see `mint::consensus::minter`).
pub fn standard_registry_with_params(
    params: &'static hacash_params::HacashParams,
) -> Ret<Registry> {
    let mut registry = Registry::new(mint::block_hasher);
    protocol::register_wire(&mut registry)?;
    mint_core::register_wire(&mut registry)?;
    vm::register_wire(&mut registry)?;
    protocol::register_exec(&mut registry, params)?;
    mint_core::register_exec(&mut registry)?;
    mint::register_wire(&mut registry)?;
    vm::register_exec(&mut registry)?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    static CUSTOM_CONTEXT_CALLED: AtomicBool = AtomicBool::new(false);

    struct EmptyState;

    fn host_def(kind: VmHostCallKind, ret: VmValueType, argc: usize) -> VmHostActionDef {
        VmHostActionDef {
            id: 1,
            name: "test_host",
            kind,
            ret,
            argc,
            allowed_policy: VmHostAllowedPolicy::Any,
        }
    }

    #[test]
    fn registry_rejects_host_defs_that_conflict_with_opcode_abi() {
        let mut registry = Registry::new(mint::block_hasher);
        assert!(
            registry
                .register_vm_host_def(host_def(VmHostCallKind::Action, VmValueType::U64, 0))
                .is_err()
        );
        assert!(
            registry
                .register_vm_host_def(host_def(VmHostCallKind::Env, VmValueType::U64, 1))
                .is_err()
        );
        registry
            .register_vm_host_def(host_def(VmHostCallKind::View, VmValueType::U64, 1))
            .expect("valid view host definition");
    }

    impl base::DiskDB for EmptyState {
        fn read(&self, _key: &[u8]) -> sys::Ret<Option<Vec<u8>>> {
            Ok(None)
        }
        fn save(&self, _key: &[u8], _val: &[u8]) {}
        fn remove(&self, _key: &[u8]) {}
        fn try_write(&self, _mem: &dyn base::MemDB) -> sys::Rerr {
            sys::errf!("empty test state is read-only")
        }
    }

    fn custom_context_creator(
        _env: Env,
        _registry: Arc<dyn ExecutionServices>,
        _chunk: StateChunkRef,
        _tx: TxRef,
    ) -> Ret<Box<dyn Context>> {
        CUSTOM_CONTEXT_CALLED.store(true, Ordering::SeqCst);
        sys::errf!("custom context creator called")
    }

    #[test]
    fn standard_registry_uses_selected_protocol_params() {
        let registry = standard_registry().expect("standard registry");
        assert_eq!(
            protocol::execution_params(&registry).expect("protocol params"),
            &hacash_params::MAINNET_PARAMS.protocol
        );
    }

    /// VM no longer depends on `hacash-params`; the injected `VmExecutionParams`
    /// must still decode the three engine limits and the tx cap through the
    /// same vocabulary the protocol profile owns.
    #[test]
    fn vm_gas_budget_limits_match_the_chain_vocabulary() {
        let vm = hacash_params::MAINNET_PARAMS.protocol.vm;
        let protocol = &hacash_params::MAINNET_PARAMS.protocol;
        let registry = standard_registry().expect("standard registry");
        let injected = *registry.vm_params().expect("vm params");
        assert_eq!(injected, vm);
        assert_eq!(vm.tx_gas_budget_cap_byte, protocol.tx_gas_budget_cap_byte);
        assert_eq!(vm.decode_gas_budget(vm.compute_limit_byte), 18009);
        assert_eq!(vm.decode_gas_budget(vm.resource_limit_byte), 6100);
        assert_eq!(vm.decode_gas_budget(vm.storage_limit_byte), 111911);
        assert_eq!(
            vm.decode_gas_budget(vm.tx_gas_budget_cap_byte),
            protocol.decode_gas_budget(protocol.tx_gas_budget_cap_byte)
        );
        for b in 0u8..=255 {
            assert_eq!(
                vm.decode_gas_budget(b),
                protocol.decode_gas_budget(b),
                "gas-budget byte {b} diverged between vm params and protocol table"
            );
        }
    }

    #[test]
    fn standard_registry_has_the_consensus_codec_surface() {
        let registry = standard_registry().expect("standard registry");
        let registered = registry.wire_codecs.action_kinds();
        assert_eq!(
            registered,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 16, 17, 18, 19, 22, 25, 26, 32, 33, 34,
                35, 36, 40, 41, 44, 46, 0x0401, 0x0402, 0x0411, 0x0412, 0x0413, 0x0414, 0x0601,
                0x0602, 0x0609, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615, 0x0616, 0x0617, 0x0701,
                0x0702, 0x0703, 0x0704, 0x0705,
            ]
        );
        assert_eq!(registry.wire_codecs.tx_types(), vec![0, 1, 2, 3]);
    }

    /// Every entry of the VM display tables (`vm::ACTION_*_DEFS` — the fitsh
    /// decompiler/parser's hand-written id -> name maps) must match the
    /// registered VM host def, and through it the action type's `NAME`
    /// constant. The VM cannot depend on protocol/mint, so this app-level
    /// cross-check is the mechanical lock that keeps the hardcoded display
    /// table in sync with action renames.
    #[test]
    fn vm_host_display_tables_match_the_registered_host_defs_exactly() {
        use vm::ValueTy;

        let registry = standard_registry().expect("standard registry");
        let host = |k: VmHostCallKind, id: u8| {
            registry
                .vm_host_def(k, id)
                .unwrap_or_else(|| panic!("vm host {k:?}/{id:#04x} not registered"))
        };
        let vm_ty = |t: ValueTy| match t {
            ValueTy::Nil => VmValueType::Nil,
            ValueTy::Bool => VmValueType::Bool,
            ValueTy::U8 => VmValueType::U8,
            ValueTy::U16 => VmValueType::U16,
            ValueTy::U64 => VmValueType::U64,
            ValueTy::Address => VmValueType::Address,
            ValueTy::Bytes => VmValueType::Bytes,
            other => panic!("vm display table uses non-host value type {other:?}"),
        };

        // EXTACTION hosts: the wire id is the action kind itself.
        for (id, name, ret, argc) in vm::ACTION_DEFS {
            let def = host(VmHostCallKind::Action, id);
            assert_eq!(def.id, id, "ACTION id mismatch");
            assert_eq!(def.name, name, "ACTION {id:#04x} display name drifted");
            assert_eq!(def.ret, vm_ty(ret), "ACTION {id:#04x} return type drifted");
            assert_eq!(def.argc, argc, "ACTION {id:#04x} arity drifted");
        }
        // ACTENV / ACTVIEW hosts: id is the low byte of the 0x07xx / 0x06xx kind.
        for (id, name, ret, argc) in vm::ACTION_ENV_DEFS {
            let def = host(VmHostCallKind::Env, id);
            assert_eq!(def.id, id, "ENV id mismatch");
            assert_eq!(def.name, name, "ENV {id:#04x} display name drifted");
            assert_eq!(def.ret, vm_ty(ret), "ENV {id:#04x} return type drifted");
            assert_eq!(def.argc, argc, "ENV {id:#04x} arity drifted");
        }
        for (id, name, ret, argc) in vm::ACTION_VIEW_DEFS {
            let def = host(VmHostCallKind::View, id);
            assert_eq!(def.id, id, "VIEW id mismatch");
            assert_eq!(def.name, name, "VIEW {id:#04x} display name drifted");
            assert_eq!(def.ret, vm_ty(ret), "VIEW {id:#04x} return type drifted");
            assert_eq!(def.argc, argc, "VIEW {id:#04x} arity drifted");
        }
    }

    /// Canonical (kind, name) snapshot of every wire action across the three
    /// catalogs. Renaming a struct (which re-derives the snake_case name) or
    /// touching an explicit `name` override updates this table consciously.
    #[test]
    fn action_name_snapshot_is_stable() {
        use base::ActionCodecBinding;
        let mut rows: Vec<(u16, &'static str)> = protocol::ACTION_CODECS
            .iter()
            .chain(mint_core::ACTION_CODECS.iter())
            .chain(vm::ACTION_CODECS.iter())
            .map(|b: &ActionCodecBinding| (b.schema.kind, b.schema.name))
            .collect();
        rows.sort_unstable();
        assert_eq!(
            rows,
            vec![
                (1, "transfer_hac_to"),
                (2, "channel_open"),
                (3, "channel_close"),
                (4, "hacd_mint"),
                (5, "transfer_hacd_single_to"),
                (6, "transfer_hacd_from_to"),
                (7, "transfer_hacd_to"),
                (8, "transfer_hacd_from"),
                (10, "transfer_sat_to"),
                (11, "transfer_sat_from"),
                (12, "transfer_sat_from_to"),
                (13, "transfer_hac_from"),
                (14, "transfer_hac_from_to"),
                (16, "asset_create"),
                (17, "transfer_asset_to"),
                (18, "transfer_asset_from"),
                (19, "transfer_asset_from_to"),
                (22, "tex_cell_execute"),
                (25, "ast_select"),
                (26, "ast_if"),
                (32, "hacd_insc_push"),
                (33, "hacd_insc_clean"),
                (34, "hacd_insc_edit"),
                (35, "hacd_insc_move"),
                (36, "hacd_insc_drop"),
                (40, "contract_deploy"),
                (41, "contract_update"),
                (44, "contract_main_call"),
                (46, "p2sh_script_prove"),
                (0x0401, "message"),
                (0x0402, "blob"),
                (0x0411, "chain_allow"),
                (0x0412, "height_scope"),
                (0x0413, "balance_floor"),
                (0x0414, "required_signers"),
                (0x0601, "balance_coin"),
                (0x0602, "balance_asset"),
                (0x0609, "check_signature"),
                (0x0611, "hacd_insc_num"),
                (0x0612, "hacd_insc_get"),
                (0x0613, "hacd_name_list"),
                (0x0614, "hacd_owner_addrs"),
                (0x0615, "tx_message"),
                (0x0616, "tx_blob"),
                (0x0617, "tx_blob_size"),
                (0x0701, "block_height"),
                (0x0702, "tx_main_addr"),
                (0x0703, "block_author_addr"),
                (0x0704, "tx_message_num"),
                (0x0705, "tx_blob_num"),
            ]
        );
    }

    #[test]
    fn call_scope_transfer_named_actions_are_exactly_the_thirteen() {
        use base::{ActionCodecBinding, ExecFrom};
        let mut rows: Vec<(u16, &'static str)> = protocol::ACTION_CODECS
            .iter()
            .filter(|b: &&ActionCodecBinding| {
                b.schema.name.starts_with("transfer_") && b.scope.allows(ExecFrom::Call)
            })
            .map(|b| (b.schema.kind, b.schema.name))
            .collect();
        rows.sort_unstable();
        assert_eq!(
            rows,
            vec![
                (1, "transfer_hac_to"),
                (5, "transfer_hacd_single_to"),
                (6, "transfer_hacd_from_to"),
                (7, "transfer_hacd_to"),
                (8, "transfer_hacd_from"),
                (10, "transfer_sat_to"),
                (11, "transfer_sat_from"),
                (12, "transfer_sat_from_to"),
                (13, "transfer_hac_from"),
                (14, "transfer_hac_from_to"),
                (17, "transfer_asset_to"),
                (18, "transfer_asset_from"),
                (19, "transfer_asset_from_to"),
            ]
        );
    }

    #[test]
    fn standard_registry_decodes_regular_action_json() {
        use field::{Encode, ToJSON};

        let registry = standard_registry().expect("standard registry");
        let source = protocol::action_std::TransferSatTo::new(
            field::Address::default(),
            field::Satoshi::from(7),
        );
        let decoded = registry
            .decode_action_json(&source.to_json())
            .expect("json codec");
        assert_eq!(decoded.encode(), source.encode());
        assert!(
            registry
                .decode_action_json("{\"kind\":10,\"to\":0,\"to\":0,\"satoshi\":7}")
                .is_err()
        );
    }

    #[test]
    fn registry_json_keeps_dynamic_actions_registry_owned() {
        use field::ToJSON;
        use std::sync::Arc;

        let registry = standard_registry().expect("standard registry");
        let child = protocol::action_std::TransferSatTo::new(
            field::Address::default(),
            field::Satoshi::from(1),
        );
        let ast =
            protocol::action_std::AstSelect::create_by(0, 1, vec![Arc::new(child)]).expect("AST");
        let decoded = registry
            .decode_action_json(&ast.to_json())
            .expect("AST JSON codec");
        assert_eq!(decoded.to_json(), ast.to_json());

        let signers =
            protocol::action_std::RequiredSigners::create_by(vec![field::AddrOrPtr::Ptr(0)])
                .expect("signer list");
        let decoded = registry
            .decode_action_json(&signers.to_json())
            .expect("RequiredSigners JSON codec");
        assert_eq!(decoded.to_json(), signers.to_json());

        assert!(
            registry
                .decode_action_json("{\"kind\":7,\"to\":0,\"diamonds\":[]}")
                .is_err()
        );
    }

    #[test]
    fn consensus_uses_registry_feature_flags() {
        let registry = standard_registry().expect("standard registry");
        let consensus = mint::HacashConsensus::with_config(
            &registry,
            mint::MintConf::default(),
            mint::MinerConf::default(),
        )
        .expect("consensus");
        assert_eq!(
            consensus.chain_flags(1),
            hacash_params::MAINNET_PARAMS.protocol.diamond_form_flag
        );
    }

    #[test]
    fn context_creation_rejects_wrong_tx_hash() {
        CUSTOM_CONTEXT_CALLED.store(false, Ordering::SeqCst);
        let mut registry = Registry::new(mint::block_hasher);
        registry
            .set_context_creator(custom_context_creator)
            .unwrap();
        let registry = Arc::new(registry);
        let tx: TxRef = Arc::new(protocol::tx_std::DefaultPreludeTx::default());
        let mut wrong_hash = tx.hash();
        wrong_hash.0[0] ^= 1;
        let root = StateChunkRef::block_draft_on_disk(Arc::new(EmptyState), 0);
        let chunk = StateChunkRef::tx_on(&root, wrong_hash);
        assert!(registry.create_context(Env::default(), chunk, tx).is_err());
        assert!(!CUSTOM_CONTEXT_CALLED.load(Ordering::SeqCst));
    }

    #[test]
    fn context_creation_rejects_non_tx_chunk() {
        let registry = Arc::new(standard_registry().expect("standard registry"));
        let tx: TxRef = Arc::new(protocol::tx_std::DefaultPreludeTx::default());
        let root = StateChunkRef::block_draft_on_disk(Arc::new(EmptyState), 0);
        let chunk = StateChunkRef::block_draft_on(&root, 1);
        assert!(registry.create_context(Env::default(), chunk, tx).is_err());
    }
}
