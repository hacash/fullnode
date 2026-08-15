//! VM syscall actions: ACTENV (0x07xx) / ACTVIEW (0x06xx).
//!
//! Invoked via `Context::action_call` with kid = `[0x07|0x06, idx]` where idx = KIND % 256.

use std::sync::Arc;

use base::{Action, ActionRef, CoreState};
use field::{
    Address, Decode, DiamondName, DiamondNameListMax200, DiamondNumber, Encode, Fold64, Uint1,
    Uint2,
};
use sys::{Ret, errf};

use super::common::check_action_kind;

#[derive(Debug, Clone, base::ActionCodec)]
pub struct EnvHeight {
    pub kind: Uint2,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct EnvMainAddr {
    pub kind: Uint2,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct EnvBlockAuthorAddr {
    pub kind: Uint2,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ViewBalance {
    pub kind: Uint2,
    pub addr: Address,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ViewAssetBalance {
    pub kind: Uint2,
    pub addr: Address,
    pub serial: Fold64,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ViewCheckSign {
    pub kind: Uint2,
    pub addr: Address,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ViewDiaInscNum {
    pub kind: Uint2,
    pub diamond: DiamondName,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ViewDiaInscGet {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub inscidx: Uint1,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ViewDiaNameList {
    pub kind: Uint2,
    pub addr: Address,
    pub page: DiamondNumber,
    pub limit: DiamondNumber,
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ViewDiaOwnerAddrs {
    pub kind: Uint2,
    pub diamonds: DiamondNameListMax200,
}

impl EnvHeight {
    pub const KIND: u16 = 0x0701;
}

impl EnvMainAddr {
    pub const KIND: u16 = 0x0702;
}

impl EnvBlockAuthorAddr {
    pub const KIND: u16 = 0x0703;
}

impl ViewBalance {
    pub const KIND: u16 = 0x0601;
}

impl ViewAssetBalance {
    pub const KIND: u16 = 0x0602;
}

impl ViewCheckSign {
    pub const KIND: u16 = 0x0609;
}

impl ViewDiaInscNum {
    pub const KIND: u16 = 0x0611;
}

impl ViewDiaInscGet {
    pub const KIND: u16 = 0x0612;
}

impl ViewDiaNameList {
    pub const KIND: u16 = 0x0613;
}

impl ViewDiaOwnerAddrs {
    pub const KIND: u16 = 0x0614;
}

base::impl_action! {
    EnvHeight {
        name: "block_height",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |_: &EnvHeight| "Syscall: Get block height".to_owned(),
        execute: (self, ctx) { Ok(ctx.env().block.height.to_be_bytes().to_vec()) }
    }
}

base::impl_action! {
    EnvMainAddr {
        name: "tx_main_addr",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |_: &EnvMainAddr| "Syscall: Get main address".to_owned(),
        execute: (self, ctx) { Ok(ctx.env().tx.main.as_ref().to_vec()) }
    }
}

base::impl_action! {
    EnvBlockAuthorAddr {
        name: "block_author_addr",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |_: &EnvBlockAuthorAddr| "Syscall: Get author address".to_owned(),
        execute: (self, ctx) { Ok(ctx.env().block.author.as_ref().to_vec()) }
    }
}

base::impl_action! {
    ViewBalance {
        name: "balance",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewBalance| format!("Syscall: Get balance for {}", this.addr.to_readable()),
        execute: (self, ctx) {
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

base::impl_action! {
    ViewAssetBalance {
        name: "asset_balance",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewAssetBalance| format!("Syscall: Get asset {} balance for {}", this.serial.uint(), this.addr.to_readable()),
        execute: (self, ctx) {
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

base::impl_action! {
    ViewCheckSign {
        name: "check_signature",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewCheckSign| format!("Syscall: Check signature for {}", this.addr.to_readable()),
        execute: (self, ctx) {
        let ok = match ctx.check_sign(&self.addr) {
            Ok(()) => 1u8,
            Err(_) => 0u8,
        };
        Ok(vec![ok])
        }
    }
}

base::impl_action! {
    ViewDiaInscNum {
        name: "hacd_insc_num",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaInscNum| format!("Syscall: Get diamond inscription number for <{}>", this.diamond.to_readable()),
        execute: (self, ctx) {
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

base::impl_action! {
    ViewDiaInscGet {
        name: "hacd_insc_get",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaInscGet| format!("Syscall: Get diamond inscription data for <{}>", this.diamond.to_readable()),
        execute: (self, ctx) {
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

base::impl_action! {
    ViewDiaNameList {
        name: "hacd_name_list",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaNameList| format!("Syscall: Get HACD name list for {} page {} limit {}", this.addr.to_readable(), this.page.uint(), this.limit.uint()),
        execute: (self, ctx) {
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

base::impl_action! {
    ViewDiaOwnerAddrs {
        name: "hacd_owner_addrs",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaOwnerAddrs| format!("Syscall: Get HACD owner addresses for {}", this.diamonds.splitstr()),
        execute: (self, ctx) {
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

pub fn create_envfunc_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    match kind {
        EnvHeight::KIND => decode_envfunc_action::<EnvHeight>(buf),
        EnvMainAddr::KIND => decode_envfunc_action::<EnvMainAddr>(buf),
        EnvBlockAuthorAddr::KIND => decode_envfunc_action::<EnvBlockAuthorAddr>(buf),
        ViewBalance::KIND => decode_envfunc_action::<ViewBalance>(buf),
        ViewAssetBalance::KIND => decode_envfunc_action::<ViewAssetBalance>(buf),
        ViewCheckSign::KIND => decode_envfunc_action::<ViewCheckSign>(buf),
        ViewDiaInscNum::KIND => decode_envfunc_action::<ViewDiaInscNum>(buf),
        ViewDiaInscGet::KIND => decode_envfunc_action::<ViewDiaInscGet>(buf),
        ViewDiaNameList::KIND => decode_envfunc_action::<ViewDiaNameList>(buf),
        ViewDiaOwnerAddrs::KIND => decode_envfunc_action::<ViewDiaOwnerAddrs>(buf),
        _ => sys::normalf!("envfunc action kind {} not registered", kind),
    }
}

fn decode_envfunc_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}
