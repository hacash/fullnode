//! ChainAllow / HeightScope / BalanceFloor / ReqSignList guard actions.

use std::collections::HashSet;
use std::sync::Arc;

use base::{ActScope, Action, ActionRef, AddrOrPtr, CoreState};
use field::{
    Amount, AssetAmt, AssetAmtW1, BlockHeight, ChainIDList, Decode, DiamondNumber, Encode, Reader,
    Satoshi, ToJSON, Uint2, json_decode_value, json_split_array, json_split_object,
};
use sys::{Rerr, Ret, errf};

use super::common::{
    addr_or_ptr_readable, addr_or_ptr_size, check_action_kind, decode_addr_or_ptr,
    encode_addr_or_ptr,
};

#[derive(Debug, Clone, base::ActionCodec)]
pub struct ChainAllow {
    pub kind: Uint2,
    pub chains: ChainIDList,
}

impl field::ToJSON for ReqSignList {
    fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
        let signers = self
            .signers
            .iter()
            .map(|signer| field::ToJSON::to_json_fmt(signer, fmt))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"kind\":{},\"signers\":[{}]}}",
            field::ToJSON::to_json_fmt(&self.kind, fmt),
            signers
        )
    }
}

#[derive(Debug, Clone, base::ActionCodec)]
pub struct HeightScope {
    pub kind: Uint2,
    pub start: BlockHeight,
    pub end: BlockHeight,
}

#[derive(Debug, Clone, base::ActionCodec)]
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

fn parse_req_sign_list_json(json: &str) -> Ret<ReqSignList> {
    let mut declared = Uint2::from(ReqSignList::KIND);
    let mut signers_json = None;
    let mut seen = HashSet::new();
    for (key, value) in json_split_object(json)? {
        if !seen.insert(key) {
            return sys::normalf!("ReqSignList JSON field {} is duplicated", key);
        }
        match key {
            "kind" => declared = json_decode_value(value)?,
            "signers" => signers_json = Some(value),
            _ => {}
        }
    }
    if declared.uint() != ReqSignList::KIND {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            ReqSignList::KIND,
            declared.uint()
        );
    }
    let raw = signers_json.ok_or_else(|| sys::Error::normal("ReqSignList JSON missing signers"))?;
    let mut signers = Vec::new();
    for value in json_split_array(raw)? {
        signers.push(json_decode_value(value)?);
    }
    Ok(ReqSignList::create_by(signers)?)
}

impl field::FromJSON for ReqSignList {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = parse_req_sign_list_json(json)?;
        Ok(())
    }
}

impl base::ActionJsonCodec for ReqSignList {
    fn decode_json(json: &str) -> Ret<Self> {
        parse_req_sign_list_json(json)
    }
}

/// Registry JSON decoder for the dynamic signer-list action.
pub fn decode_req_sign_list_json(
    _reg: &dyn base::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef> {
    if kind != ReqSignList::KIND {
        return sys::normalf!("ReqSignList JSON codec got kind {}", kind);
    }
    Ok(Arc::new(parse_req_sign_list_json(json)?))
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

base::impl_action! {
    ChainAllow {
        name: "chain_allow",
        scope: ActScope::GUARD,
        min_tx_type: 2,
        description: |this: &ChainAllow| format!("Valid chain ID list {}", this.chains.as_list().iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",")),
        execute: (self, ctx) {
#[cfg(all(feature = "codec-only", target_arch = "wasm32"))]
        {
            let _ = (self, ctx);
            crate::codec::action::execution_disabled()
        }
#[cfg(not(all(feature = "codec-only", target_arch = "wasm32")))]
        {

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
}

base::impl_action! {
    HeightScope {
        name: "height_scope",
        scope: ActScope::GUARD,
        min_tx_type: 2,
        description: |this: &HeightScope| format!("Limit height range ({}, {})", this.start.uint(), if this.end.uint() == 0 { "Unlimited".to_owned() } else { this.end.uint().to_string() }),
        execute: (self, ctx) {
#[cfg(all(feature = "codec-only", target_arch = "wasm32"))]
        {
            let _ = (self, ctx);
            crate::codec::action::execution_disabled()
        }
#[cfg(not(all(feature = "codec-only", target_arch = "wasm32")))]
        {

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
}

base::impl_action! {
    BalanceFloor {
        name: "balance_floor",
        scope: ActScope::GUARD,
        min_tx_type: 2,
        description: |this: &BalanceFloor| format!("Balance floor for {} (hac={}, sat={}, dia={}, assets={})", addr_or_ptr_readable(&this.addr), this.hacash, this.satoshi.uint(), this.diamond.uint(), this.assets.length()),
        execute: (self, ctx) {
#[cfg(all(feature = "codec-only", target_arch = "wasm32"))]
        {
            let _ = (self, ctx);
            crate::codec::action::execution_disabled()
        }
#[cfg(not(all(feature = "codec-only", target_arch = "wasm32")))]
        {

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
}

base::impl_action! {
    ReqSignList {
        name: "req_sign_list",
        scope: ActScope::TOP_GUARD_UNIQUE,
        min_tx_type: 2,
        extra9: |_: &ReqSignList| false,
        req_sign: |this: &ReqSignList| this.signers.clone(),
        as_transfer_like: none,
        description: |this: &ReqSignList| format!("Require extra signers ({})", this.signers.len()),
        execute: (self, ctx) {
#[cfg(all(feature = "codec-only", target_arch = "wasm32"))]
        {
            let _ = (self, ctx);
            crate::codec::action::execution_disabled()
        }
#[cfg(not(all(feature = "codec-only", target_arch = "wasm32")))]
        {

        self.validate_against(&ctx.env().tx.addrs)?;
        Ok(vec![])
        
        }
        }
    }
}

pub fn create_chain_guard_action(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    let mut r = Reader::new(buf);
    let kind_field: Uint2 = r.read()?;
    if kind_field.uint() != kind {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            kind,
            kind_field.uint()
        );
    }
    match kind {
        ChainAllow::KIND => decode_regular_guard_action::<ChainAllow>(buf),
        HeightScope::KIND => decode_regular_guard_action::<HeightScope>(buf),
        BalanceFloor::KIND => decode_regular_guard_action::<BalanceFloor>(buf),
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
        _ => sys::normalf!("chain guard action kind {} not registered", kind),
    }
}

fn decode_regular_guard_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}
