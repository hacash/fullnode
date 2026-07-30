use field::{Address, Decode};
use sys::Ret;

use crate::rt::{ItrErr, ItrErrCode::LogError, VmrtRes};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmLog {
    pub addr: Address,
    pub topic0: Value,
    pub topic1: Value,
    pub topic2: Value,
    pub topic3: Value,
    pub data: Value,
}

impl VmLog {
    pub fn from_bytes(buf: &[u8]) -> VmrtRes<Self> {
        let (log, _) = VmLog::decode(buf).map_err(|e| ItrErr::new(LogError, &e.to_string()))?;
        Ok(log)
    }

    pub fn new(addr: Address, tds: Vec<Value>) -> VmrtRes<Self> {
        let tl = tds.len();
        if tl < 2 {
            return itr_err_fmt!(LogError, "argv num must be at least 2");
        }
        if tl > 5 {
            return itr_err_fmt!(LogError, "argv num must be at most 5");
        }
        for a in &tds {
            a.check_scalar()?;
        }
        let mut log = Self {
            addr,
            topic0: tds[0].clone(),
            topic1: Value::nil(),
            topic2: Value::nil(),
            topic3: Value::nil(),
            data: tds[tl - 1].clone(),
        };
        match tl {
            2 => {}
            3 => log.topic1 = tds[1].clone(),
            4 => {
                log.topic1 = tds[1].clone();
                log.topic2 = tds[2].clone();
            }
            5 => {
                log.topic1 = tds[1].clone();
                log.topic2 = tds[2].clone();
                log.topic3 = tds[3].clone();
            }
            _ => unreachable!(),
        }
        Ok(log)
    }

    pub fn render(&self) -> String {
        format!(
            r#""address":"{}","topic0":"{}","topic1":"{}","topic2":"{}","topic3":"{}","data":"{}""#,
            self.addr.to_readable(),
            hex::encode(self.topic0.raw()),
            hex::encode(self.topic1.raw()),
            hex::encode(self.topic2.raw()),
            hex::encode(self.topic3.raw()),
            hex::encode(self.data.raw()),
        )
    }
}

impl field::Encode for VmLog {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.size());
        self.encode_to(&mut out);
        out
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.addr.encode_to(out);
        self.topic0.encode_to(out);
        self.topic1.encode_to(out);
        self.topic2.encode_to(out);
        self.topic3.encode_to(out);
        self.data.encode_to(out);
    }

    fn size(&self) -> usize {
        field::Encode::size(&self.addr)
            + self.topic0.size()
            + self.topic1.size()
            + self.topic2.size()
            + self.topic3.size()
            + self.data.size()
    }
}

impl field::Decode for VmLog {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut cur = buf;
        let mut used = 0usize;
        let (addr, n) = Address::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (topic0, n) = Value::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (topic1, n) = Value::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (topic2, n) = Value::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (topic3, n) = Value::decode(cur)?;
        cur = &cur[n..];
        used += n;
        let (data, n) = Value::decode(cur)?;
        used += n;
        let log = Self {
            addr,
            topic0,
            topic1,
            topic2,
            topic3,
            data,
        };
        Ok((log, used))
    }
}
