use sys::{Ret, errf};

use crate::codec::{Decode, Encode, Reader};
use crate::types::address::Address;
use crate::types::amount::Amount;
use crate::types::asset::{AssetAmt, AssetAmtW1};
use crate::types::diamond::DiamondNumberAuto;
use crate::types::fold64::Fold64;
use crate::types::satoshi::SatoshiAuto;

pub const BALANCE_ASSET_MAX: usize = 20;

#[derive(Debug, Clone, Default, PartialEq, Eq, field::FieldCodec)]
#[field_codec(json_only, check = Balance::checked)]
pub struct Balance {
    pub hacash: Amount,
    pub satoshi: SatoshiAuto,
    pub diamond: DiamondNumberAuto,
    pub assets: AssetAmtW1,
}

impl Encode for Balance {
    fn size(&self) -> usize {
        self.hacash.size() + self.satoshi.size() + self.diamond.size() + self.assets.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.hacash.encode_to(out);
        self.satoshi.encode_to(out);
        self.diamond.encode_to(out);
        self.assets.encode_to(out);
    }
}

impl Decode for Balance {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let hacash: Amount = r.read()?;
        let satoshi: SatoshiAuto = r.read()?;
        let diamond: DiamondNumberAuto = r.read()?;
        let assets: AssetAmtW1 = r.read()?;
        Self::check_assets(&assets)?;
        Ok((
            Self {
                hacash,
                satoshi,
                diamond,
                assets,
            },
            r.used(),
        ))
    }
}

impl Balance {
    pub(crate) fn check_assets(assets: &AssetAmtW1) -> Ret<()> {
        if assets.length() > BALANCE_ASSET_MAX {
            return errf!(
                "balance asset item quantity cannot exceed {}",
                BALANCE_ASSET_MAX
            );
        }
        let mut seen: Vec<u64> = Vec::with_capacity(assets.length());
        for asset in assets.as_list() {
            asset.clone().checked()?;
            if seen.contains(&asset.serial.uint()) {
                return errf!("balance asset serial {} duplicated", asset.serial);
            }
            seen.push(asset.serial.uint());
        }
        Ok(())
    }

    pub fn checked(self) -> Ret<Self> {
        Self::check_assets(&self.assets).map(|()| self)
    }

    pub fn hac(amt: Amount) -> Self {
        Self {
            hacash: amt,
            ..Default::default()
        }
    }

    pub fn asset(&self, serial: Fold64) -> Option<AssetAmt> {
        self.assets.0.iter().find(|a| a.serial == serial).cloned()
    }

    pub fn asset_must(&self, serial: Fold64) -> Ret<AssetAmt> {
        match self.asset(serial) {
            Some(v) => Ok(v),
            None => AssetAmt::from_serial(serial),
        }
    }

    pub fn asset_set(&mut self, amt: AssetAmt) -> Ret<()> {
        let amt = if amt.amount.is_zero() {
            amt
        } else {
            amt.checked()?
        };
        for i in 0..self.assets.0.len() {
            if self.assets.0[i].serial == amt.serial {
                if amt.amount.is_zero() {
                    self.assets.0.remove(i);
                } else {
                    self.assets.0[i] = amt;
                }
                return Ok(());
            }
        }
        if !amt.amount.is_zero() {
            if self.assets.0.len() >= BALANCE_ASSET_MAX {
                return errf!(
                    "balance asset item quantity cannot exceed {}",
                    BALANCE_ASSET_MAX
                );
            }
            self.assets.push(amt)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct AddrHac {
    pub address: Address,
    pub amount: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct HacSat {
    pub amount: Amount,
    pub satoshi: SatoshiAuto,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct AddrHacSat {
    pub address: Address,
    pub hacsat: HacSat,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct AddrBalance {
    pub address: Address,
    pub balance: Balance,
}

#[cfg(test)]
mod json_tests {
    use super::*;
    use crate::json::{FromJSON, ToJSON};
    use crate::types::fold64::Fold64;

    #[test]
    fn balance_json_roundtrip_runs_check_assets() {
        let empty = Balance::default();
        let mut back = Balance::default();
        back.from_json(&empty.to_json()).unwrap();
        assert_eq!(back, empty);

        let mut dup = Balance::default();
        dup.assets = AssetAmtW1::from(vec![
            AssetAmt {
                serial: Fold64::from(1).unwrap(),
                amount: Fold64::from(1).unwrap(),
            },
            AssetAmt {
                serial: Fold64::from(1).unwrap(),
                amount: Fold64::from(2).unwrap(),
            },
        ])
        .unwrap();
        let mut parsed = Balance::default();
        assert!(parsed.from_json(&dup.to_json()).is_err());
    }
}
