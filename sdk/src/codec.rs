use std::collections::HashMap;
use std::sync::OnceLock;

use base::{
    ActionCreateFn, ActionJsonDecodeFn, ActionRef, BinaryCodecs, BlockHasherFn, BlockRef, HASH_SIZE,
    TxCreateFn, TxJsonDecodeFn, TxRef, WireRegistry,
};
use field::{Decode, Uint1, Uint2};
use sys::{Ret, errf, normalf};

/// Transaction/action codec composition used by the WASM SDK.
///
/// The action/tx codec set is assembled by `chain-codec::register_standard` —
/// the same entry the full node (`app::standard_registry`) and
/// `codec-schema-gen` use — so the SDK's codec surface is the chain's surface
/// by construction, with no hand-written action list. Only the wire surface is
/// implemented (`WireRegistry`): execution-only registrations (block
/// creator/sizer, VM assigner, context, VM params) are not part of this trait
/// and can never be pulled into the wasm dependency graph.
pub(crate) struct SdkCodecs {
    transactions: HashMap<u8, TxCreateFn>,
    actions: HashMap<u16, ActionCreateFn>,
    action_json: HashMap<u16, ActionJsonDecodeFn>,
    /// Action schemas captured by the registration macro (same source as
    /// `codec-schema-gen`; used by `spec_codec`'s TransactionSpec decoder, no
    /// hand-written action list).
    action_schemas: Vec<base::ActionSchema>,
}

impl SdkCodecs {
    fn new() -> Self {
        Self {
            transactions: HashMap::new(),
            actions: HashMap::new(),
            action_json: HashMap::new(),
            action_schemas: Vec::new(),
        }
    }

    fn standard() -> Ret<Self> {
        let mut codecs = Self::new();
        chain_codec::register_standard(&mut codecs)?;
        Ok(codecs)
    }

    pub fn registered_kinds(&self) -> Vec<u16> {
        let mut kinds: Vec<u16> = self.actions.keys().copied().collect();
        kinds.sort_unstable();
        kinds
    }

    /// Registered transaction types (1/2/3 for the standard chain; the
    /// block-level CoinbaseTx type 0 lives only in the full node).
    pub fn registered_tx_types(&self) -> Vec<u8> {
        let mut types: Vec<u8> = self.transactions.keys().copied().collect();
        types.sort_unstable();
        types
    }

    pub fn action_schemas(&self) -> &[base::ActionSchema] {
        &self.action_schemas
    }
}

pub(crate) fn standard_codecs() -> Ret<&'static SdkCodecs> {
    static CODECS: OnceLock<Ret<SdkCodecs>> = OnceLock::new();
    match CODECS.get_or_init(SdkCodecs::standard) {
        Ok(codecs) => Ok(codecs),
        Err(error) => Err(error.clone()),
    }
}

impl WireRegistry for SdkCodecs {
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
        for (index, kind) in kinds.iter().enumerate() {
            if kinds[..index].contains(kind) {
                return errf!("action kind {} listed more than once", kind);
            }
        }
        if let Some(kind) = kinds.iter().find(|kind| self.actions.contains_key(kind)) {
            return errf!("action kind {} already registered", kind);
        }
        for kind in kinds {
            self.actions.insert(*kind, creator);
        }
        Ok(())
    }

    fn register_action_schema(&mut self, schema: base::ActionSchema) -> sys::Rerr {
        if !self.actions.contains_key(&schema.kind) {
            return errf!(
                "schema {} ({}) registered without a binary action codec",
                schema.kind,
                schema.name
            );
        }
        if self
            .action_schemas
            .iter()
            .any(|known| known.kind == schema.kind || known.name == schema.name)
        {
            return errf!(
                "duplicate action schema {} ({})",
                schema.kind,
                schema.name
            );
        }
        self.action_schemas.push(schema);
        Ok(())
    }

    fn register_action_json(&mut self, kinds: &[u16], decoder: ActionJsonDecodeFn) -> sys::Rerr {
        for (index, kind) in kinds.iter().enumerate() {
            if kinds[..index].contains(kind) {
                return errf!("action json kind {} listed more than once", kind);
            }
            if self.action_json.contains_key(kind) {
                return errf!("action json kind {} already registered", kind);
            }
            if !self.actions.contains_key(kind) {
                return errf!("action json kind {} has no binary action codec", kind);
            }
        }
        for kind in kinds {
            self.action_json.insert(*kind, decoder);
        }
        Ok(())
    }

    fn register_action_family(&mut self, _friendly: &'static str, _kinds: &[u16]) -> sys::Rerr {
        // The friendly family surface is consumed by the codegen/profile
        // paths through `chain_codec::collect_action_families` (the same
        // registration entry); the runtime codec container does not use it.
        // Explicit accept, never a silent default.
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

    /// The SDK's codec surface must be exactly the chain-codec capture (the
    /// shared registration entry used by the full node and codec-schema-gen).
    /// If a new action crate is ever wired into the chain without going
    /// through `chain-codec::register_standard`, this test fails.
    #[test]
    fn sdk_codec_surface_matches_chain_codec() {
        let codecs = standard_codecs().unwrap();
        let captured = chain_codec::collect_action_schemas();
        assert_eq!(
            codecs.action_schemas().len(),
            captured.len(),
            "sdk action schema set must equal the chain-codec capture"
        );
        for (a, b) in codecs.action_schemas().iter().zip(&captured) {
            assert_eq!(a.kind, b.kind, "kind drift in chain codec surface");
            assert_eq!(a.name, b.name, "name drift in chain codec surface");
        }
    }
}
