//! VM syscall actions: ACTENV (0x07xx) / ACTVIEW (0x06xx).
//!
//! Invoked via `Context::action_call` with kid = `[0x07|0x06, idx]` where idx = KIND % 256.

use std::any::Any;
use std::sync::Arc;

use base::{ActOut, ActScope, Action, ActionRef, Context, CoreState};
use field::{
    Address, DiamondName, DiamondNameListMax200, DiamondNumber, Encode, Fold64, Reader, Uint1,
    Uint2,
};
use sys::{Ret, errf};

#[derive(Debug, Clone)]
pub struct EnvHeight {
    pub kind: Uint2,
}

#[derive(Debug, Clone)]
pub struct EnvMainAddr {
    pub kind: Uint2,
}

#[derive(Debug, Clone)]
pub struct EnvBlockAuthorAddr {
    pub kind: Uint2,
}

#[derive(Debug, Clone)]
pub struct ViewBalance {
    pub kind: Uint2,
    pub addr: Address,
}

#[derive(Debug, Clone)]
pub struct ViewAssetBalance {
    pub kind: Uint2,
    pub addr: Address,
    pub serial: Fold64,
}

#[derive(Debug, Clone)]
pub struct ViewCheckSign {
    pub kind: Uint2,
    pub addr: Address,
}

#[derive(Debug, Clone)]
pub struct ViewDiaInscNum {
    pub kind: Uint2,
    pub diamond: DiamondName,
}

#[derive(Debug, Clone)]
pub struct ViewDiaInscGet {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub inscidx: Uint1,
}

#[derive(Debug, Clone)]
pub struct ViewDiaNameList {
    pub kind: Uint2,
    pub addr: Address,
    pub page: DiamondNumber,
    pub limit: DiamondNumber,
}

#[derive(Debug, Clone)]
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

impl Encode for EnvHeight {
    fn size(&self) -> usize {
        self.kind.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
    }
}

impl Action for EnvHeight {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        Ok((gas, ctx.env().block.height.to_be_bytes().to_vec()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for EnvMainAddr {
    fn size(&self) -> usize {
        self.kind.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
    }
}

impl Action for EnvMainAddr {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        Ok((gas, ctx.env().tx.main.as_ref().to_vec()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for EnvBlockAuthorAddr {
    fn size(&self) -> usize {
        self.kind.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
    }
}

impl Action for EnvBlockAuthorAddr {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        Ok((gas, ctx.env().block.author.as_ref().to_vec()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for ViewBalance {
    fn size(&self) -> usize {
        self.kind.size() + self.addr.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.addr.encode_to(out);
    }
}

impl Action for ViewBalance {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let bls = CoreState::wrap(ctx.layer())
            .balance(&self.addr)
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
        Ok((gas, res))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for ViewAssetBalance {
    fn size(&self) -> usize {
        self.kind.size() + self.addr.size() + self.serial.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.addr.encode_to(out);
        self.serial.encode_to(out);
    }
}

impl Action for ViewAssetBalance {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let serial = self.serial.uint();
        if serial == 0 {
            return errf!("asset serial cannot be zero");
        }
        let bls = CoreState::wrap(ctx.layer())
            .balance(&self.addr)
            .unwrap_or_default();
        let amt = bls
            .assets
            .as_list()
            .iter()
            .find(|a| a.serial.uint() == serial)
            .map(|a| a.amount.uint())
            .unwrap_or(0);
        Ok((gas, amt.to_be_bytes().to_vec()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for ViewCheckSign {
    fn size(&self) -> usize {
        self.kind.size() + self.addr.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.addr.encode_to(out);
    }
}

impl Action for ViewCheckSign {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let ok = match ctx.check_sign(&self.addr) {
            Ok(()) => 1u8,
            Err(_) => 0u8,
        };
        Ok((gas, vec![ok]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for ViewDiaInscNum {
    fn size(&self) -> usize {
        self.kind.size() + self.diamond.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.diamond.encode_to(out);
    }
}

impl Action for ViewDiaInscNum {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond) else {
            return errf!("diamond {} not found", self.diamond.to_readable());
        };
        let num = diaobj.inscripts.length();
        if num > u8::MAX as usize {
            return errf!(
                "diamond {} inscripts number invalid",
                self.diamond.to_readable()
            );
        }
        Ok((gas, vec![num as u8]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for ViewDiaInscGet {
    fn size(&self) -> usize {
        self.kind.size() + self.diamond.size() + self.inscidx.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.diamond.encode_to(out);
        self.inscidx.encode_to(out);
    }
}

impl Action for ViewDiaInscGet {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let Some(diaobj) = CoreState::wrap(ctx.layer()).diamond(&self.diamond) else {
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
        Ok((gas, diaobj.inscripts.as_list()[idx].content.to_vec()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for ViewDiaNameList {
    fn size(&self) -> usize {
        self.kind.size() + self.addr.size() + self.page.size() + self.limit.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.addr.encode_to(out);
        self.page.encode_to(out);
        self.limit.encode_to(out);
    }
}

impl Action for ViewDiaNameList {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        const DNM_SZ: usize = DiamondName::SIZE;
        let owned = CoreState::wrap(ctx.layer())
            .diamond_owned(&self.addr)
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
            return Ok((gas, vec![]));
        }
        let page = self.page.uint() as usize;
        let unit = limit * DNM_SZ;
        let start = page.saturating_mul(unit);
        if start >= names.len() {
            return Ok((gas, vec![]));
        }
        let end = start.saturating_add(unit).min(names.len());
        Ok((gas, names[start..end].to_vec()))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Encode for ViewDiaOwnerAddrs {
    fn size(&self) -> usize {
        self.kind.size() + self.diamonds.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.diamonds.encode_to(out);
    }
}

impl Action for ViewDiaOwnerAddrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL_ONLY
    }
    fn min_tx_type(&self) -> u8 {
        3
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let num = self.diamonds.check()?;
        if num > 50 {
            return errf!("diamond list length {} cannot exceed 50", num);
        }
        let state = CoreState::wrap(ctx.layer());
        let mut res = Vec::with_capacity(num * Address::SIZE);
        for dian in self.diamonds.as_list() {
            let Some(diaobj) = state.diamond(dian) else {
                return errf!("diamond {} not found", dian.to_readable());
            };
            res.extend_from_slice(diaobj.address.as_ref());
        }
        Ok((gas, res))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_envfunc_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind_field: Uint2 = r.read()?;
    if kind_field.uint() != kind {
        return sys::decodef!(
            "action kind mismatch: expected {} got {}",
            kind,
            kind_field.uint()
        );
    }
    match kind {
        EnvHeight::KIND => Ok((Arc::new(EnvHeight { kind: kind_field }), r.used())),
        EnvMainAddr::KIND => Ok((Arc::new(EnvMainAddr { kind: kind_field }), r.used())),
        EnvBlockAuthorAddr::KIND => {
            Ok((Arc::new(EnvBlockAuthorAddr { kind: kind_field }), r.used()))
        }
        ViewBalance::KIND => {
            let addr: Address = r.read()?;
            Ok((
                Arc::new(ViewBalance {
                    kind: kind_field,
                    addr,
                }),
                r.used(),
            ))
        }
        ViewAssetBalance::KIND => {
            let addr: Address = r.read()?;
            let serial: Fold64 = r.read()?;
            Ok((
                Arc::new(ViewAssetBalance {
                    kind: kind_field,
                    addr,
                    serial,
                }),
                r.used(),
            ))
        }
        ViewCheckSign::KIND => {
            let addr: Address = r.read()?;
            Ok((
                Arc::new(ViewCheckSign {
                    kind: kind_field,
                    addr,
                }),
                r.used(),
            ))
        }
        ViewDiaInscNum::KIND => {
            let diamond: DiamondName = r.read()?;
            Ok((
                Arc::new(ViewDiaInscNum {
                    kind: kind_field,
                    diamond,
                }),
                r.used(),
            ))
        }
        ViewDiaInscGet::KIND => {
            let diamond: DiamondName = r.read()?;
            let inscidx: Uint1 = r.read()?;
            Ok((
                Arc::new(ViewDiaInscGet {
                    kind: kind_field,
                    diamond,
                    inscidx,
                }),
                r.used(),
            ))
        }
        ViewDiaNameList::KIND => {
            let addr: Address = r.read()?;
            let page: DiamondNumber = r.read()?;
            let limit: DiamondNumber = r.read()?;
            Ok((
                Arc::new(ViewDiaNameList {
                    kind: kind_field,
                    addr,
                    page,
                    limit,
                }),
                r.used(),
            ))
        }
        ViewDiaOwnerAddrs::KIND => {
            let diamonds: DiamondNameListMax200 = r.read()?;
            Ok((
                Arc::new(ViewDiaOwnerAddrs {
                    kind: kind_field,
                    diamonds,
                }),
                r.used(),
            ))
        }
        _ => sys::decodef!("envfunc action kind {} not registered", kind),
    }
}
