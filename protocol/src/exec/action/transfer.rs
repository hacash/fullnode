//! Hac / Sat / Asset / Diamond transfer execute bodies.

use base::{
    Context, CoreState, asset_transfer, diamond_owned_move, hac_transfer, hacd_move_one_diamond,
    hacd_transfer, sat_transfer,
};
use field::{Address, DiamondNameListMax200, DiamondNumber, ToJSON};
use sys::Ret;

use crate::codec::action::{
    AssetFromToTrs, AssetFromTrs, AssetToTrs, DiaFromToTrs, DiaFromTrs, DiaSingleTrs, DiaToTrs,
    HacFromToTrs, HacFromTrs, HacToTrs, SatFromToTrs, SatFromTrs, SatToTrs,
};

base::impl_action_execute! {
    HacToTrs {
        (self, ctx) {
            let from = ctx.env().tx.main;
            let to = ctx.addr(&self.to)?;
            hac_transfer(ctx, &from, &to, &self.hacash)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    HacFromTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.env().tx.main;
            hac_transfer(ctx, &from, &to, &self.hacash)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    HacFromToTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.addr(&self.to)?;
            hac_transfer(ctx, &from, &to, &self.hacash)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    SatToTrs {
        (self, ctx) {
            let from = ctx.env().tx.main;
            let to = ctx.addr(&self.to)?;
            sat_transfer(ctx, &from, &to, &self.satoshi)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    SatFromTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.env().tx.main;
            sat_transfer(ctx, &from, &to, &self.satoshi)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    SatFromToTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.addr(&self.to)?;
            sat_transfer(ctx, &from, &to, &self.satoshi)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    AssetToTrs {
        (self, ctx) {
            let from = ctx.env().tx.main;
            let to = ctx.addr(&self.to)?;
            asset_transfer(ctx, &from, &to, &self.asset)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    AssetFromTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.env().tx.main;
            asset_transfer(ctx, &from, &to, &self.asset)?;
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    AssetFromToTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.addr(&self.to)?;
            asset_transfer(ctx, &from, &to, &self.asset)?;
            Ok(vec![])
        }
    }
}

fn is_privakey_unknown(addr: &Address) -> bool {
    addr.is_privkey_unknown()
}

fn do_diamonds_transfer(
    ctx: &mut dyn Context,
    diamonds: &DiamondNameListMax200,
    from: &Address,
    to: &Address,
) -> Ret<Vec<u8>> {
    let dianum = diamonds.check()?;
    let diamond_form_flag = crate::execution_params(ctx.services().as_ref())?.diamond_form_flag;
    let diamond_form = ctx.env().chain.consensus_flags & diamond_form_flag != 0;
    let mut state = CoreState::wrap(ctx.layer());
    for name in diamonds.as_list() {
        hacd_move_one_diamond(&mut state, from, to, name)?;
    }
    if diamond_form {
        diamond_owned_move(&mut state, from, to, diamonds)?;
    }
    hacd_transfer(
        &mut state,
        from,
        to,
        &DiamondNumber::from(dianum as u32),
        diamonds,
    )
}

base::impl_action_execute! {
    DiaSingleTrs {
        (self, ctx) {
            let from = ctx.env().tx.main;
            let to = ctx.addr(&self.to)?;
            if is_privakey_unknown(&to) {
                return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
            }
            let diamonds = DiamondNameListMax200::one(self.diamond);
            do_diamonds_transfer(ctx, &diamonds, &from, &to)
        }
    }
}

base::impl_action_execute! {
    DiaFromToTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.addr(&self.to)?;
            if is_privakey_unknown(&to) {
                return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
            }
            do_diamonds_transfer(ctx, &self.diamonds, &from, &to)
        }
    }
}

base::impl_action_execute! {
    DiaToTrs {
        (self, ctx) {
            let from = ctx.env().tx.main;
            let to = ctx.addr(&self.to)?;
            if is_privakey_unknown(&to) {
                return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
            }
            do_diamonds_transfer(ctx, &self.diamonds, &from, &to)
        }
    }
}

base::impl_action_execute! {
    DiaFromTrs {
        (self, ctx) {
            let from = ctx.addr(&self.from)?;
            let to = ctx.env().tx.main;
            do_diamonds_transfer(ctx, &self.diamonds, &from, &to)
        }
    }
}
