use sys::{Ret, errf};

use crate::codec::{Decode, Encode, Reader};
use crate::types::address::Address;
use crate::types::bytes_w::BytesW1;
use crate::types::fold64::Fold64;
use crate::types::list::ListW1;
use crate::types::uint::Uint1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetAmt {
    pub serial: Fold64,
    pub amount: Fold64,
}

impl Encode for AssetAmt {
    fn size(&self) -> usize {
        self.serial.size() + self.amount.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.serial.encode_to(out);
        self.amount.encode_to(out);
    }
}

impl Decode for AssetAmt {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut reader = Reader::new(buf);
        let serial: Fold64 = reader.read()?;
        let amount: Fold64 = reader.read()?;
        let asset = Self { serial, amount }.checked()?;
        Ok((asset, reader.used()))
    }
}

impl AssetAmt {
    pub fn checked(self) -> Ret<Self> {
        if self.serial.is_zero() {
            return errf!("AssetAmt.serial cannot be zero");
        }
        Ok(Self {
            serial: self.serial.checked()?,
            amount: self.amount.checked()?,
        })
    }

    pub fn from_serial(serial: Fold64) -> Ret<Self> {
        Self {
            serial,
            ..Default::default()
        }
        .checked()
    }

    pub fn checked_add(&self, other: &Self) -> Ret<Self> {
        if self.serial != other.serial {
            return errf!("cannot add asset {} and {}", self.serial, other.serial);
        }
        let amount = self
            .amount
            .uint()
            .checked_add(other.amount.uint())
            .ok_or_else(|| sys::Error::fault("asset amount add overflow"))?;
        Self {
            serial: self.serial,
            amount: Fold64::from(amount)?,
        }
        .checked()
    }

    pub fn checked_sub(&self, other: &Self) -> Ret<Self> {
        if self.serial != other.serial {
            return errf!("cannot sub asset {} and {}", self.serial, other.serial);
        }
        let amount = self
            .amount
            .uint()
            .checked_sub(other.amount.uint())
            .ok_or_else(|| sys::Error::revert("asset amount is insufficient"))?;
        Self {
            serial: self.serial,
            amount: Fold64::from(amount)?,
        }
        .checked()
    }
}

impl PartialOrd for AssetAmt {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.serial != other.serial {
            return None;
        }
        Some(self.amount.cmp(&other.amount))
    }
}

pub type AssetAmtW1 = ListW1<AssetAmt>;

codec_struct!(AssetSmelt {
    serial: Fold64,
    supply: Fold64,
    decimal: Uint1,
    issuer: Address,
    ticket: BytesW1,
    name: BytesW1,
});
