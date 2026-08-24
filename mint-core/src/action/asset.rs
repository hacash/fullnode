//! Asset creation action (kind 16, moved from mint; execution body gated by the `execute` feature).

use field::{Amount, AssetSmelt};

base::action_simple! { AssetCreate, 16, 2, TOP_ONLY, {
    metadata: AssetSmelt,
    protocol_cost: Amount
}, this, {
    description: format!("Register asset <{}>", this.metadata.ticket.to_readable_or_hex())
}}
