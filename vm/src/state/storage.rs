use field::*;
use field::{BlockHeight, Uint4};
use sys::Ret;

use crate::rt::{GasExtra, ItrErr, ItrErrCode::*, VmrtErr, VmrtRes};
use crate::value::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValueSto {
    pub charge: BlockHeight,
    pub live_credit: Uint4,
    pub recover_credit: Uint4,
    pub data: Value,
}

impl ValueSto {
    pub fn credit_u32(v: u64, tip: &str) -> VmrtRes<u32> {
        u32::try_from(v).map_err(|_| ItrErr::new(StorageError, tip))
    }

    pub fn new(chei: u64, data: Value, live_credit: u64, recover_credit: u64) -> VmrtRes<Self> {
        Ok(Self {
            charge: BlockHeight::from(chei),
            live_credit: Uint4::from(Self::credit_u32(live_credit, "live credit overflow")?),
            recover_credit: Uint4::from(Self::credit_u32(
                recover_credit,
                "recover credit overflow",
            )?),
            data,
        })
    }

    pub fn unit_for(gst: &GasExtra, v: &Value) -> VmrtRes<u64> {
        Ok((v.can_get_size()? as u64).saturating_add(gst.storege_value_base_size.max(0) as u64))
    }

    pub fn unit(&self, gst: &GasExtra) -> VmrtRes<u64> {
        Self::unit_for(gst, &self.data)
    }

    pub fn is_active(&self) -> bool {
        self.live_credit.uint() > 0
    }

    pub fn is_recoverable(&self) -> bool {
        self.live_credit.uint() == 0 && self.recover_credit.uint() > 0
    }

    pub fn is_absent(&self) -> bool {
        self.live_credit.uint() == 0 && self.recover_credit.uint() == 0
    }

    pub fn settle(&mut self, curhei: u64, gst: &GasExtra) -> VmrtErr {
        let unit = self.unit(gst)?;
        if unit == 0 {
            self.charge = BlockHeight::from(curhei);
            return Ok(());
        }
        let old = self.charge.uint();
        if curhei <= old {
            return Ok(());
        }
        let elapsed = (curhei - old) as u128;
        let unit = unit as u128;
        let mut burn = elapsed.saturating_mul(unit);

        let mut live = self.live_credit.uint() as u128;
        if burn >= live {
            burn -= live;
            live = 0;
        } else {
            live -= burn;
            burn = 0;
        }

        let mut recover = self.recover_credit.uint() as u128;
        if burn >= recover {
            recover = 0;
        } else {
            recover -= burn;
        }

        self.live_credit = Uint4::from(Self::credit_u32(
            live.min(u64::MAX as u128) as u64,
            "live credit overflow",
        )?);
        self.recover_credit = Uint4::from(Self::credit_u32(
            recover.min(u64::MAX as u128) as u64,
            "recover credit overflow",
        )?);
        self.charge = BlockHeight::from(curhei);
        Ok(())
    }

    pub fn live_rest_blocks(&self, gst: &GasExtra) -> VmrtRes<u64> {
        let unit = self.unit(gst)?;
        rest_blocks(self.live_credit.uint() as u64, unit)
    }

    pub fn recover_rest_blocks(&self, gst: &GasExtra) -> VmrtRes<u64> {
        let unit = self.unit(gst)?;
        rest_blocks(self.recover_credit.uint() as u64, unit)
    }
}

impl Encode for ValueSto {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.size());
        self.encode_to(&mut out);
        out
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.charge.encode_to(out);
        self.live_credit.encode_to(out);
        self.recover_credit.encode_to(out);
        self.data.encode_to(out);
    }

    fn size(&self) -> usize {
        field::Encode::size(&self.charge)
            + field::Encode::size(&self.live_credit)
            + field::Encode::size(&self.recover_credit)
            + self.data.size()
    }
}

impl Decode for ValueSto {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut cur = buf;
        let mut used = 0usize;
        let (charge, n) = BlockHeight::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (live_credit, n) = Uint4::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (recover_credit, n) = Uint4::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (data, n) = Value::decode(cur)?;
        used += n;
        Ok((
            Self {
                charge,
                live_credit,
                recover_credit,
                data,
            },
            used,
        ))
    }
}

pub fn parse_period(v: Value, max_period: u64) -> VmrtRes<u64> {
    let period = v.extract_u128().map_err(|_| {
        ItrErr::new(
            StorageError,
            &format!("period value {:?} is not uint type", v),
        )
    })?;
    if period < 1 {
        return itr_err_fmt!(StoragePeriodErr, "period min is 1");
    }
    if period > max_period as u128 {
        return itr_err_fmt!(
            StoragePeriodErr,
            "period value max is {} but got {}",
            max_period,
            period
        );
    }
    Ok(period as u64)
}

pub fn period_credit(unit: u64, period: u64, storage_period: u64) -> VmrtRes<u64> {
    if storage_period == 0 {
        return itr_err_code!(StoragePeriodErr);
    }
    let blocks = period
        .checked_mul(storage_period)
        .ok_or_else(|| ItrErr::new(StorageError, "period blocks overflow"))?;
    let credit = (unit as u128)
        .checked_mul(blocks as u128)
        .ok_or_else(|| ItrErr::new(StorageError, "credit overflow"))?;
    if credit > u64::MAX as u128 {
        return itr_err_fmt!(StorageError, "credit overflow");
    }
    Ok(credit as u64)
}

pub fn u64_to_i64_sat(v: u64) -> i64 {
    v.min(i64::MAX as u64) as i64
}

pub fn rest_blocks(credit: u64, unit: u64) -> VmrtRes<u64> {
    if unit == 0 {
        return itr_err_fmt!(StorageError, "storage unit cannot be zero");
    }
    if credit == 0 {
        Ok(0)
    } else {
        Ok(credit.saturating_sub(1) / unit + 1)
    }
}

pub fn refund_for_live_credit(credit: u64, storage_period: u64) -> i64 {
    u64_to_i64_sat(credit.checked_div(storage_period).unwrap_or(0))
}

pub fn credit_cap_for_blocks(unit: u64, blocks: u64, tip: &str) -> VmrtRes<u64> {
    let credit = (unit as u128)
        .checked_mul(blocks as u128)
        .ok_or_else(|| ItrErr::new(StorageError, tip))?;
    if credit > u64::MAX as u128 {
        return itr_err_fmt!(StorageError, "{}", tip);
    }
    Ok(credit as u64)
}

pub fn clamp_credit_to_cap(credit: u64, cap: u64) -> (u64, u64) {
    let next = credit.min(cap);
    let trimmed = credit.saturating_sub(next);
    (next, trimmed)
}
