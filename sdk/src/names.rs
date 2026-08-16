//! Stable numeric-kind → display-name table for every registered action.
//!
//! The wire kind is the protocol fact; the name is a stable display identity
//! and is never used as a wire identity (doc 14 §4.6). Kinds without a table
//! entry still decode and inspect; only `name` is omitted.

use protocol::action_std::*;
use vm::action::*;

/// (kind, name) pairs sourced from the `name:` literals of each action's
/// `impl_action!`/`ActionCodec` registration.
const NAME_TABLE: &[(u16, &str)] = &[
    (HacToTrs::KIND, "transfer_hac_to"),
    (HacFromTrs::KIND, "transfer_hac_from"),
    (HacFromToTrs::KIND, "transfer_hac_from_to"),
    (SatToTrs::KIND, "transfer_sat_to"),
    (SatFromTrs::KIND, "transfer_sat_from"),
    (SatFromToTrs::KIND, "transfer_sat_from_to"),
    (DiaSingleTrs::KIND, "transfer_hacd_single_to"),
    (DiaToTrs::KIND, "transfer_hacd_to"),
    (DiaFromTrs::KIND, "transfer_hacd_from"),
    (DiaFromToTrs::KIND, "transfer_hacd_from_to"),
    (AssetToTrs::KIND, "transfer_asset_to"),
    (AssetFromTrs::KIND, "transfer_asset_from"),
    (AssetFromToTrs::KIND, "transfer_asset_from_to"),
    (HeightScope::KIND, "height_scope"),
    (ChainAllow::KIND, "chain_allow"),
    (BalanceFloor::KIND, "balance_floor"),
    (ReqSignList::KIND, "req_sign_list"),
    (TxMessage::KIND, "tx_message"),
    (TxBlob::KIND, "tx_blob"),
    (AstIf::KIND, "ast_if"),
    (AstSelect::KIND, "ast_select"),
    (TexCellAct::KIND, "tex_cell_act"),
    (EnvHeight::KIND, "block_height"),
    (EnvMainAddr::KIND, "tx_main_addr"),
    (EnvBlockAuthorAddr::KIND, "block_author_addr"),
    (ViewBalance::KIND, "balance"),
    (ViewAssetBalance::KIND, "asset_balance"),
    (ViewCheckSign::KIND, "check_signature"),
    (ViewDiaInscNum::KIND, "hacd_insc_num"),
    (ViewDiaInscGet::KIND, "hacd_insc_get"),
    (ViewDiaNameList::KIND, "hacd_name_list"),
    (ViewDiaOwnerAddrs::KIND, "hacd_owner_addrs"),
    (ContractDeploy::KIND, "contract_deploy"),
    (ContractUpdate::KIND, "contract_update"),
    (ContractMainCall::KIND, "contract_main_call"),
    (P2SHScriptProve::KIND, "p2sh_script_prove"),
];

pub fn action_name(kind: u16) -> Option<&'static str> {
    NAME_TABLE
        .iter()
        .find(|(registered, _)| *registered == kind)
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_no_duplicate_kinds() {
        let mut kinds: Vec<u16> = NAME_TABLE.iter().map(|(kind, _)| *kind).collect();
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
        assert_eq!(action_name(HeightScope::KIND), Some("height_scope"));
        assert_eq!(action_name(ContractMainCall::KIND), Some("contract_main_call"));
    }
}
