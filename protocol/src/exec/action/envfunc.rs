//! VM syscall action execute bodies.

use base::CoreState;
use field::{Address, DiamondName, Encode};
use sys::errf;

use crate::codec::action::{
    EnvBlockAuthorAddr, EnvHeight, EnvMainAddr, ViewAssetBalance, ViewBalance, ViewCheckSign,
    ViewDiaInscGet, ViewDiaInscNum, ViewDiaNameList, ViewDiaOwnerAddrs,
};

base::impl_action_execute! {
    EnvHeight {
        (self, ctx) {
            Ok(ctx.env().block.height.to_be_bytes().to_vec())
        }
    }
}

base::impl_action_execute! {
    EnvMainAddr {
        (self, ctx) {
            Ok(ctx.env().tx.main.as_ref().to_vec())
        }
    }
}

base::impl_action_execute! {
    EnvBlockAuthorAddr {
        (self, ctx) {
            Ok(ctx.env().block.author.as_ref().to_vec())
        }
    }
}

base::impl_action_execute! {
    ViewBalance {
        (self, ctx) {
            let bls = CoreState::wrap(ctx.layer())
                .balance(&self.addr)?
                .unwrap_or_default();
            let dia = bls.diamond.uint();
            if dia > u32::MAX as u64 {
                return errf!(
                    "address {} diamond count {} exceeds u32::MAX",
                    self.addr.to_readable(),
                    dia
                );
            }
            let hac = bls.hacash.encode();
            let mut res = Vec::with_capacity(12 + hac.len());
            res.extend_from_slice(&(dia as u32).to_be_bytes());
            res.extend_from_slice(&bls.satoshi.uint().to_be_bytes());
            res.extend_from_slice(&hac);
            Ok(res)
        }
    }
}

base::impl_action_execute! {
    ViewAssetBalance {
        (self, ctx) {
            let serial = self.serial.uint();
            if serial == 0 {
                return errf!("asset serial cannot be zero");
            }
            let bls = CoreState::wrap(ctx.layer())
                .balance(&self.addr)?
                .unwrap_or_default();
            let amt = bls
                .assets
                .as_list()
                .iter()
                .find(|a| a.serial.uint() == serial)
                .map(|a| a.amount.uint())
                .unwrap_or(0);
            Ok(amt.to_be_bytes().to_vec())
        }
    }
}

base::impl_action_execute! {
    ViewCheckSign {
        (self, ctx) {
            let ok = match ctx.check_sign(&self.addr) {
                Ok(()) => 1u8,
                Err(_) => 0u8,
            };
            Ok(vec![ok])
        }
    }
}

base::impl_action_execute! {
    ViewDiaInscNum {
        (self, ctx) {
            let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond)? else {
                return errf!("diamond {} not found", self.diamond.to_readable());
            };
            let num = diaobj.inscripts.length();
            if num > u8::MAX as usize {
                return errf!(
                    "diamond {} inscripts number invalid",
                    self.diamond.to_readable()
                );
            }
            Ok(vec![num as u8])
        }
    }
}

base::impl_action_execute! {
    ViewDiaInscGet {
        (self, ctx) {
            let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond)? else {
                return errf!("diamond {} not found", self.diamond.to_readable());
            };
            let num = diaobj.inscripts.length();
            let idx = self.inscidx.uint() as usize;
            if idx >= num {
                return errf!(
                    "diamond {} inscripts number overflow",
                    self.diamond.to_readable()
                );
            }
            Ok(diaobj.inscripts.as_list()[idx].content.to_vec())
        }
    }
}

base::impl_action_execute! {
    ViewDiaNameList {
        (self, ctx) {
            const DNM_SZ: usize = DiamondName::SIZE;
            let owned = CoreState::wrap(ctx.layer())
                .diamond_owned(&self.addr)?
                .unwrap_or_default();
            let names = owned.names.as_ref();
            if names.len() % DNM_SZ != 0 {
                return errf!(
                    "address {} diamond names length {} invalid",
                    self.addr.to_readable(),
                    names.len()
                );
            }
            let limit = self.limit.uint() as usize;
            if limit > 200 {
                return errf!("limit {} cannot exceed 200", limit);
            }
            if limit == 0 {
                return Ok(vec![]);
            }
            let page = self.page.uint() as usize;
            let unit = limit * DNM_SZ;
            let start = page.saturating_mul(unit);
            if start >= names.len() {
                return Ok(vec![]);
            }
            let end = start.saturating_add(unit).min(names.len());
            Ok(names[start..end].to_vec())
        }
    }
}

base::impl_action_execute! {
    ViewDiaOwnerAddrs {
        (self, ctx) {
            let num = self.diamonds.check()?;
            if num > 50 {
                return errf!("diamond list length {} cannot exceed 50", num);
            }
            let state = CoreState::wrap(ctx.layer());
            let mut res = Vec::with_capacity(num * Address::SIZE);
            for dian in self.diamonds.as_list() {
                let Some(diaobj) = state.diamond(dian)? else {
                    return errf!("diamond {} not found", dian.to_readable());
                };
                res.extend_from_slice(diaobj.address.as_ref());
            }
            Ok(res)
        }
    }
}
