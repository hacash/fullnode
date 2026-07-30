use sys::{Ret, decodef, errf};

use crate::codec::{Decode, Encode};
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
            return decodef!("buffer too short for AddrOrPtr");
        };
        if first < ADDR_REF_MARKER_BASE {
            let (addr, used) = Address::decode(buf)?;
            return Ok((Self::Addr(addr), used));
        }
        Ok((Self::Ptr(first - ADDR_REF_MARKER_BASE), 1))
    }
}
