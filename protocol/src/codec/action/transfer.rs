//! Hac / Sat / Asset / Diamond transfer actions.

use std::sync::Arc;

use base::{
    ActScope, Action, ActionJsonCodec, ActionExecute, ActionRef, AddrOrPtr, Context, CoreState, TransferLike,
    TransferPayload, asset_transfer, diamond_owned_move, hac_transfer, hacd_move_one_diamond,
    hacd_transfer, sat_transfer,
};
use field::{
    Address, Amount, AssetAmt, Decode, DiamondName, DiamondNameListMax200, DiamondNumber, Encode,
    Satoshi, ToJSON, Uint2,
};
use sys::Ret;

use super::common::{addr_or_ptr_readable, check_action_kind};

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct HacToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub hacash: Amount,
}

pub type HacTransfer = HacToTrs;

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct HacFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub hacash: Amount,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct HacFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub hacash: Amount,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct SatToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct SatFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct SatFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct AssetToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct AssetFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct AssetFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaSingleTrs {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub to: AddrOrPtr,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct DiaFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

impl HacToTrs {
    pub const KIND: u16 = 1;

    pub fn new(to: Address, amount: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            hacash: amount,
        }
    }
}

impl HacFromTrs {
    pub const KIND: u16 = 13;

    pub fn new(from: Address, amount: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            hacash: amount,
        }
    }
}

impl HacFromToTrs {
    pub const KIND: u16 = 14;

    pub fn new(from: Address, to: Address, amount: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            hacash: amount,
        }
    }
}

impl SatToTrs {
    pub const KIND: u16 = 10;

    pub fn new(to: Address, satoshi: Satoshi) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            satoshi,
        }
    }
}

impl SatFromTrs {
    pub const KIND: u16 = 11;

    pub fn new(from: Address, satoshi: Satoshi) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            satoshi,
        }
    }
}

impl SatFromToTrs {
    pub const KIND: u16 = 12;

    pub fn new(from: Address, to: Address, satoshi: Satoshi) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            satoshi,
        }
    }
}

impl AssetToTrs {
    pub const KIND: u16 = 17;

    pub fn new(to: Address, asset: AssetAmt) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            asset,
        }
    }
}

impl AssetFromTrs {
    pub const KIND: u16 = 18;

    pub fn new(from: Address, asset: AssetAmt) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            asset,
        }
    }
}

impl AssetFromToTrs {
    pub const KIND: u16 = 19;

    pub fn new(from: Address, to: Address, asset: AssetAmt) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            asset,
        }
    }
}

impl DiaSingleTrs {
    pub const KIND: u16 = 5;

    pub fn new(diamond: DiamondName, to: Address) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            diamond,
            to: AddrOrPtr::Addr(to),
        }
    }
}

impl DiaFromToTrs {
    pub const KIND: u16 = 6;

    pub fn new(from: Address, to: Address, diamonds: DiamondNameListMax200) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            diamonds,
        }
    }
}

impl DiaToTrs {
    pub const KIND: u16 = 7;

    pub fn new(to: Address, diamonds: DiamondNameListMax200) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            diamonds,
        }
    }
}

impl DiaFromTrs {
    pub const KIND: u16 = 8;

    pub fn new(from: Address, diamonds: DiamondNameListMax200) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            diamonds,
        }
    }
}

impl TransferLike for HacToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        &self.hacash
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hac {
            amount: self.hacash.encode(),
        }
    }
}

impl TransferLike for HacFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        &self.hacash
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hac {
            amount: self.hacash.encode(),
        }
    }
}

impl TransferLike for HacFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        &self.hacash
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hac {
            amount: self.hacash.encode(),
        }
    }
}

impl TransferLike for SatToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Sat {
            satoshi: self.satoshi.uint(),
        }
    }
}

impl TransferLike for SatFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Sat {
            satoshi: self.satoshi.uint(),
        }
    }
}

impl TransferLike for SatFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Sat {
            satoshi: self.satoshi.uint(),
        }
    }
}

base::impl_action! {
    HacToTrs {
        name: "transfer_hac_to",
        scope: ActScope::CALL,
        min_tx_type: 1,
        extra9: |_: &HacToTrs| false,
        req_sign: |_: &HacToTrs| vec![],
        as_transfer_like: self,
        description: |this: &HacToTrs| format!("Transfer {} HAC to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)?;
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    HacFromTrs {
        name: "transfer_hac_from",
        scope: ActScope::CALL,
        min_tx_type: 1,
        extra9: |_: &HacFromTrs| false,
        req_sign: |this: &HacFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &HacFromTrs| format!("Transfer {} HAC from {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from)),
        execute: (self, ctx) {

        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        hac_transfer(ctx, &from, &to, &self.hacash)?;
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    HacFromToTrs {
        name: "transfer_hac_from_to",
        scope: ActScope::CALL,
        min_tx_type: 1,
        extra9: |_: &HacFromToTrs| false,
        req_sign: |this: &HacFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &HacFromToTrs| format!("Transfer {} HAC from {} to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)?;
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    SatToTrs {
        name: "transfer_sat_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &SatToTrs| false,
        req_sign: |_: &SatToTrs| vec![],
        as_transfer_like: self,
        description: |this: &SatToTrs| format!("Transfer {} SAT to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)?;
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    SatFromTrs {
        name: "transfer_sat_from",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &SatFromTrs| false,
        req_sign: |this: &SatFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &SatFromTrs| format!("Transfer {} SAT from {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from)),
        execute: (self, ctx) {

        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        sat_transfer(ctx, &from, &to, &self.satoshi)?;
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    SatFromToTrs {
        name: "transfer_sat_from_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &SatFromToTrs| false,
        req_sign: |this: &SatFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &SatFromToTrs| format!("Transfer {} SAT from {} to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)?;
        Ok(vec![])
        
        }
    }
}

impl TransferLike for AssetToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Asset {
            serial: self.asset.serial.uint(),
            amount: self.asset.amount.uint(),
        }
    }
}

impl TransferLike for AssetFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Asset {
            serial: self.asset.serial.uint(),
            amount: self.asset.amount.uint(),
        }
    }
}

impl TransferLike for AssetFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Asset {
            serial: self.asset.serial.uint(),
            amount: self.asset.amount.uint(),
        }
    }
}

base::impl_action! {
    AssetToTrs {
        name: "transfer_asset_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &AssetToTrs| true,
        req_sign: |_: &AssetToTrs| vec![],
        as_transfer_like: self,
        description: |this: &AssetToTrs| format!("Transfer {{{}:{}}} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        asset_transfer(ctx, &from, &to, &self.asset)?;
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    AssetFromTrs {
        name: "transfer_asset_from",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &AssetFromTrs| true,
        req_sign: |this: &AssetFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &AssetFromTrs| format!("Transfer {{{}:{}}} from {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from)),
        execute: (self, ctx) {

        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        asset_transfer(ctx, &from, &to, &self.asset)?;
        Ok(vec![])
        
        }
    }
}

base::impl_action! {
    AssetFromToTrs {
        name: "transfer_asset_from_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &AssetFromToTrs| true,
        req_sign: |this: &AssetFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &AssetFromToTrs| format!("Transfer {{{}:{}}} from {} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

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

fn diamond_names_payload(diamonds: &DiamondNameListMax200) -> Vec<u8> {
    let encoded = diamonds.encode();
    encoded.get(1..).unwrap_or_default().to_vec()
}

impl TransferLike for DiaSingleTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: 1,
            names: self.diamond.to_vec(),
        }
    }
}

impl TransferLike for DiaToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: self.diamonds.length() as u32,
            names: diamond_names_payload(&self.diamonds),
        }
    }
}

impl TransferLike for DiaFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: self.diamonds.length() as u32,
            names: diamond_names_payload(&self.diamonds),
        }
    }
}

impl TransferLike for DiaFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: self.diamonds.length() as u32,
            names: diamond_names_payload(&self.diamonds),
        }
    }
}

base::impl_action! {
    DiaSingleTrs {
        name: "transfer_hacd_single_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaSingleTrs| false,
        req_sign: |_: &DiaSingleTrs| vec![],
        as_transfer_like: self,
        description: |this: &DiaSingleTrs| format!("Transfer 1 HACD ({}) to {}", this.diamond.to_readable(), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

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

base::impl_action! {
    DiaFromToTrs {
        name: "transfer_hacd_from_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaFromToTrs| false,
        req_sign: |this: &DiaFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &DiaFromToTrs| format!("Transfer {} HACD ({}) from {} to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        if is_privakey_unknown(&to) {
            return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
        }
        do_diamonds_transfer(ctx, &self.diamonds, &from, &to)
        
        }
    }
}

base::impl_action! {
    DiaToTrs {
        name: "transfer_hacd_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaToTrs| false,
        req_sign: |_: &DiaToTrs| vec![],
        as_transfer_like: self,
        description: |this: &DiaToTrs| format!("Transfer {} HACD ({}) to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.to)),
        execute: (self, ctx) {

        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        if is_privakey_unknown(&to) {
            return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
        }
        do_diamonds_transfer(ctx, &self.diamonds, &from, &to)
        
        }
    }
}

base::impl_action! {
    DiaFromTrs {
        name: "transfer_hacd_from",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaFromTrs| false,
        req_sign: |this: &DiaFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &DiaFromTrs| format!("Transfer {} HACD ({}) from {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from)),
        execute: (self, ctx) {

        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        do_diamonds_transfer(ctx, &self.diamonds, &from, &to)
        
        }
    }
}

pub fn create_hac_transfer(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    match kind {
        HacToTrs::KIND => decode_regular_action::<HacToTrs>(buf),
        HacFromTrs::KIND => decode_regular_action::<HacFromTrs>(buf),
        HacFromToTrs::KIND => decode_regular_action::<HacFromToTrs>(buf),
        _ => sys::normalf!("hac action kind {} not registered", kind),
    }
}

pub fn create_sat_transfer(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    match kind {
        SatToTrs::KIND => decode_regular_action::<SatToTrs>(buf),
        SatFromTrs::KIND => decode_regular_action::<SatFromTrs>(buf),
        SatFromToTrs::KIND => decode_regular_action::<SatFromToTrs>(buf),
        _ => sys::normalf!("sat action kind {} not registered", kind),
    }
}

pub fn create_asset_transfer(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    match kind {
        AssetToTrs::KIND => decode_regular_action::<AssetToTrs>(buf),
        AssetFromTrs::KIND => decode_regular_action::<AssetFromTrs>(buf),
        AssetFromToTrs::KIND => decode_regular_action::<AssetFromToTrs>(buf),
        _ => sys::normalf!("asset action kind {} not registered", kind),
    }
}

pub fn create_diamond_transfer(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    check_action_kind(kind, buf)?;
    match kind {
        DiaSingleTrs::KIND => {
            let (action, used) = DiaSingleTrs::decode(buf)?;
            DiamondName::check_bytes(action.diamond.as_ref())?;
            Ok((Arc::new(action), used))
        }
        DiaFromToTrs::KIND => {
            let (action, used) = DiaFromToTrs::decode(buf)?;
            action.diamonds.check()?;
            Ok((Arc::new(action), used))
        }
        DiaToTrs::KIND => {
            let (action, used) = DiaToTrs::decode(buf)?;
            action.diamonds.check()?;
            Ok((Arc::new(action), used))
        }
        DiaFromTrs::KIND => {
            let (action, used) = DiaFromTrs::decode(buf)?;
            action.diamonds.check()?;
            Ok((Arc::new(action), used))
        }
        _ => sys::normalf!("diamond action kind {} not registered", kind),
    }
}

/// JSON decoder for diamond transfers, retaining the list-level consensus
/// checks performed by the legacy API parser and binary creator.
pub fn decode_diamond_transfer_json(
    _reg: &dyn base::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef> {
    macro_rules! decode_checked {
        ($ty:ty) => {{
            let action = <$ty as ActionJsonCodec>::decode_json(json)?;
            action.diamonds.check()?;
            Ok(Arc::new(action) as ActionRef)
        }};
    }
    match kind {
        DiaFromToTrs::KIND => decode_checked!(DiaFromToTrs),
        DiaToTrs::KIND => decode_checked!(DiaToTrs),
        DiaFromTrs::KIND => decode_checked!(DiaFromTrs),
        DiaSingleTrs::KIND => {
            let action = DiaSingleTrs::decode_json(json)?;
            DiamondName::check_bytes(action.diamond.as_ref())?;
            Ok(Arc::new(action))
        }
        _ => sys::normalf!("diamond JSON action kind {} not registered", kind),
    }
}


#[cfg(feature = "execute")]
fn decode_regular_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + ActionExecute + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}

#[cfg(not(feature = "execute"))]
fn decode_regular_action<T>(buf: &[u8]) -> Ret<(ActionRef, usize)>
where
    T: Action + Decode + 'static,
{
    let (action, used) = T::decode(buf)?;
    Ok((Arc::new(action), used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_codec_round_trips_wire_and_json_fields() {
        let action = SatFromToTrs::new(Address::default(), Address::default(), Satoshi::from(7));
        let mut wire = action.encode();
        let action_size = wire.len();
        wire.extend_from_slice(&[0xaa, 0xbb]);
        let (decoded, used) = SatFromToTrs::decode(&wire).expect("decode action");
        assert_eq!(used, action_size);
        assert_eq!(decoded.encode(), wire[..action_size]);

        let json = action.to_json();
        assert_eq!(
            json,
            format!(
                "{{\"kind\":{},\"from\":{},\"to\":{},\"satoshi\":7}}",
                SatFromToTrs::KIND,
                Address::default().to_json(),
                Address::default().to_json(),
            )
        );

        let wrong_kind = SatToTrs::new(Address::default(), Satoshi::from(7)).encode();
        assert!(SatFromToTrs::decode(&wrong_kind).is_err());

        let decoded = <SatFromToTrs as base::ActionJsonCodec>::decode_json(&json)
            .expect("decode action json");
        assert_eq!(decoded.encode(), action.encode());
        assert!(
            <SatFromToTrs as base::ActionJsonCodec>::decode_json(
                "{\"kind\":12,\"from\":0,\"from\":0,\"to\":0,\"satoshi\":7}"
            )
            .is_err()
        );
        assert!(
            <SatFromToTrs as base::ActionJsonCodec>::decode_json(
                "{\"kind\":12,\"from\":0,\"to\":0}"
            )
            .is_err()
        );
    }
}
