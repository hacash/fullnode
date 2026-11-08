//! Shared apply path for declared transfers and TEX named-diamond moves.

use base::{
    Context, TransferAsset, TransferIntent, asset_transfer, hac_transfer, hacd_transfer,
    sat_transfer,
};
use field::{Address, DiamondNameListMax200, ToJSON};
use sys::Ret;

/// Named diamond moves are user-to-user: every `privkey_unknown` recipient,
/// including the blackhole, is rejected.
fn check_diamond_recipient(to: &Address) -> sys::Rerr {
    if to.is_privkey_unknown() {
        return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
    }
    Ok(())
}

fn diamond_form(ctx: &dyn Context) -> Ret<bool> {
    let flag = crate::execution_params(ctx.services().as_ref())?.diamond_form_flag;
    Ok(ctx.env().chain.consensus_flags & flag != 0)
}

pub(crate) fn diamonds_transfer(
    ctx: &mut dyn Context,
    from: &Address,
    to: &Address,
    diamonds: &DiamondNameListMax200,
) -> Ret<Vec<u8>> {
    check_diamond_recipient(to)?;
    let form = diamond_form(ctx)?;
    hacd_transfer(ctx, from, to, diamonds, form)
}

pub(crate) fn apply_transfer_intent(ctx: &mut dyn Context, intent: TransferIntent) -> Ret<Vec<u8>> {
    let (from, to) = intent.resolve_endpoints(ctx)?;
    match intent.asset {
        TransferAsset::Hac(amount) => hac_transfer(ctx, &from, &to, &amount),
        TransferAsset::Sat(satoshi) => sat_transfer(ctx, &from, &to, &satoshi),
        TransferAsset::Asset(asset) => asset_transfer(ctx, &from, &to, &asset),
        TransferAsset::Diamond(list) => diamonds_transfer(ctx, &from, &to, &list),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::Address;

    fn user_addr() -> Address {
        Address::from([
            0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ])
    }

    #[test]
    fn recipient_check_rejects_system_and_blackhole() {
        let user = user_addr();
        // The TEX settlement address and the blackhole are privkey_unknown
        // system addresses; named diamonds must never land there.
        assert!(check_diamond_recipient(&crate::params::SETTLEMENT_ADDR).is_err());
        assert!(check_diamond_recipient(&Address::from([0u8; 21])).is_err());
        assert!(check_diamond_recipient(&user).is_ok());
    }
}
