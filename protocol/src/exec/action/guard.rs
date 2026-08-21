//! Guard action execute bodies.

use base::CoreState;
use field::AssetAmt;
use field::ToJSON;
use sys::errf;

use crate::codec::action::guard::validate_balance_floor_struct;
use crate::codec::action::{BalanceFloor, ChainAllow, HeightScope, ReqSignList};

base::impl_action_execute! {
    ChainAllow {
        (self, ctx) {
            let cid = ctx.env().chain.id;
            if !self
                .chains
                .as_list()
                .iter()
                .any(|id| id.uint() == cid.get())
            {
                let cids = self
                    .chains
                    .as_list()
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                return sys::revertf!(
                    "transaction must belong to chains {} but on chain {}",
                    cids,
                    cid
                );
            }
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    HeightScope {
        (self, ctx) {
            let left = self.start.uint();
            let right = match self.end.uint() {
                0 => u64::MAX,
                h => h,
            };
            if left > right {
                return errf!("left height {} cannot exceed right height {}", left, right);
            }
            let height = ctx.env().block.height;
            if height < left || height > right {
                return sys::revertf!(
                    "transaction must be submitted in height between {} and {}",
                    left,
                    right
                );
            }
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    BalanceFloor {
        (self, ctx) {
            let (check_hac, check_sat, check_dia, _check_assets) = validate_balance_floor_struct(self)?;
            let addr = ctx.addr(&self.addr)?;
            let balance = CoreState::wrap(ctx.layer())
                .balance(&addr)?
                .unwrap_or_default();
            if check_hac && balance.hacash < self.hacash {
                return sys::revertf!(
                    "address {} hacash {} is lower than floor {}",
                    addr.to_json(),
                    balance.hacash,
                    self.hacash
                );
            }
            if check_sat {
                let sat = balance.satoshi.to_satoshi();
                if sat < self.satoshi {
                    return sys::revertf!(
                        "address {} satoshi {} is lower than floor {}",
                        addr.to_json(),
                        sat,
                        self.satoshi
                    );
                }
            }
            if check_dia {
                let dia = balance.diamond.to_diamond()?;
                if dia < self.diamond {
                    return sys::revertf!(
                        "address {} diamond {} is lower than floor {}",
                        addr.to_json(),
                        dia,
                        self.diamond
                    );
                }
            }
            for floor in self.assets.as_list() {
                let cur = balance
                    .asset(floor.serial)
                    .unwrap_or(AssetAmt::from_serial(floor.serial)?);
                if cur.amount < floor.amount {
                    return sys::revertf!(
                        "address {} asset {}:{} is lower than floor {}:{}",
                        addr.to_json(),
                        cur.serial,
                        cur.amount,
                        floor.serial,
                        floor.amount
                    );
                }
            }
            Ok(vec![])
        }
    }
}

base::impl_action_execute! {
    ReqSignList {
        (self, ctx) {
            self.validate_against(&ctx.env().tx.addrs)?;
            Ok(vec![])
        }
    }
}
