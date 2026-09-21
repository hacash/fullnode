//! Asset creation action (kind 16, moved from mint; execution body gated by the `execute` feature).

use field::{Amount, AssetSmelt};

base::action_simple! { AssetCreate, 16, 2, TOP_UNIQUE, {
    metadata: AssetSmelt,
    protocol_cost: Amount
}, this, {
    description: format!("Register asset <{}>", this.metadata.ticket.to_readable_or_hex())
}}

#[cfg(test)]
mod scope_tests {
    use base::{ActScope, Action};

    use super::AssetCreate;

    // Mirrored by the local stub in `protocol::level`'s topology tests, which cannot
    // depend on this crate; this assertion is what keeps the two in step.
    //
    // OPEN QUESTION, raised rather than settled: AssetCreate is TOP_UNIQUE, not TOP.
    // Uniqueness is the conservative reading (one asset registration per tx) and the
    // only combination it forbids is `[AssetCreate, AssetCreate, ...]`. Deploy moved
    // to TOP precisely because a factory needs that batch, so if asset registration
    // ever needs the same batch shape, this is the line to change.
    #[test]
    fn asset_create_is_top_unique() {
        assert_eq!(AssetCreate::KIND, 16);
        assert_eq!(AssetCreate::SCOPE, ActScope::TOP_UNIQUE);
        let create = AssetCreate::default();
        assert_eq!(create.scope(), ActScope::TOP_UNIQUE);
        assert_eq!(create.min_tx_type(), 2);
        assert_eq!(create.required_flags(), 0);
    }
}
