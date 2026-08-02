//! Hac / Sat / Asset / Diamond transfer actions.

use std::any::Any;
use std::sync::Arc;

use base::{
    ActOut, ActScope, Action, ActionRef, AddrOrPtr, Context, CoreState, TransferLike,
    TransferPayload, asset_transfer, diamond_owned_move, hac_transfer, hacd_move_one_diamond,
    hacd_transfer, sat_transfer,
};
use field::{
    Address, Amount, AssetAmt, DiamondName, DiamondNameListMax200, DiamondNumber, Encode, Reader,
    Satoshi, ToJSON, Uint2,
};
use sys::Ret;

use super::common::{addr_or_ptr_size, decode_addr_or_ptr, encode_addr_or_ptr};

fn check_transfer_addresses(ctx: &dyn Context, from: &Address, to: &Address) -> Ret<()> {
    crate::upgrade::check_transfer_addr_online_open(
        ctx.env().chain.id,
        ctx.env().block.height,
        from,
        to,
    )
}

#[derive(Debug, Clone)]
pub struct HacToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub hacash: Amount,
}

pub type HacTransfer = HacToTrs;

#[derive(Debug, Clone)]
pub struct HacFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub hacash: Amount,
}

#[derive(Debug, Clone)]
pub struct HacFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub hacash: Amount,
}

#[derive(Debug, Clone)]
pub struct SatToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone)]
pub struct SatFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone)]
pub struct SatFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone)]
pub struct AssetToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone)]
pub struct AssetFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone)]
pub struct AssetFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone)]
pub struct DiaSingleTrs {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub to: AddrOrPtr,
}

#[derive(Debug, Clone)]
pub struct DiaFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

#[derive(Debug, Clone)]
pub struct DiaToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

#[derive(Debug, Clone)]
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

impl Encode for HacToTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.to) + self.hacash.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.to, out);
        self.hacash.encode_to(out);
    }
}

impl Encode for HacFromTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.from) + self.hacash.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        self.hacash.encode_to(out);
    }
}

impl Encode for HacFromToTrs {
    fn size(&self) -> usize {
        self.kind.size()
            + addr_or_ptr_size(&self.from)
            + addr_or_ptr_size(&self.to)
            + self.hacash.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        encode_addr_or_ptr(&self.to, out);
        self.hacash.encode_to(out);
    }
}

impl Encode for SatToTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.to) + self.satoshi.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.to, out);
        self.satoshi.encode_to(out);
    }
}

impl Encode for SatFromTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.from) + self.satoshi.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        self.satoshi.encode_to(out);
    }
}

impl Encode for SatFromToTrs {
    fn size(&self) -> usize {
        self.kind.size()
            + addr_or_ptr_size(&self.from)
            + addr_or_ptr_size(&self.to)
            + self.satoshi.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        encode_addr_or_ptr(&self.to, out);
        self.satoshi.encode_to(out);
    }
}

impl Encode for AssetToTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.to) + self.asset.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.to, out);
        self.asset.encode_to(out);
    }
}

impl Encode for AssetFromTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.from) + self.asset.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        self.asset.encode_to(out);
    }
}

impl Encode for AssetFromToTrs {
    fn size(&self) -> usize {
        self.kind.size()
            + addr_or_ptr_size(&self.from)
            + addr_or_ptr_size(&self.to)
            + self.asset.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        encode_addr_or_ptr(&self.to, out);
        self.asset.encode_to(out);
    }
}

impl Encode for DiaSingleTrs {
    fn size(&self) -> usize {
        self.kind.size() + self.diamond.size() + addr_or_ptr_size(&self.to)
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.diamond.encode_to(out);
        encode_addr_or_ptr(&self.to, out);
    }
}

impl Encode for DiaFromToTrs {
    fn size(&self) -> usize {
        self.kind.size()
            + addr_or_ptr_size(&self.from)
            + addr_or_ptr_size(&self.to)
            + self.diamonds.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        encode_addr_or_ptr(&self.to, out);
        self.diamonds.encode_to(out);
    }
}

impl Encode for DiaToTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.to) + self.diamonds.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.to, out);
        self.diamonds.encode_to(out);
    }
}

impl Encode for DiaFromTrs {
    fn size(&self) -> usize {
        self.kind.size() + addr_or_ptr_size(&self.from) + self.diamonds.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        encode_addr_or_ptr(&self.from, out);
        self.diamonds.encode_to(out);
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

impl Action for HacToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        check_transfer_addresses(ctx, &from, &to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for HacFromTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        check_transfer_addresses(ctx, &from, &to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for HacFromToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        check_transfer_addresses(ctx, &from, &to)?;
        hac_transfer(ctx, &from, &to, &self.hacash)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for SatToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        check_transfer_addresses(ctx, &from, &to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for SatFromTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        check_transfer_addresses(ctx, &from, &to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for SatFromToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        check_transfer_addresses(ctx, &from, &to)?;
        sat_transfer(ctx, &from, &to, &self.satoshi)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
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

impl Action for AssetToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn extra9(&self) -> bool {
        true
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        check_transfer_addresses(ctx, &from, &to)?;
        asset_transfer(ctx, &from, &to, &self.asset)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for AssetFromTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn extra9(&self) -> bool {
        true
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        check_transfer_addresses(ctx, &from, &to)?;
        asset_transfer(ctx, &from, &to, &self.asset)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for AssetFromToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn extra9(&self) -> bool {
        true
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        check_transfer_addresses(ctx, &from, &to)?;
        asset_transfer(ctx, &from, &to, &self.asset)?;
        Ok((gas, vec![]))
    }
    fn as_any(&self) -> &dyn Any {
        self
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
    check_transfer_addresses(ctx, from, to)?;
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

impl Action for DiaSingleTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        if is_privakey_unknown(&to) {
            return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
        }
        let diamonds = DiamondNameListMax200::one(self.diamond);
        let ret = do_diamonds_transfer(ctx, &diamonds, &from, &to)?;
        Ok((gas, ret))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for DiaFromToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.addr(&self.to)?;
        if is_privakey_unknown(&to) {
            return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
        }
        let ret = do_diamonds_transfer(ctx, &self.diamonds, &from, &to)?;
        Ok((gas, ret))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for DiaToTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.env().tx.main;
        let to = ctx.addr(&self.to)?;
        if is_privakey_unknown(&to) {
            return sys::errf!("cannot transfer diamond to system address {}", to.to_json());
        }
        let ret = do_diamonds_transfer(ctx, &self.diamonds, &from, &to)?;
        Ok((gas, ret))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Action for DiaFromTrs {
    fn kind(&self) -> u16 {
        Self::KIND
    }
    fn scope(&self) -> ActScope {
        ActScope::CALL
    }
    fn min_tx_type(&self) -> u8 {
        2
    }
    fn req_sign(&self) -> Vec<AddrOrPtr> {
        vec![self.from.clone()]
    }
    fn as_transfer_like(&self) -> Option<&dyn TransferLike> {
        Some(self)
    }
    fn execute(&self, ctx: &mut dyn Context) -> Ret<ActOut> {
        let gas = self.size() as u32;
        let from = ctx.addr(&self.from)?;
        let to = ctx.env().tx.main;
        let ret = do_diamonds_transfer(ctx, &self.diamonds, &from, &to)?;
        Ok((gas, ret))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn create_hac_transfer(
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
        HacToTrs::KIND => {
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let hacash: Amount = r.read()?;
            Ok((
                Arc::new(HacToTrs {
                    kind: kind_field,
                    to,
                    hacash,
                }),
                r.used(),
            ))
        }
        HacFromTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let hacash: Amount = r.read()?;
            Ok((
                Arc::new(HacFromTrs {
                    kind: kind_field,
                    from,
                    hacash,
                }),
                r.used(),
            ))
        }
        HacFromToTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let hacash: Amount = r.read()?;
            Ok((
                Arc::new(HacFromToTrs {
                    kind: kind_field,
                    from,
                    to,
                    hacash,
                }),
                r.used(),
            ))
        }
        _ => sys::decodef!("hac action kind {} not registered", kind),
    }
}

pub fn create_sat_transfer(
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
        SatToTrs::KIND => {
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let satoshi: Satoshi = r.read()?;
            Ok((
                Arc::new(SatToTrs {
                    kind: kind_field,
                    to,
                    satoshi,
                }),
                r.used(),
            ))
        }
        SatFromTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let satoshi: Satoshi = r.read()?;
            Ok((
                Arc::new(SatFromTrs {
                    kind: kind_field,
                    from,
                    satoshi,
                }),
                r.used(),
            ))
        }
        SatFromToTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let satoshi: Satoshi = r.read()?;
            Ok((
                Arc::new(SatFromToTrs {
                    kind: kind_field,
                    from,
                    to,
                    satoshi,
                }),
                r.used(),
            ))
        }
        _ => sys::decodef!("sat action kind {} not registered", kind),
    }
}

pub fn create_asset_transfer(
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
        AssetToTrs::KIND => {
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let asset: AssetAmt = r.read()?;
            Ok((
                Arc::new(AssetToTrs {
                    kind: kind_field,
                    to,
                    asset,
                }),
                r.used(),
            ))
        }
        AssetFromTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let asset: AssetAmt = r.read()?;
            Ok((
                Arc::new(AssetFromTrs {
                    kind: kind_field,
                    from,
                    asset,
                }),
                r.used(),
            ))
        }
        AssetFromToTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let asset: AssetAmt = r.read()?;
            Ok((
                Arc::new(AssetFromToTrs {
                    kind: kind_field,
                    from,
                    to,
                    asset,
                }),
                r.used(),
            ))
        }
        _ => sys::decodef!("asset action kind {} not registered", kind),
    }
}

pub fn create_diamond_transfer(
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
        DiaSingleTrs::KIND => {
            let diamond: DiamondName = r.read()?;
            DiamondName::check_bytes(diamond.as_ref())?;
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            Ok((
                Arc::new(DiaSingleTrs {
                    kind: kind_field,
                    diamond,
                    to,
                }),
                r.used(),
            ))
        }
        DiaFromToTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let diamonds: DiamondNameListMax200 = r.read()?;
            diamonds.check()?;
            Ok((
                Arc::new(DiaFromToTrs {
                    kind: kind_field,
                    from,
                    to,
                    diamonds,
                }),
                r.used(),
            ))
        }
        DiaToTrs::KIND => {
            let (to, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let diamonds: DiamondNameListMax200 = r.read()?;
            diamonds.check()?;
            Ok((
                Arc::new(DiaToTrs {
                    kind: kind_field,
                    to,
                    diamonds,
                }),
                r.used(),
            ))
        }
        DiaFromTrs::KIND => {
            let (from, used) = decode_addr_or_ptr(&buf[r.used()..])?;
            let _ = r.read_bytes(used)?;
            let diamonds: DiamondNameListMax200 = r.read()?;
            diamonds.check()?;
            Ok((
                Arc::new(DiaFromTrs {
                    kind: kind_field,
                    from,
                    diamonds,
                }),
                r.used(),
            ))
        }
        _ => sys::decodef!("diamond action kind {} not registered", kind),
    }
}
