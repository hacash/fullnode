use sys::{Ret, normalf, errf};

use crate::codec::{Decode, Encode};
use crate::json::{FromJSON, JSONFormater, ToJSON, json_expect_unquoted};
use crate::types::address::Address;

/// Wire discriminator for an inline address versus a compact address-list reference.
/// This is consensus encoding and must remain 20.
pub const ADDR_REF_MARKER_BASE: u8 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrOrPtr {
    Addr(Address),
    Ptr(u8),
}

impl Default for AddrOrPtr {
    fn default() -> Self {
        Self::Addr(Address::default())
    }
}

impl From<Address> for AddrOrPtr {
    fn from(addr: Address) -> Self {
        Self::Addr(addr)
    }
}

impl AddrOrPtr {
    pub const MAX_INDEX: u8 = u8::MAX - ADDR_REF_MARKER_BASE;

    pub fn from_addr(addr: Address) -> Self {
        Self::Addr(addr)
    }

    pub fn from_ptr(index: u8) -> Self {
        assert!(index <= Self::MAX_INDEX, "AddrOrPtr index overflow");
        Self::Ptr(index)
    }

    pub fn real(&self, addrs: &[Address]) -> Ret<Address> {
        match self {
            Self::Addr(addr) => Ok(*addr),
            Self::Ptr(index) => addrs
                .get(*index as usize)
                .copied()
                .map_or_else(|| errf!("addr ptr index {} out of range", index), Ok),
        }
    }
}

impl Encode for AddrOrPtr {
    fn size(&self) -> usize {
        match self {
            Self::Addr(addr) => addr.size(),
            Self::Ptr(_) => 1,
        }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        match self {
            Self::Addr(addr) => addr.encode_to(out),
            Self::Ptr(index) => out.push(
                index
                    .checked_add(ADDR_REF_MARKER_BASE)
                    .expect("AddrOrPtr index overflow"),
            ),
        }
    }
}

impl Decode for AddrOrPtr {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let Some(&first) = buf.first() else {
            return normalf!("buffer too short for AddrOrPtr");
        };
        if first < ADDR_REF_MARKER_BASE {
            let (addr, used) = Address::decode(buf)?;
            return Ok((Self::Addr(addr), used));
        }
        Ok((Self::Ptr(first - ADDR_REF_MARKER_BASE), 1))
    }
}

impl ToJSON for AddrOrPtr {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        match self {
            Self::Addr(addr) => addr.to_json_fmt(fmt),
            Self::Ptr(index) => index.to_string(),
        }
    }
}

impl FromJSON for AddrOrPtr {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let raw = json.trim();
        if raw.starts_with('"') {
            let mut addr = Address::default();
            addr.from_json(raw)?;
            *self = Self::Addr(addr);
            return Ok(());
        }
        let index: u16 = json_expect_unquoted(raw)?
            .parse()
            .map_err(|_| sys::Error::normal("cannot parse AddrOrPtr"))?;
        if index > Self::MAX_INDEX as u16 {
            return errf!("AddrOrPtr index {} exceeds max {}", index, Self::MAX_INDEX);
        }
        *self = Self::Ptr(index as u8);
        Ok(())
    }
}
