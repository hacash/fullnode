//! Asset creation action (kind 16, moved from mint; execution body gated by the `execute` feature).

use field::descfmt::user_text;
use field::{Amount, AssetSmelt};

base::action_simple! { AssetCreate, 16, 2, TOP_UNIQUE, {
    metadata: AssetSmelt,
    protocol_cost: Amount
}, this, {
    description: {
        let meta = &this.metadata;
        let mut desc = format!(
            "Register Asset #{} {} (name {}, supply {} atoms, {} decimals, issuer {})",
            meta.serial.uint(),
            user_text(meta.ticket.as_ref()),
            user_text(meta.name.as_ref()),
            meta.supply.uint(),
            meta.decimal.uint(),
            meta.issuer.to_readable()
        );
        if this.protocol_cost.is_positive() {
            desc.push_str(&format!(
                "; pay {} HAC protocol fee",
                this.protocol_cost.to_hac_string()
            ));
        }
        desc
    }
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
