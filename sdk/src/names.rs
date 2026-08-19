//! Stable numeric-kind → display-name lookup for every registered action.
//!
//! The wire kind is the protocol fact; the name is a stable display identity
//! and is never used as a wire identity (doc 14 §4.6). Names come from the
//! schema capture of `standard_codecs()` (same registration macro as
//! `codec-schema-gen`, naturally the same source — new actions need no
//! registration here).

use crate::codec::standard_codecs;

/// (kind, name) lazy registry of the `ACTION_SCHEMA` captured during
/// registration assembly.
fn name_table() -> &'static [(u16, &'static str)] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<(u16, &'static str)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let codecs = standard_codecs().expect("standard codecs assembly");
        codecs
            .action_schemas()
            .iter()
            .map(|s| (s.kind, s.name))
            .collect()
    })
}

pub fn action_name(kind: u16) -> Option<&'static str> {
    name_table()
        .iter()
        .find(|(registered, _)| *registered == kind)
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mint_core::action::{
        asset::AssetCreate,
        channel::{ChannelClose, ChannelOpen},
        diamond::DiamondMint,
    };
    use protocol::action_std::{HacToTrs, TexCellAct};
    use vm::action::ContractDeploy;

    #[test]
    fn table_has_no_duplicate_kinds() {
        let mut kinds: Vec<u16> = name_table().iter().map(|(kind, _)| *kind).collect();
        kinds.sort_unstable();
        let deduped = {
            let mut out = kinds.clone();
            out.dedup();
            out
        };
        assert_eq!(kinds, deduped, "duplicate kind in name table");
    }

    #[test]
    fn known_kinds_resolve_names() {
        assert_eq!(action_name(HacToTrs::KIND), Some("transfer_hac_to"));
        assert_eq!(action_name(TexCellAct::KIND), Some("tex_cell_act"));
        assert_eq!(action_name(ContractDeploy::KIND), Some("contract_deploy"));
    }

    #[test]
    fn mint_actions_resolve_names() {
        assert_eq!(action_name(ChannelOpen::KIND), Some("channel_open"));
        assert_eq!(action_name(ChannelClose::KIND), Some("channel_close"));
        assert_eq!(action_name(AssetCreate::KIND), Some("asset_create"));
        assert_eq!(action_name(DiamondMint::KIND), Some("diamond_mint"));
    }
}
