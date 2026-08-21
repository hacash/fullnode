//! Application-owned component registry and standard Hacash assembly.

use std::collections::HashMap;
use std::sync::Arc;

use base::*;
use sys::{Ret, normalf};

pub struct Registry {
    block_hasher: BlockHasherFn,
    block_creator: Option<BlockCreateFn>,
    block_sizer: Option<BlockSizeFn>,
    vm_assigner: Option<VmAssignFn>,
    wire_codecs: WireCodecTable,
    vm_host_defs: HashMap<(VmHostCallKind, u8), VmHostActionDef>,
    context_creator: Option<ContextCreateFn>,
    context_gas_budget: i64,
    vm_params: Option<VmExecutionParams>,
    execution_profile: Option<&'static dyn ExecutionProfile>,
}

impl Registry {
    pub fn new(block_hasher: BlockHasherFn) -> Self {
        Self {
            block_hasher,
            block_creator: None,
            block_sizer: None,
            vm_assigner: None,
            wire_codecs: WireCodecTable::new(),
            vm_host_defs: HashMap::new(),
            context_creator: None,
            context_gas_budget: 0,
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

    fn set_block_sizer(&mut self, f: BlockSizeFn) -> sys::Rerr {
        if self.block_sizer.is_some() {
            return sys::errf!("block sizer already registered");
        }
        self.block_sizer = Some(f);
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

    fn set_context_creator(&mut self, f: ContextCreateFn, gas_budget: i64) -> sys::Rerr {
        if self.context_creator.is_some() {
            return sys::errf!("context creator already registered");
        }
        self.context_creator = Some(f);
        self.context_gas_budget = gas_budget;
        Ok(())
    }

    fn set_vm_params(&mut self, params: VmExecutionParams) -> sys::Rerr {
        if self.vm_params.is_some() {
            return sys::errf!("VM execution params already registered");
        }
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
        match self.block_sizer {
            Some(sizer) => sizer(self, buf),
            None => self.decode_block(buf).map(|(_, used)| used),
        }
    }
}

impl JsonCodecs for Registry {
    fn decode_action_json(&self, kind: u16, json: &str) -> Ret<Option<ActionRef>> {
        self.wire_codecs.decode_action_json(self, kind, json)
    }
}

impl ExecutionServices for Registry {
    fn assign_vm(&self, height: u64) -> Option<Box<dyn Vm>> {
        self.vm_assigner.map(|assign| assign(self, height))
    }

    fn vm_host_def(&self, kind: VmHostCallKind, id: u8) -> Option<&VmHostActionDef> {
        self.vm_host_defs.get(&(kind, id))
    }

    fn vm_host_defs(&self, kind: VmHostCallKind) -> Vec<&VmHostActionDef> {
        let mut defs: Vec<_> = self
            .vm_host_defs
            .values()
            .filter(|def| def.kind == kind)
            .collect();
        defs.sort_unstable_by_key(|def| (def.id, def.name));
        defs
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
        let gas_budget = self.context_gas_budget;
        match self.context_creator {
            Some(create) => create(env, self, chunk, tx, gas_budget),
            None => sys::errf!("context creator not registered"),
        }
    }
}

pub fn standard_registry() -> Ret<Registry> {
    let mut registry = Registry::new(mint::block_hasher);
    protocol::register_wire(&mut registry)?;
    mint_core::register_wire(&mut registry)?;
    vm::register_wire(&mut registry)?;
    protocol::register_exec(&mut registry, &hacash_params::MAINNET_PARAMS)?;
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
        _gas_budget: i64,
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

    #[test]
    fn standard_registry_has_the_consensus_codec_surface() {
        let registry = standard_registry().expect("standard registry");
        let registered = registry.wire_codecs.action_kinds();
        assert_eq!(
            registered,
            vec![
                1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 16, 17, 18, 19, 22, 25, 26, 32, 33, 34,
                35, 36, 40, 41, 44, 46, 0x0401, 0x0402, 0x0411, 0x0412, 0x0413, 0x0414, 0x0601,
                0x0602, 0x0609, 0x0611, 0x0612, 0x0613, 0x0614, 0x0701, 0x0702, 0x0703,
            ]
        );
        assert_eq!(registry.wire_codecs.tx_types(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn standard_registry_host_defs_match_the_action_name_constants() {
        let registry = standard_registry().expect("standard registry");
        let name = |k: VmHostCallKind, id: u8| registry.vm_host_def(k, id).map(|d| d.name);
        // protocol transfer EXTACTION hosts: id == kind, name == Type::NAME
        assert_eq!(
            name(VmHostCallKind::Action, 1),
            Some(protocol::action_std::HacToTrs::NAME)
        );
        assert_eq!(
            name(VmHostCallKind::Action, 10),
            Some(protocol::action_std::SatToTrs::NAME)
        );
        assert_eq!(
            name(VmHostCallKind::Action, 7),
            Some(protocol::action_std::DiaToTrs::NAME)
        );
        // mint inscription host
        assert_eq!(
            name(VmHostCallKind::Action, 34),
            Some(mint_core::inscription::DiaInscEdit::NAME)
        );
        // ACTENV / ACTVIEW hosts: id == KIND low byte, name == Type::NAME
        assert_eq!(
            name(VmHostCallKind::Env, 1),
            Some(protocol::action_std::EnvHeight::NAME)
        );
        assert_eq!(
            name(VmHostCallKind::Env, 2),
            Some(protocol::action_std::EnvMainAddr::NAME)
        );
        assert_eq!(
            name(VmHostCallKind::View, 18),
            Some(protocol::action_std::ViewDiaInscGet::NAME)
        );
        assert_eq!(
            name(VmHostCallKind::View, 20),
            Some(protocol::action_std::ViewDiaOwnerAddrs::NAME)
        );
    }

    #[test]
    fn standard_registry_decodes_regular_action_json() {
        use field::{Encode, ToJSON};

        let registry = standard_registry().expect("standard registry");
        let source =
            protocol::action_std::SatToTrs::new(field::Address::default(), field::Satoshi::from(7));
        let decoded = registry
            .decode_action_json(source.kind(), &source.to_json())
            .expect("json codec")
            .expect("registered action");
        assert_eq!(decoded.encode(), source.encode());
        assert!(
            registry
                .decode_action_json(
                    source.kind(),
                    "{\"kind\":10,\"to\":0,\"to\":0,\"satoshi\":7}"
                )
                .is_err()
        );
    }

    #[test]
    fn registry_json_keeps_dynamic_actions_registry_owned() {
        use field::ToJSON;
        use std::sync::Arc;

        let registry = standard_registry().expect("standard registry");
        let child =
            protocol::action_std::SatToTrs::new(field::Address::default(), field::Satoshi::from(1));
        let ast =
            protocol::action_std::AstSelect::create_by(0, 1, vec![Arc::new(child)]).expect("AST");
        let decoded = registry
            .decode_action_json(ast.kind(), &ast.to_json())
            .expect("AST JSON codec")
            .expect("registered AST action");
        assert_eq!(decoded.to_json(), ast.to_json());

        let signers = protocol::action_std::ReqSignList::create_by(vec![field::AddrOrPtr::Ptr(0)])
            .expect("signer list");
        let decoded = registry
            .decode_action_json(signers.kind(), &signers.to_json())
            .expect("ReqSignList JSON codec")
            .expect("registered ReqSignList action");
        assert_eq!(decoded.to_json(), signers.to_json());

        assert!(
            registry
                .decode_action_json(
                    protocol::action_std::DiaToTrs::KIND,
                    "{\"kind\":7,\"to\":0,\"diamonds\":[]}"
                )
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
            .set_context_creator(custom_context_creator, 0)
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
