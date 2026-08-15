use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fmt::*;
use std::ops::Deref;
use std::rc::*;

use field::{Decode, Encode, FromJSON, JSONFormater, ToJSON};
use ripemd::{Digest, Ripemd160};
use sys::*;

use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::space::*;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractAddress {
    addr: field::Address,
}

impl ContractAddress {
    pub const WIDTH: usize = field::Address::SIZE;

    pub fn calculate(addr: &field::Address, nonce: &field::Uint4) -> Self {
        let mut data = Vec::with_capacity(field::Encode::size(addr) + field::Encode::size(nonce));
        addr.encode_to(&mut data);
        nonce.encode_to(&mut data);
        let hash = sys::calculate_hash(data);
        let digest = Ripemd160::digest(hash);
        let mut raw = [0u8; field::Address::SIZE];
        raw[0] = 1;
        raw[1..].copy_from_slice(&digest);
        Self::from_addr(field::Address::from(raw)).unwrap()
    }

    pub fn must(bytes: [u8; field::Address::SIZE]) -> Self {
        Self::from_addr(field::Address::from(bytes)).unwrap()
    }

    pub fn from_unchecked(addr: field::Address) -> Self {
        Self { addr }
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&self) -> Rerr {
        if !self.addr.is_contract() {
            return errf!("address version {} is not CONTRACT", self.addr.version());
        }
        Ok(())
    }

    pub fn from_addr(addr: field::Address) -> Ret<Self> {
        if !addr.is_contract() {
            return errf!("address version {} is not CONTRACT", addr.version());
        }
        Ok(Self { addr })
    }

    pub fn to_addr(&self) -> field::Address {
        self.addr
    }

    pub fn into_addr(self) -> field::Address {
        self.addr
    }

    pub fn parse(bytes: &[u8]) -> Ret<Self> {
        let (addr, used) = field::Address::decode(bytes)?;
        if used != field::Address::SIZE {
            return normalf!("contract address size invalid");
        }
        Self::from_addr(addr)
    }

    pub fn to_readable(&self) -> String {
        self.addr.to_readable()
    }

    pub fn readable(&self) -> String {
        self.to_readable()
    }
}

impl Deref for ContractAddress {
    type Target = field::Address;

    fn deref(&self) -> &Self::Target {
        &self.addr
    }
}

impl Display for ContractAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        std::fmt::Display::fmt(&self.addr.to_readable(), f)
    }
}

impl Encode for ContractAddress {
    fn size(&self) -> usize {
        field::Address::SIZE
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.addr.encode_to(out);
    }
}

impl ToJSON for ContractAddress {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        self.addr.to_json_fmt(fmt)
    }
}

impl FromJSON for ContractAddress {
    fn from_json(&mut self, json: &str) -> sys::Ret<()> {
        let mut addr = self.addr;
        addr.from_json(json)?;
        self.addr = addr;
        self.check()
    }
}

impl Decode for ContractAddress {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let (addr, used) = field::Address::decode(buf)?;
        Ok((Self::from_addr(addr)?, used))
    }
}

fn address_from_bytes(buf: &[u8]) -> Ret<field::Address> {
    if buf.len() != field::Address::SIZE {
        return errf!(
            "address bytes length {} invalid, expected {}",
            buf.len(),
            field::Address::SIZE
        );
    }
    Ok(field::Address::decode(buf)?.0)
}

pub const REF_DUP_SIZE: usize = 8;

mod handle;
pub use handle::*;

include!("util.rs");
include!("list.rs");
include!("convert.rs");
include!("compo.rs");
include!("tuple.rs");
include!("canbe.rs");
include!("type.rs");
include!("item.rs");
include!("cast.rs");
include!("operand.rs");
include!("field.rs");
