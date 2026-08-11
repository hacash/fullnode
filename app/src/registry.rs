//! Application-owned component registry and standard Hacash assembly.

use std::collections::HashMap;
use std::sync::Arc;

use base::*;
use field::{Decode, Uint1, Uint2};
use sys::{Ret, decodef};

/// Protocol rules selected by this Hacash application.
pub static CHAIN_PROTOCOL_PARAMS: protocol::ProtocolParams = protocol::PROTOCOL_PARAMS;

pub struct Registry {
    block_hasher: BlockHasherFn,
    block_creator: Option<BlockCreateFn>,
    block_sizer: Option<BlockSizeFn>,
    vm_assigner: Option<VmAssignFn>,
    tx_codecs: HashMap<u8, TxCreateFn>,
    tx_json_codecs: HashMap<u8, TxJsonDecodeFn>,
    action_codecs: HashMap<u16, ActionCreateFn>,
    action_json_codecs: HashMap<u16, ActionJsonDecodeFn>,
    vm_host_defs: HashMap<(VmHostCallKind, u8), VmHostActionDef>,
    context_creator: Option<ContextCreateFn>,
    context_gas_budget: i64,
    vm_params: Option<VmExecutionParams>,
    execution_profile: Option<&'static (dyn std::any::Any + Send + Sync)>,
}

impl Registry {
    pub fn new(block_hasher: BlockHasherFn) -> Self {
        Self {
            block_hasher,
            block_creator: None,
            block_sizer: None,
            vm_assigner: None,
            tx_codecs: HashMap::new(),
            tx_json_codecs: HashMap::new(),
            action_codecs: HashMap::new(),
            action_json_codecs: HashMap::new(),
            vm_host_defs: HashMap::new(),
            context_creator: None,
            context_gas_budget: 0,
            vm_params: None,
            execution_profile: None,
        }
    }
}

impl RegistryWriter for Registry {
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

    fn register_tx(&mut self, ty: u8, f: TxCreateFn) -> sys::Rerr {
        if self.tx_codecs.contains_key(&ty) {
            return sys::errf!("transaction type {} already registered", ty);
        }
        self.tx_codecs.insert(ty, f);
        Ok(())
    }

    fn register_tx_json(&mut self, ty: u8, f: TxJsonDecodeFn) -> sys::Rerr {
        if self.tx_json_codecs.contains_key(&ty) {
            return sys::errf!("transaction json type {} already registered", ty);
        }
        self.tx_json_codecs.insert(ty, f);
        Ok(())
    }

    fn register_action(&mut self, kinds: &[u16], f: ActionCreateFn) -> sys::Rerr {
        if let Some(kind) = kinds.iter().find(|k| self.action_codecs.contains_key(k)) {
            return sys::errf!("action kind {} already registered", kind);
        }
        for kind in kinds {
            self.action_codecs.insert(*kind, f);
        }
        Ok(())
    }

    fn register_action_json(&mut self, kinds: &[u16], f: ActionJsonDecodeFn) -> sys::Rerr {
        if let Some(kind) = kinds
            .iter()
            .find(|k| self.action_json_codecs.contains_key(k))
        {
            return sys::errf!("action json kind {} already registered", kind);
        }
        for kind in kinds {
            self.action_json_codecs.insert(*kind, f);
        }
        Ok(())
    }

    fn register_vm_host_def(&mut self, def: VmHostActionDef) -> sys::Rerr {
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

    fn set_execution_profile(
        &mut self,
        profile: &'static (dyn std::any::Any + Send + Sync),
    ) -> sys::Rerr {
        if self.execution_profile.is_some() {
            return sys::errf!("execution profile already registered");
        }
        self.execution_profile = Some(profile);
        Ok(())
    }
}

impl BinaryCodecs for Registry {
    fn decode_action(&self, buf: &[u8]) -> Ret<(ActionRef, usize)> {
        let (kind, _) = Uint2::decode(buf)?;
        let kind = kind.uint();
        match self.action_codecs.get(&kind) {
            Some(codec) => codec(self, kind, buf),
            None => decodef!("action kind {} not registered", kind),
        }
    }

    fn decode_action_exact(&self, buf: &[u8]) -> Ret<ActionRef> {
        let (obj, used) = self.decode_action(buf)?;
        if used != buf.len() {
            return decodef!(
                "action parse length mismatch: consumed {} but payload length is {}",
                used,
                buf.len()
            );
        }
        Ok(obj)
    }
    fn decode_transaction(&self, buf: &[u8]) -> Ret<(TxRef, usize)> {
        let (ty, _) = Uint1::decode(buf)?;
        let ty = ty.uint();
        match self.tx_codecs.get(&ty) {
            Some(codec) => codec(self, buf),
            None => decodef!("transaction type {} not registered", ty),
        }
    }

    fn decode_transaction_exact(&self, buf: &[u8]) -> Ret<TxRef> {
        let (obj, used) = self.decode_transaction(buf)?;
        if used != buf.len() {
            return decodef!(
                "transaction parse length mismatch: consumed {} but payload length is {}",
                used,
                buf.len()
            );
        }
        Ok(obj)
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
            None => decodef!("block creator not registered"),
        }
    }

    fn decode_block_exact(&self, buf: &[u8]) -> Ret<BlockRef> {
        let (obj, used) = self.decode_block(buf)?;
        if used != buf.len() {
            return decodef!(
                "block parse length mismatch: consumed {} but payload length is {}",
                used,
                buf.len()
            );
        }
        Ok(obj)
    }

    fn peek_block_size(&self, buf: &[u8]) -> Ret<usize> {
        match self.block_sizer {
            Some(sizer) => sizer(self, buf),
            None => self.decode_block(buf).map(|(_, used)| used),
        }
    }
}

impl JsonCodecs for Registry {
    fn decode_tx_json(&self, ty: u8, json: &str) -> Ret<Option<TxRef>> {
        match self.tx_json_codecs.get(&ty) {
            Some(codec) => codec(self, ty, json).map(Some),
            None => Ok(None),
        }
    }

    fn decode_action_json(&self, kind: u16, json: &str) -> Ret<Option<ActionRef>> {
        match self.action_json_codecs.get(&kind) {
            Some(codec) => codec(self, kind, json).map(Some),
            None => Ok(None),
        }
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

    fn execution_profile(&self) -> Ret<&'static (dyn std::any::Any + Send + Sync)> {
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
    protocol::register_standard(&mut registry, &CHAIN_PROTOCOL_PARAMS)?;
    mint::register(&mut registry)?;
    vm::register(&mut registry)?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    static CUSTOM_CONTEXT_CALLED: AtomicBool = AtomicBool::new(false);

    struct EmptyState;

    impl base::DiskDB for EmptyState {
        fn read(&self, _key: &[u8]) -> Option<Vec<u8>> {
            None
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
            &CHAIN_PROTOCOL_PARAMS
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
            CHAIN_PROTOCOL_PARAMS.diamond_form_flag
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
