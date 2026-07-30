//! ChainAllow / HeightScope / BalanceFloor / ReqSignList guard actions.

use std::any::Any;
use std::collections::HashSet;
use std::sync::Arc;

use base::{ActOut, ActScope, Action, ActionRef, AddrOrPtr, Context, CoreState};
use field::{
    Amount, AssetAmt, AssetAmtW1, BlockHeight, ChainIDList, DiamondNumber, Encode, Reader, Satoshi,
    ToJSON, Uint2,
};
use sys::{Rerr, Ret, errf};

use super::common::{addr_or_ptr_size, decode_addr_or_ptr, encode_addr_or_ptr};

#[derive(Debug, Clone)]
pub struct ChainAllow {
    pub kind: Uint2,
    pub chains: ChainIDList,
}

#[derive(Debug, Clone)]
pub struct HeightScope {
    pub kind: Uint2,
    pub start: BlockHeight,
    pub end: BlockHeight,
}

#[derive(Debug, Clone)]
pub struct BalanceFloor {
    pub kind: Uint2,
    pub addr: AddrOrPtr,
    pub hacash: Amount,
    pub satoshi: Satoshi,
    pub diamond: DiamondNumber,
    pub assets: AssetAmtW1,
}

impl ChainAllow {
    pub const KIND: u16 = 0x0411;

    pub fn new(chains: ChainIDList) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            chains,
        }
    }
}

impl HeightScope {
    pub const KIND: u16 = 0x0412;

    pub fn new(start: BlockHeight, end: BlockHeight) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            start,
            end,
        }
    }
}

impl BalanceFloor {
    pub const KIND: u16 = 0x0413;

    pub fn new(addr: AddrOrPtr, hacash: Amount, satoshi: Satoshi, diamond: DiamondNumber) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            addr,
            hacash,
            satoshi,
            diamond,
            assets: AssetAmtW1::default(),
        }
    }
}

/// Explicit extra required signers beyond intrinsic action req_sign.
/// Type3 uses this as E in D = R0 ∪ E (exact SignW2 match).
#[derive(Debug, Clone)]
pub struct ReqSignList {
    pub kind: Uint2,
    pub signers: Vec<AddrOrPtr>,
}

impl ReqSignList {
    pub const KIND: u16 = 0x0414;

    pub fn create_by(signers: Vec<AddrOrPtr>) -> Ret<Self> {
        if signers.is_empty() {
            return errf!("ReqSignList cannot be empty");
        }
        Ok(Self {
            kind: Uint2::from(Self::KIND),
            signers,
        })
    }

    pub fn create_by_addrs(addrs: Vec<field::Address>) -> Ret<Self> {
        let ptrs = addrs.into_iter().map(AddrOrPtr::Addr).collect();
        Self::create_by(ptrs)
    }

    /// Resolve and validate E: non-empty, unique, PRIVAKEY, not unknown system.
    pub fn validate_against(&self, addrs: &[field::Address]) -> Ret<HashSet<field::Address>> {
        if self.signers.is_empty() {
            return errf!("ReqSignList cannot be empty");
        }
        let mut e = HashSet::new();
        for ptr in &self.signers {
            let adr = ptr.real(addrs)?;
            if !adr.is_privkey() {
                return errf!(
                    "ReqSignList address {} must be PRIVAKEY type",
                    adr.to_readable()
                );
            }
            if adr.is_privkey_unknown() {
                return errf!(
                    "ReqSignList address {} is a system address with unknown private key",
                    adr.to_readable()
                );
            }
            if !e.insert(adr) {
                return errf!("ReqSignList address {} is duplicated", adr.to_readable());
            }
        }
        Ok(e)
    }
}

impl Encode for ChainAllow {
    fn size(&self) -> usize {
        self.kind.size() + self.chains.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.chains.encode_to(out);
    }
}

impl Encode for HeightScope {
    fn size(&self) -> usize {
        self.kind.size() + self.start.size() + self.end.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.start.encode_to(out);
        self.end.encode_to(out);
    }
}

impl Encode for BalanceFloor {
    fn size(&self) -> usize {
        self.kind.size()
            + addr_or_ptr_size(&self.addr)
            + self.hacash.size()
            + self.satoshi.size()
            + self.diamond.size()
            + self.assets.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.addr, out);
        self.hacash.encode_to(out);
        self.satoshi.encode_to(out);
        self.diamond.encode_to(out);
        self.assets.encode_to(out);
    }
}

impl Encode for ReqSignList {
    fn size(&self) -> usize {
        self.kind.size() + Uint2::SIZE + self.signers.iter().map(addr_or_ptr_size).sum::<usize>()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        Uint2::from(self.signers.len() as u16).encode_to(out);
        for ptr in &self.signers {
            encode_addr_or_ptr(ptr, out);
        }
    }
}

fn check_balance_floor_assets(assets: &AssetAmtW1) -> Rerr {
    const BALANCE_ASSET_MAX: usize = 20;
    if assets.length() > BALANCE_ASSET_MAX {
        return errf!(
            "balance floor assets item quantity cannot exceed {}",
            BALANCE_ASSET_MAX
        );
    }
    let mut seen = std::collections::HashSet::new();
    for ast in assets.as_list() {
        let serial = ast.serial.uint();
        let amount = ast.amount.uint();
        if serial == 0 {
            return errf!("balance floor asset serial cannot be zero");
        }
        if amount == 0 {
            return errf!("balance floor asset {} amount cannot be zero", serial);
        }
        if !seen.insert(serial) {
            return errf!("balance floor asset serial {} is duplicated", serial);
        }
    }
    Ok(())
}

impl Action for ChainAllow {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::GUARD
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
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
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for HeightScope {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::GUARD
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
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
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for BalanceFloor {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::GUARD
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        if self.hacash.is_negative() {
            return errf!("balance floor hacash {} cannot be negative", self.hacash);
        }
        check_balance_floor_assets(&self.assets)?;
        let check_hac = !self.hacash.is_zero();
        let check_sat = self.satoshi.uint() > 0;
        let check_dia = self.diamond.uint() > 0;
        let check_assets = self.assets.length() > 0;
        if !(check_hac || check_sat || check_dia || check_assets) {
            return errf!("balance floor is empty");
        }
        let addr = ctx.addr(&self.addr)?;
        let balance = CoreState::wrap(ctx.layer())
            .balance(&addr)
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
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for ReqSignList {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::TOP_GUARD_UNIQUE
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        self.signers.clone()
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        self.validate_against(&ctx.env().tx.addrs)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_chain_guard_action(
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
        ChainAllow::KIND => {
            let chains: ChainIDList = r.read()?;
            Ok((
                Arc::new(ChainAllow {
                    kind: kind_field,
                    chains,
                }),
                r.used(),
            ))
        }
        HeightScope::KIND => {
            let start: BlockHeight = r.read()?;
            let end: BlockHeight = r.read()?;
            Ok((
                Arc::new(HeightScope {
                    kind: kind_field,
                    start,
                    end,
                }),
                r.used(),
            ))
        }
        BalanceFloor::KIND => {
            let (addr, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let hacash: Amount = r.read()?;
            let satoshi: Satoshi = r.read()?;
            let diamond: DiamondNumber = r.read()?;
            let assets: AssetAmtW1 = r.read()?;
            Ok((
                Arc::new(BalanceFloor {
                    kind: kind_field,
                    addr,
                    hacash,
                    satoshi,
                    diamond,
                    assets,
                }),
                r.used(),
            ))
        }
        ReqSignList::KIND => {
            let count: Uint2 = r.read()?;
            let mut signers = Vec::with_capacity(count.uint() as usize);
            for _ in 0..count.uint() {
                let (ptr, used) = decode_addr_or_ptr(&buf[r.used()..])?;
                let _ = r.read_bytes(used)?;
                signers.push(ptr);
            }
            Ok((
                Arc::new(ReqSignList {
                    kind: kind_field,
                    signers,
                }),
                r.used(),
            ))
        }
        _ => sys::decodef!("chain guard action kind {} not registered", kind),
    }
}
