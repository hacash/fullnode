
use num_bigint::*;
use num_bigint::Sign::*;
use num_traits::*;

const U128S: usize = u128::BITS as usize / 8;
const U64S:  usize =  u64::BITS as usize / 8;

pub const UNIT_MEI:  u8 = 248;
pub const UNIT_ZHU:  u8 = 240;
pub const UNIT_SHUO: u8 = 232;
pub const UNIT_AI:   u8 = 224;
pub const UNIT_MIAO: u8 = 216;


const FROM_CHARS: &[u8; 13] = b"0123456789-.:"; 


pub enum AmtMode {
    U64,
    U128,
    BIGINT,
}



#[derive(Default, Hash, Clone, PartialEq, Eq)]
pub struct Amount {
	unit: u8,
	dist: i8,
	byte: Vec<u8>,
}

impl Display for Amount {
    fn fmt(&self,f: &mut Formatter) -> Result{
        write!(f,"{}", self.to_fin_string())
    }
}

impl Debug for Amount {
    fn fmt(&self,f: &mut Formatter) -> Result {
        write!(f,"[{},{},{:?}]", self.unit, self.dist, self.byte)
    }
}

impl Ord for Amount {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.equal(other) {
            return Ordering::Equal
        }
        if self.greater(other) {
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
        self.unit = bufeatone(&buf)?;
        self.dist = bufeatone(&buf[1..])? as i8;
        let btlen = self.dist.abs() as usize;
        self.byte = bufeat(&buf[2..], btlen)?;
        Ok(2 + btlen)
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

    pub fn tail_u128(&self) -> Ret<u128> {
        if self.byte.len() > U128S {
            return errf!("amount tail bytes length too long over {}", U128S)
        }
        Ok(u128::from_be_bytes(add_left_padding(&self.byte, U128S).try_into().unwrap()))
    }

    pub fn tail_u64(&self) -> Ret<u64> {
        if self.byte.len() > U64S {
            return errf!("amount tail bytes length too long over {}", U64S)
        }
        Ok(u64::from_be_bytes(add_left_padding(&self.byte, U64S).try_into().unwrap()))
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

macro_rules! coin_with {
    ($fn:ident, $ty:ty) => {
        fn $fn(mut v: $ty, mut u: u8) -> Amount {
            if v == 0 || u == 0 {
                return Self::zero()
            }
            while v % 10 == 0 {
                if u == 255 {
                    break // unit max
                }
                v /= 10;
                u += 1;
            }
            let bts = drop_left_zero(&v.to_be_bytes());
            Self {
                unit: u,
                dist: bts.len() as i8,
                byte: bts
            }
        }
    }
}

// from
impl Amount {


    pub fn zero() -> Amount {
        Self::default()
    }
    pub fn small(v: u8, u: u8) -> Amount {
        Self {
            unit: u,
            dist: 1i8,
            byte: vec![v],
        }
    }
    pub fn small_mei(v: u8) -> Amount {
        Self {
            unit: UNIT_MEI,
            dist: 1i8,
            byte: vec![v],
        }
    }
    pub fn mei(v: u64) -> Amount {
        Self::coin(v, UNIT_MEI)
    }
    pub fn zhu(v: u64) -> Amount {
        Self::coin(v, UNIT_ZHU)
    }
    pub fn shuo(v: u64) -> Amount {
        Self::coin(v, UNIT_SHUO)
    }
    pub fn ai(v: u64) -> Amount {
        Self::coin(v, UNIT_AI)
    }
    pub fn miao(v: u64) -> Amount {
        Self::coin(v, UNIT_MIAO)
    }

    coin_with!{coin_u128, u128}
    coin_with!{coin_u64,  u64}

    pub fn coin(v: u64, u: u8) -> Amount {
        Self::coin_u64(v, u)
    }

    pub fn from(v: &str) -> Ret<Amount> {
        let v = v.replace(",", "").replace(" ", "").replace("\n", "");
        for a in v.chars() {
            if ! FROM_CHARS.contains(&(a as u8)) {
                ret_amtfmte!{"unsupported characters", String::from(a)}
            }
        }
        match v.contains(":") {
            true  => Self::from_fin(v),
            false => Self::from_mei(v),
        } 
    }

    fn from_fin(v: String) -> Ret<Amount> {
        let amt: Vec<&str> = v.split(":").collect();
        let Ok(u) = amt[1].parse::<u8>() else {
            ret_amtfmte!{"unit", amt[1]}
        };
        let Ok(v) = amt[0].parse::<i128>() else {
            // from bigint
            let Ok(bign) = BigInt::from_str_radix(&amt[0], 10) else {
                return errf!("amount '{}' overflow BigInt::from_str_radix", &v)
            };
            let mut amt = Self::from_bigint(&bign)?;
            amt.unit = u;
            return Ok(amt)
        };
        // from u128
        let mut amt = Self::coin_u128(v.abs() as u128, u);
        if v < 0 {
            amt.dist *= -1; // if neg
        }
        Ok(amt)
    }
    
    fn from_mei(v: String) -> Ret<Amount> {
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
        if f > u128::MAX as f64 {
            ret_amtfmte!{"value", v}
        }

        let mut amt = Self::coin_u128(f.abs() as u128, u);
        if f < 0.0 {
            amt.dist *= -1; // if neg
        }
        Ok(amt)
    }

    pub fn from_bigint( bignum: &BigInt ) -> Ret<Amount> {
        let numstr = bignum.to_string();
        if numstr == "0" {
            return Ok(Amount::zero())
        }
        let mut numuse = numstr.as_str().trim_end_matches('0').to_owned();
        let mut unit = numstr.len() - numuse.len();
        if unit > 255 { // unit max is 255 
            numuse += &"0".repeat(unit - 255);
            unit = 255;
        }
        let biguse = BigInt::from_str_radix(&numuse, 10);
        if let Err(e) = biguse {
            return errf!("BigInt::from_str_radix error: {} {} {} {}", numstr, numuse, numuse, e.to_string())
        }
        let biguse = biguse.unwrap();
        let (sign, byte) = biguse.to_bytes_be();
        let dist = byte.len();
        if dist > 127 {
            return Err("Amount is too wide.".to_string())
        }
        let mut dist = dist as i8;
        if sign == Minus {
            dist *= -1; // is negative
        }
        // ok
        Ok( Self {
            byte,
            dist,
            unit: unit as u8
        })
    }


    pub fn from_unit_byte(unit: u8, byte: Vec<u8>) -> Ret<Amount> {
        let bl = byte.len();
        if bl > 127 {
            return Err("amount bytes len overflow 127.".to_string())
        }
        Ok(Amount{
            unit: unit,
            dist: bl as i8,
            byte: byte,
        })
    }


}

// to string
impl Amount {


    pub fn sign(&self) -> String {
        match self.dist < 0 {
            true => "-",
            false => "",
        }.to_string()
    }

    pub fn to_string(&self) -> String {
        let a = self.to_fin_string();
        "ㄜ".to_owned() + a.as_str()
    }

    pub fn to_fin_string(&self) -> String {
        let (a, b, c) = self.to_string_part();
        format!("{}{}:{}", a, b, c)
    }

    pub fn to_string_part(&self) -> (String, String, String) {
        let blen = self.tail_len();
        let s2 = match blen > U128S {
            true => BigInt::from_bytes_be(Plus, &self.byte).to_string(),
            false => match blen > U64S {
                true => u128::from_be_bytes(add_left_padding(&self.byte, U128S).try_into().unwrap()).to_string(),
                false => u64::from_be_bytes(add_left_padding(&self.byte,  U64S).try_into().unwrap()).to_string(),
            }
        };
        (self.sign(), s2, self.unit.to_string())
    }

    pub fn to_bigint(&self) -> BigInt {
        if self.is_zero() {
            return 0u64.into();
        }
        let sig = match self.dist > 0 { // sign
            true => Plus,
            false => Minus,
        };
        let bignum = BigInt::from_bytes_be(sig, &self.byte[..]);
        let base: BigInt = 10u64.into();
        let powv = base.pow(self.unit as u32);
        bignum * powv
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
        match unit > 0 {
            true => self.to_unit_unsafe(unit).to_string(),
            false => self.to_fin_string(),
        }
    }

}




impl Amount {

    pub fn to_zhu_unsafe(&self) -> f64 {
        self.to_unit_unsafe(UNIT_ZHU)
    }

    pub fn to_shuo_unsafe(&self) -> f64 {
        self.to_unit_unsafe(UNIT_SHUO)
    }

    pub fn to_unit_unsafe(&self, base_unit: u8) -> f64 {
        if self.is_zero() {
            return 0f64
        }
        let chax = (base_unit as i64 - (self.unit as i64)).abs();
        let tv = match self.tail_len() <= U128S {
            true => self.tail_u128().unwrap() as f64, // u128
            false => match BigInt::from_bytes_be(Plus, &self.byte[..]).to_f64() {
                Some(v) => v,
                None => return f64::NAN,
            },
        };
        // by f64
        let base = 10f64.powf(chax as f64);
        let resv = match self.unit > base_unit {
            true => tv * base,
            false => tv / base,
        };
        // sign
        return match self.dist > 0 {
            true => resv,
            false => resv * -1f64,
        }

    }

}


macro_rules! greater_with {
    ($fn:ident, $ty:ty) => {
        fn $fn(&self, src: &Amount) -> bool {
            concat_idents!{ tail_u = tail_, $ty {
                let mut tns1 = self.tail_u().unwrap().to_string();
                let mut tns2 =  src.tail_u().unwrap().to_string();
            }}
            let ts1 = tns1.len();
            let ts2 = tns2.len();
            let us1 = self.unit as usize;
            let us2 =  src.unit as usize;
            let rlunit1 = us1 + ts1;
            let rlunit2 = us2 + ts2;
            if rlunit1 > rlunit2 {
                return true
            } else if rlunit1 < rlunit2 {
                return false
            }
            // byte width is same
            if us1 > us2 {
                tns1 += &"0".repeat(us1-us2);
            } else if us1 < us2 {
                tns2 += &"0".repeat(us2-us1);
            }
            let ns = tns1.len(); 
            assert!(ns==tns2.len());
            // !("greater_with: {}   {}", tns1, tns2);
            for i in 0 .. ns {
                let a1 = tns1.as_bytes()[i];
                let a2 = tns2.as_bytes()[i];
                if a1 > a2 {
                    return true // more
                }else if a1 < a2 {
                    return false // less
                }
            }
            return false // all same
        }
        
    };
}


// compare 
impl Amount {

    pub fn equal(&self, src: &Amount) -> bool {
        self.unit == src.unit &&
        self.dist == src.dist &&
        self.byte == src.byte
    }

    greater_with!{greater_mode_u128, u128}
    greater_with!{greater_mode_u64,  u64}

    pub fn greater_mode_bigint(&self, src: &Amount) -> bool {
        let db = self.to_bigint();
        let sb =  src.to_bigint();
        db > sb
    }

    pub fn greater(&self, src: &Amount) -> bool {
        if self.dist < 0 || src.dist < 0 {
            panic!("cannot compare between with negative")
        }
        if self.equal(src) {
            return false // a == b
        }
        let dzro = self.is_zero();
        let szro =  src.is_zero();
        if dzro && szro {
            return false // both(0
        } else if dzro {
            return false // left(0) < right(+)
        } else if szro {
            return true // left(+) > right(0)
        }
        // U128 or U64
        let dtl = self.tail_len();
        let stl =  src.tail_len();
        if dtl <= U64S && stl <= U64S {
            return self.greater_mode_u64(src)
        } else if dtl <= U128S && stl <= U128S {
            return self.greater_mode_u128(src)
        }else {
            return self.greater_mode_bigint(src)
        }
        // UBIG
    }


}

// compute 
impl Amount {

    pub fn unit_sub(&mut self, sub: u8) {
        assert!(sub < self.unit, "unit_sub error: unit must big than {}", sub);
        self.unit -= sub;
    }

    pub fn add_mode_bigint(&self, src: &Amount) -> Ret<Amount> {
        let mut db = self.to_bigint();
        let ds =  src.to_bigint();
        db = db + ds;
        Self::from_bigint(&db)
    }

    pub fn sub_mode_bigint(&self, src: &Amount) -> Ret<Amount> {
        let mut db = self.to_bigint();
        let ds =  src.to_bigint();
        db = db - ds;
        Self::from_bigint(&db)
    }

    pub fn add(&self, amt: &Amount, mode: AmtMode) -> Ret<Amount> {
        match mode {
            AmtMode::U64 => self.add_mode_u64(amt),
            AmtMode::U128 => self.add_mode_u128(amt),
            AmtMode::BIGINT => self.add_mode_bigint(amt),
        }
    }

    pub fn sub(&self, amt: &Amount, mode: AmtMode) -> Ret<Amount> {
        match mode {
            AmtMode::U64 => self.sub_mode_u64(amt),
            AmtMode::U128 => self.sub_mode_u128(amt),
            AmtMode::BIGINT => self.sub_mode_bigint(amt),
        }
    }

    pub fn compress(&self, btn: usize, upvalue: bool) -> Ret<Amount> {
        if self.dist < 0 {
            return errf!("cannot compress negative amount")
        }
        let mut amt = self.clone();
        if amt.tail_len() > U128S {
            return errf!("amount bytes too long to compress")
        }
        while amt.tail_len() > btn {
            if amt.unit == 255 {
                return errf!("amount uint too big to compress")
            }
            let mut numpls = u128::from_be_bytes(add_left_padding(&amt.byte, U128S).try_into().unwrap()) / 10;
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
    ($fun:ident, $op:ident, $ty:ty, $ts:expr, $add_or_sub:expr) => {

        pub fn $fun(&self, src: &Amount) -> Ret<Amount> {
            let dst: &Amount = self;
            if dst.dist < 0 || src.dist < 0 {
                rte_cneg!{stringify!($op)}
            }
            let dzro = dst.is_zero();
            let szro = src.is_zero();
            if dzro && szro {
                return Ok(Self::zero())
            }
            if $add_or_sub {
                // add
                if dzro {
                    return Ok(src.clone())
                }else if szro {
                    return Ok(dst.clone())
                }
            }else{
                // sub
                if dzro {
                    rte_ovfl!{}
                }else if szro {
                    return Ok(dst.clone())
                }
            }
            // both not zero
            let dtl = dst.tail_len();
            let stl = src.tail_len();
            if dtl > $ts || stl > $ts {
                rte_ovfl!{}
            }
            let mut du = <$ty>::from_be_bytes(add_left_padding(&dst.byte, $ts).try_into().unwrap());
            let mut su = <$ty>::from_be_bytes(add_left_padding(&src.byte, $ts).try_into().unwrap());
            let utsk = (dst.unit as i32 - src.unit as i32).abs() as u32;
            let baseut;
            if dst.unit > src.unit {
                let Some(ndu) = du.checked_mul( (10 as $ty).pow(utsk) as $ty ) else {
                    // return self.add_mode_bigint(src) // to mode bigint
                    rte_ovfl!{}
                };
                du = ndu;
                baseut = src.unit;
            }else if dst.unit < src.unit {
                let Some(nsu) = su.checked_mul( (10 as $ty).pow(utsk) as $ty ) else {
                    // return self.add_mode_bigint(src) // to mode bigint
                    rte_ovfl!{}
                };
                su = nsu;
                baseut = dst.unit;
            }else{
                baseut = dst.unit;
                if !$add_or_sub && du == su {
                    // sub with same
                    return Ok(Self::zero())
                }
            }
            // do add
            let Some(resv) = du.$op( su ) else {
                // return self.add_mode_bigint(src) // to mode bigint
                rte_ovfl!{}
            };

            concat_idents!{ coin_u = coin_, $ty {
                Ok(Self::coin_u(resv, baseut))
            }} 
        }
    }
}

impl Amount {

compute_mode_define!{add_mode_u64,  checked_add, u64,   U64S, true}
compute_mode_define!{add_mode_u128, checked_add, u128, U128S, true}
compute_mode_define!{sub_mode_u64,  checked_sub, u64,   U64S, false}
compute_mode_define!{sub_mode_u128, checked_sub, u128, U128S, false}

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


    #[test]
    fn test2() {

        let a1 = Amount::mei(1000);
        let a2 = Amount::mei(2000);
        let a3 = Amount::mei(3000);
        let a4 = a1.add_mode_u128(&a2).unwrap();
        let a5 = a1.add_mode_u64(&a2).unwrap();
        let a6 = a3.sub_mode_u128(&a1).unwrap();
        let a7 = a3.sub_mode_u128(&a4).unwrap();
        assert_eq!(a3.to_fin_string(), a4.to_fin_string());
        assert_eq!(a3.to_fin_string(), a5.to_fin_string());
        assert_eq!(a3.to_fin_string(), "3:251");
        assert_eq!(a6.to_fin_string(), a2.to_fin_string());
        assert_eq!(a7.to_fin_string(), "0:0");
        println!("{}  {}  {}  {}  {}  {}  {}", a1, a2, a3, a4, a5, a6, a7);
        println!("{:?}  {:?}  {:?}  {:?}  {:?}  {:?}  {:?}", a1, a2, a3, a4, a5, a6, a7);

    }

    #[test]
    fn test3() {
        let stf = "340282366920938463463374607431768211455";
        let a0 = Amount::from("7:200").unwrap();
        for i in 1 .. stf.len() {
            let f = format!("{}:200", &stf[..i]);
            let a1 = Amount::from(&f).unwrap();
            let a3 = a0.add_mode_u128(&a1).unwrap();
            println!("{}", a3);
        }

    }

    #[test]
    fn test4() {
        let stf = "340282366920938463463374607431768211455";
        let a0 = Amount::from(&format!("{}:200", stf)).unwrap();
        let stl = stf.len();
        for i in 1 .. stl {
            let a1 = Amount::from(&format!("{}:200", &stf[stl-i..])).unwrap();
            let a2 = a0.sub_mode_u128(&a1).unwrap();
            println!("{}", a2);
        }

    }


    #[test]
    fn test5() {
        let stf = "340282366920938463463374607431768211455";
        let a0 = Amount::from(&format!("{}:100", stf)).unwrap();
        for i in 1 .. 16 {
            let a1 = a0.compress(16-i,  true).unwrap();
            let a2 = a0.compress(16-i, false).unwrap();
            println!("{}", a1);
            println!("{}", a2);
        }
    }


    #[test]
    fn test6() {
        let stf = "34028236692093846346337460743176821145579745987243958534961784351938476103756328479843750927435562347109475";
        let _a0 = Amount::from(&format!("{}:100", stf)).unwrap();
        let stl = stf.len();
        for i in 1 .. stl {
            let a1 = Amount::from(&format!("{}:100", &stf[stl-i..])).unwrap();
            println!("{}", a1)
            // let a2 = a0.sub_mode_bigint(&a1).unwrap();
            // println!("{}", a2);
        }

    }

    #[test]
    fn test7() {

        let a1 = Amount::from("11111111111111111:201").unwrap();
        let a2 = Amount::from("111111111111112:202").unwrap();
        // assert!(a2 > a1);
        println!("{} {} {} {} {}", a1<a2, a1==a2, a1>a2, a1>=a2, a1<=a2)

    }


    #[test]
    fn test8() {

        // RUST_BACKTRACE=all cargo test amount_tests::test8 -- --nocapture

        use std::time::Instant;

        let a1 = Amount::mei(1234567890);
        let a2 = Amount::mei(3344520);
        let mx = 100000usize;
        let mut aa = Amount::zero();

        let now = Instant::now();
        for _ in 0..mx {
            aa = a1.add_mode_u64(&a2).unwrap();
        }
        println!("{}  {}", aa, now.elapsed().as_secs_f32());


        let now = Instant::now();
        for _ in 0..mx {
            aa = a1.add_mode_u128(&a2).unwrap();
        }
        println!("{}  {}", aa, now.elapsed().as_secs_f32());


        let now = Instant::now();
        for _ in 0..mx {
            aa = a1.add_mode_bigint(&a2).unwrap();
        }
        println!("{}  {}", aa, now.elapsed().as_secs_f32());


    }


    #[test]
    fn test9() {

        let a1 = Amount::zero();
        let a2 = Amount::from("0:0").unwrap();

        println!("a1 = {:?}, a2 = {:?}, a1 < a2 = {}", a1, a2, a1 < a2);

    }


}
