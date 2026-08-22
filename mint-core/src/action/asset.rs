//! Asset creation action (kind 16, moved from mint; execution body gated by the `execute` feature).

use field::{Amount, AssetSmelt, Uint2};

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct AssetCreate {
    pub kind: Uint2,
    pub metadata: AssetSmelt,
    pub protocol_cost: Amount,
}

impl AssetCreate {
    pub const KIND: u16 = 16;

    pub fn new(metadata: AssetSmelt, protocol_cost: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            metadata,
            protocol_cost,
        }
    }
}

base::impl_action_facts! {
    AssetCreate {
        name: "asset_create",
        scope: base::ActScope::TOP_ONLY,
        min_tx_type: 2,
        description: |this: &AssetCreate| format!("Register asset <{}>", this.metadata.ticket.to_readable_or_hex()),

    }
}
