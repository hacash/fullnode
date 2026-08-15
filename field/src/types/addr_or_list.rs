use sys::{Ret, normalf, errf};

use crate::codec::{Decode, Encode, ParsePrefix};
use crate::json::{FromJSON, JSONFormater, ToJSON, json_decode_array};
use crate::types::addr_or_ptr::ADDR_REF_MARKER_BASE;
use crate::types::address::Address;
use crate::types::list::AddressW1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrOrList {
    Single(Address),
    List(AddressW1),
}

impl Default for AddrOrList {
    fn default() -> Self {
        Self::Single(Address::default())
    }
}

impl AddrOrList {
    pub fn to_list(&self) -> Vec<Address> {
        match self {
            Self::Single(v) => vec![*v],
            Self::List(v) => v.as_list().clone(),
        }
    }

    pub fn from_addr(v: Address) -> Self {
        Self::Single(v)
    }

    pub fn from_list(list: Vec<Address>) -> Ret<Self> {
        if list.is_empty() {
            return errf!("AddrOrList list cannot be empty");
        }
        let max_count = u8::MAX as usize - ADDR_REF_MARKER_BASE as usize;
        if list.len() > max_count {
            return errf!(
                "AddrOrList list length {} exceeds max {}",
                list.len(),
                max_count
            );
        }
        Ok(Self::List(AddressW1::from(list)?))
    }
}

impl Encode for AddrOrList {
    fn size(&self) -> usize {
        match self {
            Self::Single(v) => v.size(),
            Self::List(v) => v.size(),
        }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        match self {
            Self::Single(v) => v.encode_to(out),
            Self::List(v) => {
                let start = out.len();
                v.encode_to(out);
                out[start] = out[start]
                    .checked_add(ADDR_REF_MARKER_BASE)
                    .expect("AddrOrList list marker overflow");
            }
        }
    }
}

impl Decode for AddrOrList {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        if buf.is_empty() {
            return normalf!("buffer too short for AddrOrList");
        }
        if buf[0] < ADDR_REF_MARKER_BASE {
            let (v, used) = Address::decode(buf)?;
            return Ok((Self::Single(v), used));
        }

        let count = buf[0] - ADDR_REF_MARKER_BASE;
        if count == 0 {
            return normalf!("AddrOrList list cannot be empty");
        }
        let (list, used) = AddressW1::create_with_prefix(&[count], &buf[1..])?;
        Ok((Self::List(list), used))
    }
}

impl ToJSON for AddrOrList {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        match self {
            Self::Single(v) => v.to_json_fmt(fmt),
            Self::List(v) => v.to_json_fmt(fmt),
        }
    }
}

impl FromJSON for AddrOrList {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let json = json.trim();
        if json.starts_with('"') && json.ends_with('"') {
            let mut addr = Address::default();
            addr.from_json(json)?;
            *self = Self::Single(addr);
            return Ok(());
        }

        let (items, count) = json_decode_array(json)?;
        if count == 0 {
            return errf!("invalid AddrOrList JSON: list length cannot be zero");
        }
        let max_count = u8::MAX as usize - ADDR_REF_MARKER_BASE as usize;
        if count > max_count {
            return errf!(
                "invalid AddrOrList JSON: list length {} exceeds max {}",
                count,
                max_count
            );
        }
        let mut list = Vec::with_capacity(count);
        for item in items {
            let mut addr = Address::default();
            addr.from_json(&item)?;
            list.push(addr);
        }
        *self = Self::from_list(list)?;
        Ok(())
    }
}
