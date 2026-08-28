//! TexCellExecute execute body and TEX cell state changes.

use base::{
    asset_add, asset_sub, hac_add, hac_sub, sat_add, sat_sub, Context, CoreState, ExecFrom,
};
use field::{Address, Amount, Fold64, Hash, Satoshi, Sign};
use sys::{errf, Account, Rerr};

use crate::codec::action::tex::{TexCell, TexCellExecute};
use crate::exec::apply::{diamonds_transfer, DiamondMove};
use crate::params::SETTLEMENT_ADDR;

fn tex_check_asset_serial(ctx: &mut dyn Context, serial: Fold64) -> Rerr {
    if serial.is_zero() {
        return errf!("tex asset serial cannot be zero");
    }
    {
        let tex = ctx.tex_ledger();
        if tex.asset_is_checked(serial) {
            return Ok(());
        }
    }
    let exist = {
        let state = CoreState::wrap(ctx.layer());
        state.asset(&serial)?.is_some()
    };
    if !exist {
        return errf!("tex asset <{}> does not exist", serial.uint());
    }
    ctx.tex_ledger_mut_top()?.mark_asset_checked(serial);
    Ok(())
}

fn verify_signature(hash: &Hash, addr: &Address, sign: &Sign) -> bool {
    let got = Address::from(Account::get_address_by_public_key(
        sign.publickey.into_array(),
    ));
    got == *addr && Account::verify_signature(&hash.0, &sign.publickey, &sign.signature)
}

impl TexCell {
    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        match self {
            Self::ZhuPay { haczhu } => {
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_sub(ctx, taradr, &amt)?;
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(zhures) = tex.zhu.checked_add(zhu as i64) else {
                    return errf!("cell state coin zhu overflow");
                };
                tex.zhu = zhures;
                Ok(())
            }
            Self::ZhuGet { haczhu } => {
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_add(ctx, taradr, &amt)?;
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(zhures) = tex.zhu.checked_sub(zhu as i64) else {
                    return errf!("cell state coin zhu overflow");
                };
                tex.zhu = zhures;
                Ok(())
            }
            Self::SatPay { satnum } => {
                let sat = Satoshi::from(satnum.uint());
                sat_sub(ctx, taradr, &sat)?;
                let n = satnum.uint();
                if n > i64::MAX as u64 {
                    return errf!("cell sat too large");
                }
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(satres) = tex.sat.checked_add(n as i64) else {
                    return errf!("cell state coin sat overflow");
                };
                tex.sat = satres;
                Ok(())
            }
            Self::SatGet { satnum } => {
                let sat = Satoshi::from(satnum.uint());
                sat_add(ctx, taradr, &sat)?;
                let n = satnum.uint();
                if n > i64::MAX as u64 {
                    return errf!("cell sat too large");
                }
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(satres) = tex.sat.checked_sub(n as i64) else {
                    return errf!("cell state coin sat overflow");
                };
                tex.sat = satres;
                Ok(())
            }
            Self::DiaPay { diamonds } => {
                diamonds_transfer(
                    ctx,
                    taradr,
                    &SETTLEMENT_ADDR,
                    diamonds,
                    DiamondMove::TexEscrow,
                )?;
                let max = crate::execution_params(ctx.services().as_ref())?.tex_diamond_pay_max;
                ctx.tex_ledger_mut_top()?.record_diamond_pay(diamonds, max)
            }
            Self::DiaGet { dianum } => {
                if dianum.uint() == 0 {
                    return errf!("cell diamond get: number cannot be zero");
                }
                let max =
                    crate::execution_params(ctx.services().as_ref())?.tex_diamond_get_max_per_tx;
                ctx.tex_ledger_mut_top()?
                    .record_diamond_get(*taradr, dianum.uint() as usize, max)
            }
            Self::AssetPay { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                {
                    let mut state = CoreState::wrap(ctx.layer());
                    asset_sub(&mut state, taradr, asset)?;
                }
                ctx.tex_ledger_mut_top()?.record_asset_pay(asset)?;
                Ok(())
            }
            Self::AssetGet { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                {
                    let mut state = CoreState::wrap(ctx.layer());
                    asset_add(&mut state, taradr, asset)?;
                }
                ctx.tex_ledger_mut_top()?.record_asset_get(*taradr, asset)?;
                Ok(())
            }
            Self::CondZhuAtMost { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu >= bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondZhuAtLeast { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu <= bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondZhuEq { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu == bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondSatAtMost { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() >= sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondSatAtLeast { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() <= sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondSatEq { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() == sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondDiaAtMost { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() >= dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondDiaAtLeast { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() <= dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondDiaEq { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() == dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondAssetAtMost { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() >= ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondAssetAtLeast { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() <= ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondAssetEq { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() == ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondHeightAtMost { height } => {
                if height.uint() >= ctx.env().block.height {
                    Ok(())
                } else {
                    errf!("cell condition check failed")
                }
            }
            Self::CondHeightAtLeast { height } => {
                if height.uint() <= ctx.env().block.height {
                    Ok(())
                } else {
                    errf!("cell condition check failed")
                }
            }
            Self::CondChainIdEq { chainid } => {
                if ctx.env().chain.id.get() == chainid.uint() {
                    Ok(())
                } else {
                    errf!("cell condition chain id check failed")
                }
            }
        }
    }
}

base::impl_action_execute! {
    TexCellExecute {
        (self, ctx) {
            if ctx.exec_from() != ExecFrom::Top {
                return errf!(
                    "TexCellExecute can only run in TOP context, got {}",
                    ctx.exec_from()
                );
            }
            self.addr.must_privkey()?;
            let thx = self.get_sign_stuff();
            if !verify_signature(&thx, &self.addr, &self.sign) {
                return errf!(
                    "address {} signature verification failed in tex cell action",
                    self.addr.to_readable()
                );
            }
            for cell in &self.cells {
                cell.execute(ctx, &self.addr)?;
            }
            Ok(vec![])
        }
    }
}
