//! Channel open/close action (kind 2/3, moved from mint; execution body gated by the `execute` feature).

use base::AddrOrPtr;
use field::{AddrHac, ChannelId};

base::action_simple! { ChannelOpen, 2, 2, TOP, {
    channel_id: ChannelId,
    left_bill: AddrHac,
    right_bill: AddrHac
}, this, {
    req_sign: {vec![AddrOrPtr::Addr(this.left_bill.address), AddrOrPtr::Addr(this.right_bill.address)]},
    description: format!("Open channel {} for {} and {}", this.channel_id, this.left_bill.address.to_readable(), this.right_bill.address.to_readable())
}}

base::action_simple! { ChannelClose, 3, 2, TOP, {
    channel_id: ChannelId
}, this, {
    description: format!("Close channel {}", this.channel_id)
}}

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
