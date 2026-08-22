//! Channel open/close action (kind 2/3, moved from mint; execution body gated by the `execute` feature).

use base::AddrOrPtr;
use field::{AddrHac, ChannelId, Uint2};

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ChannelOpen {
    pub kind: Uint2,
    pub channel_id: ChannelId,
    pub left_bill: AddrHac,
    pub right_bill: AddrHac,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ChannelClose {
    pub kind: Uint2,
    pub channel_id: ChannelId,
}

impl ChannelOpen {
    pub const KIND: u16 = 2;

    pub fn new(channel_id: ChannelId, left_bill: AddrHac, right_bill: AddrHac) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            channel_id,
            left_bill,
            right_bill,
        }
    }
}

impl ChannelClose {
    pub const KIND: u16 = 3;

    pub fn new(channel_id: ChannelId) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            channel_id,
        }
    }
}

base::impl_action_facts! {
    ChannelOpen {
        name: "channel_open",
        scope: base::ActScope::TOP,
        min_tx_type: 2,
        extra9: |_: &ChannelOpen| false,
        req_sign: |this: &ChannelOpen| vec![
            AddrOrPtr::Addr(this.left_bill.address),
            AddrOrPtr::Addr(this.right_bill.address),
        ],
        as_transfer_like: none,
        description: |this: &ChannelOpen| format!("Open channel {} for {} and {}", this.channel_id, this.left_bill.address.to_readable(), this.right_bill.address.to_readable()),

    }
}

base::impl_action_facts! {
    ChannelClose {
        name: "channel_close",
        scope: base::ActScope::TOP,
        min_tx_type: 2,
        description: |this: &ChannelClose| format!("Close channel {}", this.channel_id),

    }
}

#[cfg(test)]
mod tests {
    use field::Amount;

    #[test]
    fn channel_totals_keep_the_legacy_u64_consensus_boundary() {
        let high = Amount::from("1:248").unwrap();
        let low = Amount::from("1:228").unwrap();
        assert!(high.add_mode_u64(&low).is_err());
    }
}
