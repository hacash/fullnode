//! Asset creation action (kind 16, moved from mint; execution body gated by the `execute` feature).

use std::sync::Arc;

use base::ActionRef;
use field::{Amount, AssetSmelt, Decode, Uint2};
use sys::Ret;

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

pub fn create_asset_create(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (action, used) = AssetCreate::decode(buf)?;
    Ok((Arc::new(action), used))
}
