use std::collections::HashMap;
use std::sync::OnceLock;

use base::{
    ActionCreateFn, ActionJsonDecodeFn, ActionRef, BinaryCodecs, BlockCreateFn, BlockHasherFn,
    BlockRef, BlockSizeFn, ContextCreateFn, HASH_SIZE, RegistryWriter, TxCreateFn, TxJsonDecodeFn,
    TxRef, VmAssignFn, VmExecutionParams, VmHostActionDef,
};
use field::{Decode, Uint1, Uint2};
use sys::{Ret, errf, normalf};

/// Transaction/action codec composition used by the WASM SDK.
///
/// The full-node registry lives in `app` and also pulls in consensus, storage,
/// VM and x16rs dependencies. The SDK only needs the standard protocol codecs
/// plus the four VM action codecs (ContractDeploy 40, ContractUpdate 41,
/// ContractMainCall 44, P2SHScriptProve 46), so it records those registrations
/// and deliberately ignores execution-only hooks (plan 13 §2, S1).
pub(crate) struct SdkCodecs {
    transactions: HashMap<u8, TxCreateFn>,
    actions: HashMap<u16, ActionCreateFn>,
    action_json: HashMap<u16, ActionJsonDecodeFn>,
}

impl SdkCodecs {
    fn new() -> Self {
        Self {
            transactions: HashMap::new(),
            actions: HashMap::new(),
            action_json: HashMap::new(),
        }
    }

    fn standard() -> Ret<Self> {
        let mut codecs = Self::new();
        protocol::register_standard(&mut codecs, &protocol::PROTOCOL_PARAMS)?;
        vm::action::register_actions(&mut codecs)?;
        Ok(codecs)
    }

    pub fn registered_kinds(&self) -> Vec<u16> {
        let mut kinds: Vec<u16> = self.actions.keys().copied().collect();
        kinds.sort_unstable();
        kinds
    }
}

pub(crate) fn standard_codecs() -> Ret<&'static SdkCodecs> {
    static CODECS: OnceLock<Ret<SdkCodecs>> = OnceLock::new();
    match CODECS.get_or_init(SdkCodecs::standard) {
        Ok(codecs) => Ok(codecs),
        Err(error) => Err(error.clone()),
    }
}

impl RegistryWriter for SdkCodecs {
    fn set_block_creator(&mut self, _creator: BlockCreateFn) -> sys::Rerr {
        Ok(())
    }

    fn set_block_sizer(&mut self, _sizer: BlockSizeFn) -> sys::Rerr {
        Ok(())
    }

    fn set_vm_assigner(&mut self, _assigner: VmAssignFn) -> sys::Rerr {
        Ok(())
    }

    fn register_tx(&mut self, ty: u8, creator: TxCreateFn) -> sys::Rerr {
        if self.transactions.contains_key(&ty) {
            return errf!("transaction type {} already registered", ty);
        }
        self.transactions.insert(ty, creator);
        Ok(())
    }

    fn register_tx_json(&mut self, _ty: u8, _decoder: TxJsonDecodeFn) -> sys::Rerr {
        Ok(())
    }

    fn register_action(&mut self, kinds: &[u16], creator: ActionCreateFn) -> sys::Rerr {
        if let Some(kind) = kinds.iter().find(|kind| self.actions.contains_key(kind)) {
            return errf!("action kind {} already registered", kind);
        }
        for kind in kinds {
            self.actions.insert(*kind, creator);
        }
        Ok(())
    }

    fn register_action_json(&mut self, kinds: &[u16], decoder: ActionJsonDecodeFn) -> sys::Rerr {
        for kind in kinds {
            self.action_json.insert(*kind, decoder);
        }
        Ok(())
    }

    fn register_vm_host_def(&mut self, _definition: VmHostActionDef) -> sys::Rerr {
        Ok(())
    }

    fn set_context_creator(&mut self, _creator: ContextCreateFn, _gas_budget: i64) -> sys::Rerr {
        Ok(())
    }

    fn set_vm_params(&mut self, _params: VmExecutionParams) -> sys::Rerr {
        Ok(())
    }

    fn set_execution_profile(
        &mut self,
        _profile: &'static (dyn std::any::Any + Send + Sync),
    ) -> sys::Rerr {
        Ok(())
    }
}

fn sdk_block_hash(_height: u64, stuff: &[u8]) -> [u8; HASH_SIZE] {
    sys::calculate_hash(stuff)
}

impl BinaryCodecs for SdkCodecs {
    fn decode_action(&self, buf: &[u8]) -> Ret<(ActionRef, usize)> {
        let (kind, _) = Uint2::decode(buf)?;
        let kind = kind.uint();
        match self.actions.get(&kind) {
            Some(creator) => creator(self, kind, buf),
            None => normalf!("action kind {} not registered in sdk", kind),
        }
    }

    fn decode_action_exact(&self, buf: &[u8]) -> Ret<ActionRef> {
        let (action, used) = self.decode_action(buf)?;
        if used != buf.len() {
            return normalf!(
                "action parse length mismatch: consumed {} but payload length is {}",
                used,
                buf.len()
            );
        }
        Ok(action)
    }

    fn decode_transaction(&self, buf: &[u8]) -> Ret<(TxRef, usize)> {
        let (ty, _) = Uint1::decode(buf)?;
        let ty = ty.uint();
        match self.transactions.get(&ty) {
            Some(creator) => creator(self, buf),
            None => normalf!("transaction type {} not registered in sdk", ty),
        }
    }

    fn decode_transaction_exact(&self, buf: &[u8]) -> Ret<TxRef> {
        let (transaction, used) = self.decode_transaction(buf)?;
        if used != buf.len() {
            return normalf!(
                "transaction parse length mismatch: consumed {} but payload length is {}",
                used,
                buf.len()
            );
        }
        Ok(transaction)
    }

    fn decode_block(&self, _buf: &[u8]) -> Ret<(BlockRef, usize)> {
        normalf!("block decoding is not part of the wasm sdk")
    }

    fn decode_block_exact(&self, _buf: &[u8]) -> Ret<BlockRef> {
        normalf!("block decoding is not part of the wasm sdk")
    }

    fn peek_block_size(&self, _buf: &[u8]) -> Ret<usize> {
        normalf!("block decoding is not part of the wasm sdk")
    }

    fn block_hash(&self, height: u64, stuff: &[u8]) -> [u8; HASH_SIZE] {
        sdk_block_hash(height, stuff)
    }

    fn block_hasher_fn(&self) -> BlockHasherFn {
        sdk_block_hash
    }
}

impl base::JsonCodecs for SdkCodecs {
    fn decode_tx_json(&self, _ty: u8, _json: &str) -> Ret<Option<TxRef>> {
        Ok(None)
    }

    fn decode_action_json(&self, kind: u16, json: &str) -> Ret<Option<ActionRef>> {
        match self.action_json.get(&kind) {
            Some(decoder) => decoder(self, kind, json).map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_protocol_and_vm_actions_are_registered() {
        let codecs = standard_codecs().unwrap();
        assert!(codecs.transactions.contains_key(&1));
        assert!(codecs.transactions.contains_key(&2));
        assert!(codecs.transactions.contains_key(&3));
        for kind in [40u16, 41, 44, 46] {
            assert!(
                codecs.actions.contains_key(&kind),
                "vm action kind {kind} must be registered"
            );
        }
    }
}
