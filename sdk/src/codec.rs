//! SDK wire composition: one static selection over crate-owned catalogs.
//! The SDK does not share the fullnode's `register_wire` registry; it selects
//! wallet-reachable entries from the same tables (tx types 2/3, no CALL_ONLY
//! actions) — compatibility is that policy, not a second action list.
//! The selection rules themselves live in `selection`; this module only turns
//! the selected catalogs into the read-side decode table.

use std::sync::OnceLock;

use base::{
    ActionRef, BinaryCodecs, BlockHasherFn, BlockRef, HASH_SIZE, JsonCodecs, TxRef, WireCodecTable,
};
use sys::{Ret, normalf};

use crate::selection::{sdk_action_codecs, sdk_tx_codecs};

/// Transaction/action codec composition used by the WASM SDK: loads the
/// wallet-reachable subset of crate-owned static catalogs into a read-side table; no registration trait or execution surface enters the wasm graph.
pub(crate) struct SdkCodecs {
    table: WireCodecTable,
}

impl SdkCodecs {
    fn new() -> Self {
        Self {
            table: WireCodecTable::new(),
        }
    }

    fn standard() -> Ret<Self> {
        let mut codecs = Self::new();
        for binding in sdk_tx_codecs() {
            codecs.table.add_tx(*binding)?;
        }
        for binding in sdk_action_codecs() {
            codecs.table.add_action(*binding)?;
        }
        Ok(codecs)
    }

    pub fn registered_kinds(&self) -> Vec<u16> {
        self.table.action_kinds()
    }

    /// Registered transaction types (2/3 for the wallet SDK).
    pub fn registered_tx_types(&self) -> Vec<u8> {
        self.table.tx_types()
    }
}

pub(crate) fn standard_codecs() -> Ret<&'static SdkCodecs> {
    static CODECS: OnceLock<Ret<SdkCodecs>> = OnceLock::new();
    match CODECS.get_or_init(SdkCodecs::standard) {
        Ok(codecs) => Ok(codecs),
        Err(error) => Err(error.clone()),
    }
}

fn sdk_block_hash(_height: u64, stuff: &[u8]) -> [u8; HASH_SIZE] {
    sys::calculate_hash(stuff)
}

impl JsonCodecs for SdkCodecs {
    fn decode_action_json(&self, json: &str) -> Ret<ActionRef> {
        self.table.decode_action_json(self, json)
    }
}

impl BinaryCodecs for SdkCodecs {
    fn decode_action(&self, buf: &[u8]) -> Ret<(ActionRef, usize)> {
        self.table.decode_action(self, buf)
    }

    fn decode_transaction(&self, buf: &[u8]) -> Ret<(TxRef, usize)> {
        self.table.decode_transaction(self, buf)
    }

    fn decode_block(&self, _buf: &[u8]) -> Ret<(BlockRef, usize)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{action_schema_refs, action_schemas, struct_schemas};

    #[test]
    fn sdk_schema_set_is_unique_and_closed() {
        base::validate_schema_set(&action_schemas(), &struct_schemas())
            .expect("SDK schema set valid");
    }

    #[test]
    fn standard_protocol_and_vm_actions_are_registered() {
        let codecs = standard_codecs().unwrap();
        let types = codecs.registered_tx_types();
        assert!(types.contains(&hacash_params::TX_TYPE_2));
        assert!(types.contains(&hacash_params::TX_TYPE_3));
        for kind in [
            vm::action::ContractDeploy::KIND,
            vm::action::ContractUpdate::KIND,
            vm::action::ContractMainCall::KIND,
            vm::action::P2SHScriptProve::KIND,
        ] {
            assert!(
                codecs.registered_kinds().contains(&kind),
                "vm action kind {kind} must be registered"
            );
        }
    }

    #[test]
    fn loaded_codecs_match_the_static_selection() {
        let codecs = standard_codecs().unwrap();
        let selected: Vec<_> = action_schema_refs().map(|schema| schema.kind).collect();
        assert_eq!(codecs.registered_kinds(), {
            let mut kinds = selected.clone();
            kinds.sort_unstable();
            kinds
        });
        for kind in selected {
            assert!(
                codecs.registered_kinds().contains(&kind),
                "kind drift in SDK codec surface: {kind}"
            );
        }
    }
}
