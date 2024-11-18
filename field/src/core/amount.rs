const U128WIDTH: usize = u128::BITS as usize / 8;
const U64WIDTH:  usize =  u64::BITS as usize / 8;

#[allow(dead_code)] const UNIT_MEI:  u8 = 248;
#[allow(dead_code)] const UNIT_ZHU:  u8 = 240;
#[allow(dead_code)] const UNIT_SHUO: u8 = 232;
#[allow(dead_code)] const UNIT_AI:   u8 = 224;
#[allow(dead_code)] const UNIT_MIAO: u8 = 216;


const FROM_CHARS: &[u8; 14] = b"0123456789-.: "; 




pub enum AmtMode {
    U64,
    U128,
}



#[derive(Default, Hash, Clone, PartialEq, Eq)]
pub struct Amount {
	unit: u8,
	dist: i8,
	byte: Vec<u8>,
}


impl Display for Amount{
    fn fmt(&self,f: &mut Formatter) -> Result{
        write!(f,"{}", self.to_string())
    }
}

impl Debug for Amount {
    fn fmt(&self,f: &mut Formatter) -> Result {
        write!(f,"[unit:{}, dist:{}, byte: {:?}]", self.unit, self.dist, self.byte)
    }
}


impl Ord for Amount {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.equal(other) {
            return Ordering::Equal
        }
        if self.more_than(other) {
            return Ordering::Greater
        }
        return Ordering::Less
    }
}

impl PartialOrd for Amount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}


impl Parse for Amount {
    fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
        let mut seek = 0;
        // unit
        let btv = bufeat(&buf[seek..], 1)?;
        self.unit = btv[0];
        seek += 1;
        // dist
        let btv = bufeat(&buf[seek..], 1)?;
        self.dist = btv[0] as i8;
        seek += 1;
        // bytes
        let btlen = self.dist.abs() as usize;
        let btv = bufeat(&buf[seek..], btlen)?;
        self.byte = btv;
        Ok(seek + btlen)
    }
}

impl Serialize for Amount {
    fn serialize(&self) -> Vec<u8> {
        vec![
            vec![self.unit, self.dist as u8],
            self.byte.clone()
        ].concat()
    }
    fn size(&self) -> usize {
        1 + 1 + self.dist.abs() as usize
    }
}


impl_field_only_new!{Amount}


impl Amount {

    pub fn unit(&self) -> u8 {
        self.unit
    }

    pub fn dist(&self) -> i8 {
        self.dist
    }

    pub fn byte(&self) -> &Vec<u8> {
        &self.byte
    }

    pub fn tail_len(&self) -> usize {
        self.dist.abs() as usize
    }

    pub fn tail_u128(&self) -> u128 {
        if self.byte.len() > U128WIDTH {
            panic!("amount tail bytes length too long over {}", U128WIDTH)
        }
        u128::from_be_bytes(add_left_padding(&self.byte, U128WIDTH).try_into().unwrap())
    }

    pub fn tail_u64(&self) -> u64 {
        if self.byte.len() > U64WIDTH {
            panic!("amount tail bytes length too long over {}", U64WIDTH)
        }
        u64::from_be_bytes(add_left_padding(&self.byte, U64WIDTH).try_into().unwrap())
    }


    pub fn is_zero(&self) -> bool {
        self.unit == 0 || self.dist == 0
    }

    pub fn not_zero(&self) -> bool {
        self.unit > 0 && self.dist != 0
    }

    // check must be positive and cannot be zero
    pub fn is_positive(&self) -> bool {
        self.unit > 0 && self.dist > 0
    }   

    // check must be negative and cannot be zero
    pub fn is_negative(&self) -> bool {
        self.unit > 0 && self.dist < 0
    }
    
}


macro_rules! ret_amtfmte {
    ($tip: expr, $v: expr) => {
        return Err(format!("amount {} from '{}' format error or overflow", $tip, $v))
    };
}

// from
impl Amount {


    pub fn mei(v: u64) -> Amount {
        Self::coin(v as u128, UNIT_MEI)
    }
    pub fn zhu(v: u64) -> Amount {
        Self::coin(v as u128, UNIT_ZHU)
    }
    pub fn shuo(v: u64) -> Amount {
        Self::coin(v as u128, UNIT_SHUO)
    }
    pub fn ai(v: u64) -> Amount {
        Self::coin(v as u128, UNIT_AI)
    }
    pub fn miao(v: u64) -> Amount {
        Self::coin(v as u128, UNIT_MIAO)
    }

    pub fn coin(mut v: u128, mut u: u8) -> Amount {
        while v % 10 == 0 {
            if u == 255 {
                break;
            }
            v /= 10;
            u += 1;
        }
        let bts = drop_left_zero(&v.to_be_bytes());
        Self{
            unit: u,
            dist: bts.len() as i8,
            byte: bts
        }
    }


    pub fn from(v: &str) -> Ret<Amount> {
        let mut v = v.replace(",", "").replace(" ", "");
        for a in v.chars() {
            if ! FROM_CHARS.contains(&(a as u8)) {
                ret_amtfmte!{"unsupported characters", String::from(a)}
            }
        }
        let negmark: i8 = match v.starts_with("-") {
            false => 1,
            true => {
                v = v.trim_start_matches('-').to_string();
                -1
            }
        };
        match v.contains(":") {
            true  => Self::from_fin(v, negmark),
            false => Self::from_mei(v, negmark),
        } 
    }

    fn from_fin(v: String, negmark: i8) -> Ret<Amount> {
        let amt: Vec<&str> = v.split(":").collect();
        let Ok(v) = amt[0].parse::<u128>() else {
            ret_amtfmte!{"value", amt[0]}
        };
        let Ok(u) = amt[1].parse::<u8>() else {
            ret_amtfmte!{"unit", amt[1]}
        };
        let mut amt = Self::coin(v, u);
        amt.dist *= negmark; // if neg
        Ok(amt)
    }
    
    fn from_mei(v: String, negmark: i8) -> Ret<Amount> {
        let mut u: u8 = UNIT_MEI;
        let Ok(mut f) = v.parse::<f64>() else {
            ret_amtfmte!{"value", v}
        };
        while f.fract() > 0.0 {
            if u == 0 {
                ret_amtfmte!{"value", v}
            }
            u -= 1;
            f *= 10.0;
        }
        let v = f as u128;
        let mut amt = Self::coin(v, u);
        amt.dist *= negmark; // if neg
        Ok(amt)
    }


}

// to string
impl Amount {


    pub fn to_string(&self) -> String {
        let a = self.to_fin_string();
        "ㄜ".to_owned() + a.as_str()
    }

    pub fn to_fin_string(&self) -> String {
        let (a, b, c) = self.to_strings();
        format!("{}{}:{}", a, b, c)
    }

    pub fn to_strings(&self) -> (String, String, String) {
        let blen =self.byte.len();
        if blen > U128WIDTH {
            return ("*".into(), "*".into(), "*".into())
        }
        let s1 = match self.dist < 0 {
            true => "-",
            false => "",
        }.to_string();
        let s2 = match blen > U64WIDTH {
            true => u128::from_be_bytes(add_left_padding(&self.byte, U128WIDTH).try_into().unwrap()).to_string(),
            false => u64::from_be_bytes(add_left_padding(&self.byte,  U64WIDTH).try_into().unwrap()).to_string(),
        };
        (s1, s2, self.unit.to_string())
    }

    pub fn to_unit_string(&self, unit_str: &str) -> String {
        let unit;
        if let Ok(u) = unit_str.parse::<u8>() {
            unit = u;
        }else{
            unit = match unit_str {
                "mei"  => UNIT_MEI,
                "zhu"  => UNIT_ZHU,
                "shuo" => UNIT_SHUO,
                "ai"   => UNIT_AI,
                "miao" => UNIT_MIAO,
                _ => 0,
            }
        }
        if unit > 0 {
            self.to_unit_unsafe(unit).to_string()
        }else{
            self.to_fin_string()
        }
    }

}

impl Amount {

    pub fn to_unit_unsafe(&self, base_unit: u8) -> f64 {
        if self.is_zero() {
            return 0f64
        }
        if self.tail_len() > U128WIDTH {
            return f64::NAN
        }
        // 
        let chax = (base_unit as i64 - (self.unit as i64)).abs() as u64;
        let tv = self.tail_u128() as f64;
        // unit
        let base = 10f64.powf(chax as f64) as f64;
        let mut resv = match self.unit > base_unit {
            true => tv * base,
            false => tv / base,
        };
        // sign
        if self.dist < 0 {
            resv = resv * -1f64;
        }
        resv
    }

}


// compare 
impl Amount {

    pub fn equal(&self, src: &Amount) -> bool {
        self.unit == src.unit &&
        self.dist == src.dist &&
        self.byte == src.byte
    }

    pub fn more_than(&self, src: &Amount) -> bool {
        if self.dist < 0 || src.dist < 0 {
            panic!("cannot compare between with negative")
        }
        if self.equal(src) {
            return false // a == b
        }
        let us1 = self.unit as usize;
        let us2 =  src.unit as usize;
        let mut tns1 = self.tail_u128().to_string();
        let mut tns2 =  src.tail_u128().to_string();
        let ts1 = tns1.len();
        let ts2 = tns2.len();
        let rlunit1 = us1 + ts1;
        let rlunit2 = us2 + ts2;
        if rlunit1 > rlunit2 {
            return true
        } else if rlunit1 < rlunit2 {
            return false
        }
        // byte width match
        if us1 > us2 {
            tns1 += &"0".repeat(us2-us1);
        } else if us1 < us2 {
            tns2 += &"0".repeat(us1-us2);
        }
        // 
        let Ok(ru1) = tns1.parse::<u128>() else {
            panic!("amount bytes value too big")
        };
        let Ok(ru2) = tns2.parse::<u128>() else {
            panic!("amount bytes value too big")
        };
        ru1 > ru2
    }


}

// compute 
impl Amount {

    pub fn add(&self, amt: &Amount, mode: AmtMode) -> Ret<Amount> {
        match mode {
            AmtMode::U64 => self.add_mode_u64(amt),
            AmtMode::U128 => self.add_mode_u128(amt),
        }
    }

    pub fn sub(&self, amt: &Amount, mode: AmtMode) -> Ret<Amount> {
        match mode {
            AmtMode::U64 => self.sub_mode_u64(amt),
            AmtMode::U128 => self.sub_mode_u128(amt),
        }
    }

    pub fn compress(&self, btn: usize, upvalue: bool) -> Ret<Amount> {
        if self.dist < 0 {
            return errf!("cannot compress negative amount")
        }
        let mut amt = self.clone();
        while amt.tail_len() > btn {
            if amt.byte.len() > U128WIDTH {
                return errf!("amount bytes too long to compress")
            }
            if amt.unit == 255 {
                return errf!("amount uint too big to compress")
            }
            let mut numpls = u128::from_be_bytes(add_left_padding(&amt.byte, U128WIDTH).try_into().unwrap()) / 10;
            if upvalue {
                numpls += 1;
            }
            let nbts = drop_left_zero(&numpls.to_be_bytes());
            // update
            amt.unit += 1;
            amt.dist = nbts.len() as i8;
            amt.byte = nbts;
        }
        // ok
        Ok(amt)
    }


}


/************* compute *************/


macro_rules! rte_ovfl {
    () => {
        return Err("amount computing size overflow".to_string());
    };
}
macro_rules! rte_cneg {
    ($tip: expr) => {
        return Err(format!("amount {} cannot between negative", $tip));
    };
}

fn add_left_padding(v: &Vec<u8>, n: usize) -> Vec<u8> {
    vec![
        vec![0u8; n-v.len()],
        v.clone(),
    ].concat()
}

fn drop_left_zero(v: &[u8]) -> Vec<u8> {
    let mut res = &v[..];
    while res.len() > 0 && res[0] == 0 {
        res = &res[1..];
    }
    res.to_vec()
}


macro_rules! compute_mode_define {
    ($fun:ident, $op: ident, $ty:ty, $ts:expr) => {

        pub fn $fun(&self, src: &Amount) -> Ret<Amount> {
            let dst: &Amount = self;
            if dst.dist < 0 || src.dist < 0 {
                rte_cneg!{stringify!($op)}
            }
            let dtl = dst.tail_len();
            let stl = src.tail_len();
            if dtl > $ts || stl > $ts {
                rte_ovfl!{}
            }
            let mut du = <$ty>::from_be_bytes(add_left_padding(&dst.byte, $ts).try_into().unwrap());
            let mut su = <$ty>::from_be_bytes(add_left_padding(&src.byte, $ts).try_into().unwrap());
            let utsk = (dst.unit as i32 - src.unit as i32).abs() as u32;
            let mut baseut;
            if dst.unit > src.unit {
                let Some(ndu) = du.checked_mul( 10u64.pow(utsk) as $ty ) else {
                    rte_ovfl!{}
                };
                du = ndu;
                baseut = src.unit;
            }else if dst.unit < src.unit {
                let Some(nsu) = su.checked_mul( 10u64.pow(utsk) as $ty ) else {
                    rte_ovfl!{}
                };
                su = nsu;
                baseut = dst.unit;
            }else{
                baseut = dst.unit;
            }
            // do add
            let Some(mut resv) = du.$op( su ) else {
                rte_ovfl!{}
            };
            // drop tail zero
            while resv % 10 == 0 {
                if baseut == 255 {
                    rte_ovfl!{}
                }
                resv /= 10;
                baseut += 1;
            };
            // ok
            let bts = drop_left_zero(&resv.to_be_bytes());
            Ok(Amount{
                unit: baseut,
                dist: bts.len() as i8,
                byte: bts,
            })

        }
    }
}

impl Amount {

compute_mode_define!{add_mode_u64,  checked_add, u64,   U64WIDTH}
compute_mode_define!{add_mode_u128, checked_add, u128, U128WIDTH}
compute_mode_define!{sub_mode_u64,  checked_sub, u64,   U64WIDTH}
compute_mode_define!{sub_mode_u128, checked_sub, u128, U128WIDTH}

}



/************************ test ************************/







#[cfg(test)]
mod amount_tests {
    use super::*;

    #[test]
    fn test1() {

        let a1 = Amount::mei(9527);
        let a2 = Amount::coin(9527, 248);
        let a3 = Amount::from("133188:246").unwrap();
        let a4 = Amount::from("1000.88   ").unwrap();
        let a3 = a3.sub(&Amount::mei(331), AmtMode::U64).unwrap();
        assert_eq!(a1.to_fin_string(), a2.to_fin_string());
        assert_eq!(a3.to_fin_string(), a4.to_fin_string());

    }

}
