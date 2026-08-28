//! Shared apply path for declared transfers and TEX named-diamond moves.

use base::{
    asset_transfer, hac_transfer, hacd_transfer, sat_transfer, Context, TransferAsset,
    TransferIntent,
};
use field::{Address, DiamondNameListMax200, ToJSON};
use sys::Ret;

use crate::params::SETTLEMENT_ADDR;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiamondMove {
    /// User-to-user. Rejects every `privkey_unknown` recipient, including the blackhole.
    User,
    /// TEX escrow: one endpoint must be `SETTLEMENT_ADDR`.
    TexEscrow,
}

pub(crate) fn check_diamond_move(from: &Address, to: &Address, mode: DiamondMove) -> sys::Rerr {
    match mode {
        DiamondMove::User => {
            if to.is_privkey_unknown() {
                return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
            }
        }
        DiamondMove::TexEscrow => {
            if !SETTLEMENT_ADDR.is_privkey() {
                return sys::errf!(
                    "tex settlement address {} must be PRIVAKEY type",
                    SETTLEMENT_ADDR.to_readable()
                );
            }
            if !SETTLEMENT_ADDR.is_privkey_unknown() {
                return sys::errf!(
                    "tex settlement address {} must be a system address (value < u32::MAX)",
                    SETTLEMENT_ADDR.to_readable()
                );
            }
            if *from != SETTLEMENT_ADDR && *to != SETTLEMENT_ADDR {
                return sys::errf!("tex diamond move must involve the settlement address");
            }
        }
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
    mode: DiamondMove,
) -> Ret<Vec<u8>> {
    check_diamond_move(from, to, mode)?;
    let form = diamond_form(ctx)?;
    hacd_transfer(ctx, from, to, diamonds, form)
}

pub(crate) fn apply_transfer_intent(ctx: &mut dyn Context, intent: TransferIntent) -> Ret<Vec<u8>> {
    let (from, to) = intent.resolve_endpoints(ctx)?;
    match intent.asset {
        TransferAsset::Hac(amount) => hac_transfer(ctx, &from, &to, &amount),
        TransferAsset::Sat(satoshi) => sat_transfer(ctx, &from, &to, &satoshi),
        TransferAsset::Asset(asset) => asset_transfer(ctx, &from, &to, &asset),
        TransferAsset::Diamond(list) => {
            diamonds_transfer(ctx, &from, &to, &list, DiamondMove::User)
        }
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
    fn user_mode_rejects_system_and_blackhole() {
        let from = user_addr();
        assert!(check_diamond_move(&from, &SETTLEMENT_ADDR, DiamondMove::User).is_err());
        assert!(check_diamond_move(&from, &Address::from([0u8; 21]), DiamondMove::User).is_err());
        assert!(check_diamond_move(&from, &user_addr(), DiamondMove::User).is_ok());
    }

    #[test]
    fn escrow_mode_requires_settlement_endpoint() {
        let user = user_addr();
        let other = Address::from([
            0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        assert!(check_diamond_move(&user, &other, DiamondMove::TexEscrow).is_err());
        assert!(check_diamond_move(&user, &SETTLEMENT_ADDR, DiamondMove::TexEscrow).is_ok());
        assert!(check_diamond_move(&SETTLEMENT_ADDR, &user, DiamondMove::TexEscrow).is_ok());
    }
}
