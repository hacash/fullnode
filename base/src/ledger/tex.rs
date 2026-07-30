use std::collections::HashSet;

use field::{Address, AssetAmt, DiamondName, DiamondNameListMax200, Fold64, Hash};
use sys::{Ret, errf};

#[derive(Clone, Default, Debug)]
pub struct TexLedger {
    pub zhu: i64,
    pub sat: i64,
    pub dia: i32,
    /// Diamond names paid into the settlement pool (FIFO for gets).
    pub diamonds: Vec<Hash>,
    pub diatrs: Vec<(Address, usize)>,
    pub entries: Vec<TexEntry>,
    pub asset_checked: HashSet<u64>,
}

impl TexLedger {
    pub fn record_diamond_pay(
        &mut self,
        names: &DiamondNameListMax200,
        max_diamonds: usize,
    ) -> Ret<()> {
        let n = names.length() as i32;
        let Some(newdia) = self.dia.checked_add(n) else {
            return errf!("cell state diamond record overflow");
        };
        let Some(new_len) = self.diamonds.len().checked_add(names.length()) else {
            return errf!("tex diamond pay record overflow");
        };
        if new_len > max_diamonds {
            return errf!("tex diamond pay exceeds {} diamonds", max_diamonds);
        }
        for name in names.as_list() {
            self.diamonds.push(diamond_name_as_hash(name));
        }
        self.dia = newdia;
        Ok(())
    }

    pub fn record_diamond_get(
        &mut self,
        addr: Address,
        count: usize,
        max_diamonds: usize,
    ) -> Ret<()> {
        if count == 0 {
            return errf!("tex diamond_get count cannot be zero");
        }
        if count > max_diamonds {
            return errf!("Tex state diamond trs num cannot exceed {}", max_diamonds);
        }
        let Some(diares) = self.dia.checked_sub(count as i32) else {
            return errf!("cell state diamond overflow");
        };
        self.dia = diares;
        self.diatrs.push((addr, count));
        Ok(())
    }

    pub fn record_asset_pay(&mut self, asset: &AssetAmt) -> Ret<()> {
        self.record_asset_delta(
            Address::default(),
            asset.serial.uint(),
            asset.amount.uint() as i128,
        )
    }

    pub fn record_asset_get(&mut self, addr: Address, asset: &AssetAmt) -> Ret<()> {
        self.record_asset_delta(addr, asset.serial.uint(), -(asset.amount.uint() as i128))
    }

    pub fn record_asset_delta(&mut self, addr: Address, asset_serial: u64, delta: i128) -> Ret<()> {
        if asset_serial == 0 {
            return errf!("tex asset serial cannot be zero");
        }
        self.entries.push(TexEntry {
            addr,
            asset_serial,
            delta,
        });
        Ok(())
    }

    pub fn mark_asset_checked(&mut self, serial: Fold64) {
        self.asset_checked.insert(serial.uint());
    }

    pub fn asset_is_checked(&self, serial: Fold64) -> bool {
        self.asset_checked.contains(&serial.uint())
    }
}

fn diamond_name_as_hash(name: &DiamondName) -> Hash {
    let mut buf = [0u8; 32];
    buf[..DiamondName::SIZE].copy_from_slice(name.as_ref());
    Hash::from(buf)
}

#[derive(Clone, Debug)]
pub struct TexEntry {
    pub addr: Address,
    pub asset_serial: u64,
    pub delta: i128,
}
