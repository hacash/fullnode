//! SDK selection policy: wallet-reachable tx / action codecs (`sdk_tx_codecs` /
//! `sdk_action_codecs`).
//!
//! This is the single review entry point for what the SDK registers / supports.
//! The SDK does not share the fullnode's `register_wire` registry; it selects a
//! subset of each crate-owned static catalog (`protocol` / `mint_core` / `vm`)
//! by the two explicit rules below. Binding details (wire schema, decode
//! functions) always come from the catalogs themselves; this file only decides
//! which entries are selected, never re-defines a binding.
//!
//! Exclusion rules:
//! - **Transaction envelopes: exclude Type 1.** Type 1 envelopes are deprecated
//!   after mainnet height 33,033 (`type1_deprecated_after_height`); the catalog
//!   keeps them for historical / chain-internal compatibility. The wallet only
//!   builds the live envelopes Type 2 / Type 3.
//! - **Actions: exclude `ActScope::CALL_ONLY`.** CALL_ONLY actions (VM env/view
//!   syscalls such as `block_height`, `balance`, `hacd_insc_get`) can only be
//!   triggered by `action_call` inside a contract execution body; they can never
//!   appear as ordinary transaction actions. The wallet neither can nor should
//!   construct them. The rule reads the static `scope` field on each binding
//!   instead of guessing from kind arithmetic, so a future CALL_ONLY action in a
//!   different kind space is excluded correctly too.

use base::{ActionCodecBinding, ActionSchema, ActScope, StructSchema, TxCodecBinding};

/// Envelope rule: every `TX_CODECS` entry except Type 1 (deprecated).
fn is_sdk_tx_type(ty: u8) -> bool {
    ty != hacash_params::TX_TYPE_1
}

/// Action rule: every action whose scope is not `CALL_ONLY`.
fn is_sdk_action_kind(binding: &ActionCodecBinding) -> bool {
    binding.scope != ActScope::CALL_ONLY
}

/// Transaction codecs the SDK registers (Type 2 / Type 3).
pub(crate) fn sdk_tx_codecs() -> impl Iterator<Item = &'static TxCodecBinding> {
    protocol::TX_CODECS
        .iter()
        .filter(|binding| is_sdk_tx_type(binding.ty))
}

/// Action codecs the SDK registers: every catalog entry with scope != CALL_ONLY.
/// The rule applies uniformly to all three catalogs; today only `protocol`
/// contains CALL_ONLY entries, so chaining first and filtering once at the end
/// is equivalent to per-catalog filters.
pub(crate) fn sdk_action_codecs() -> impl Iterator<Item = &'static ActionCodecBinding> {
    protocol::ACTION_CODECS
        .iter()
        .chain(mint_core::ACTION_CODECS)
        .chain(vm::ACTION_CODECS)
        .filter(|binding| is_sdk_action_kind(binding))
}

pub(crate) fn action_schema_refs() -> impl Iterator<Item = &'static ActionSchema> {
    sdk_action_codecs().map(|binding| &binding.schema)
}

pub(crate) fn action_schema(kind: u16) -> Option<&'static ActionSchema> {
    action_schema_refs().find(|schema| schema.kind == kind)
}

pub(crate) fn action_schema_named(name: &str) -> Option<&'static ActionSchema> {
    action_schema_refs().find(|schema| schema.name == name)
}

pub(crate) fn action_schemas() -> Vec<ActionSchema> {
    action_schema_refs().copied().collect()
}

/// Nested struct schemas: the full union of the three crates' struct catalogs,
/// not filtered by the rules above (they are not actions, so wallet-reachability
/// does not apply).
pub(crate) fn struct_schema_refs() -> impl Iterator<Item = &'static StructSchema> {
    protocol::STRUCT_SCHEMAS
        .iter()
        .chain(mint_core::STRUCT_SCHEMAS.iter())
        .chain(vm::STRUCT_SCHEMAS.iter())
}

pub(crate) fn struct_schema_named(name: &str) -> Option<&'static StructSchema> {
    struct_schema_refs().find(|schema| schema.name == name)
}

pub(crate) fn struct_schemas() -> Vec<StructSchema> {
    struct_schema_refs().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type 1 is the only excluded envelope in the catalog; the SDK registers
    /// Type 2 / Type 3.
    #[test]
    fn sdk_tx_selection_excludes_the_deprecated_type1() {
        let selected: Vec<_> = sdk_tx_codecs().map(|binding| binding.ty).collect();
        assert_eq!(
            selected,
            vec![hacash_params::TX_TYPE_2, hacash_params::TX_TYPE_3]
        );
        // The exclusion lands only on Type 1: every other catalog envelope is selected.
        let excluded: Vec<_> = protocol::TX_CODECS
            .iter()
            .filter(|binding| !is_sdk_tx_type(binding.ty))
            .map(|binding| binding.ty)
            .collect();
        assert_eq!(excluded, vec![hacash_params::TX_TYPE_1]);
    }

    /// The excluded set is exactly the entries with scope == CALL_ONLY across
    /// all three catalogs — no more, no fewer.
    #[test]
    fn sdk_action_selection_excludes_call_only_scope() {
        let selected: Vec<u16> = sdk_action_codecs()
            .map(|binding| binding.schema.kind)
            .collect();
        assert_eq!(selected.len(), 35, "SDK capability profile changed");

        let mut call_only: Vec<u16> = protocol::ACTION_CODECS
            .iter()
            .chain(mint_core::ACTION_CODECS)
            .chain(vm::ACTION_CODECS)
            .filter(|binding| binding.scope == ActScope::CALL_ONLY)
            .map(|binding| binding.schema.kind)
            .collect();
        call_only.sort_unstable();
        // Today the CALL_ONLY entries are exactly the VM env/view syscalls
        // (0x06xx / 0x07xx kind space).
        assert_eq!(
            call_only,
            vec![0x0601, 0x0602, 0x0609, 0x0611, 0x0612, 0x0613, 0x0614, 0x0701, 0x0702, 0x0703]
        );

        // Every selected action's scope is indeed not CALL_ONLY, and every
        // CALL_ONLY action is indeed excluded.
        for binding in sdk_action_codecs() {
            assert_ne!(binding.scope, ActScope::CALL_ONLY);
            assert!(!call_only.contains(&binding.schema.kind));
        }
        let selected_set: std::collections::HashSet<_> = selected.into_iter().collect();
        for kind in &call_only {
            assert!(!selected_set.contains(kind), "CALL_ONLY kind {kind:#06x} leaked into SDK");
        }
    }

    /// The rule is semantic (scope), not a kind-space coincidence: it would still
    /// hold if a CALL_ONLY action moved to a different kind space (asserted by
    /// scope above). This test additionally pins the current realization — the
    /// CALL_ONLY actions are exactly the env/view syscalls — for audit contrast.
    #[test]
    fn call_only_actions_are_the_envfunc_syscalls() {
        let mut names: Vec<&str> = protocol::ACTION_CODECS
            .iter()
            .filter(|binding| binding.scope == ActScope::CALL_ONLY)
            .map(|binding| binding.schema.name)
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "asset_balance", "balance", "block_author_addr", "block_height", "check_signature",
                "hacd_insc_get", "hacd_insc_num", "hacd_name_list", "hacd_owner_addrs",
                "tx_main_addr",
            ]
        );
    }
}
