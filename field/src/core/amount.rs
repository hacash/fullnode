
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


impl Field for Amount {
    fn new() -> Self where Self: Sized {
        Self::default()
    }
}


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
        let bts = drop_left_zero(&v.to_be_bytes());
        Ok(Self{
            unit: u,
            dist: (bts.len() as i8) * negmark,
            byte: bts
        })
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
        let bts = drop_left_zero(&v.to_be_bytes());
        Ok(Self{
            unit: u,
            dist: (bts.len() as i8) * negmark,
            byte: bts
        })
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
        if blen > 16 {
            return ("*".into(), "*".into(), "*".into())
        }
        let s1 = match self.dist < 0 {
            true => "-",
            false => "",
        }.to_string();
        let s2 = match blen > 8 {
            true => u128::from_be_bytes(add_left_padding(&self.byte, 16).try_into().unwrap()).to_string(),
            false => u64::from_be_bytes(add_left_padding(&self.byte,  8).try_into().unwrap()).to_string(),
        };
        (s1, s2, self.unit.to_string())
    }



}


// compute 
impl Amount {

    pub fn add(&self, amt: &Amount, mode: AmtMode) -> Ret<Amount> {
        match mode {
            AmtMode::U64 => add_mode_u64(self, amt),
            AmtMode::U128 => add_mode_u128(self, amt),
        }
    }

    pub fn sub(&self, amt: &Amount, mode: AmtMode) -> Ret<Amount> {
        match mode {
            AmtMode::U64 => sub_mode_u64(self, amt),
            AmtMode::U128 => sub_mode_u128(self, amt),
        }
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

        fn $fun(dst: &Amount, src: &Amount) -> Ret<Amount> {
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

compute_mode_define!{add_mode_u64,  checked_add, u64,   8}
compute_mode_define!{add_mode_u128, checked_add, u128, 16}
compute_mode_define!{sub_mode_u64,  checked_sub, u64,   8}
compute_mode_define!{sub_mode_u128, checked_sub, u128, 16}






/************************ test ************************/







#[cfg(test)]
mod amount_tests {
    use super::*;

    #[test]
    fn test1() {

        let a1 = Amount::mei(9527);
        let a2 = Amount::coin(9527, 248);
        let a3 = Amount::from("133188:246").unwrap();
        let a4 = Amount::from("1328.88   ").unwrap();
        let a3 = a3.sub(&Amount::mei(3), AmtMode::U64).unwrap();
        assert_eq!(a1.to_fin_string(), a2.to_fin_string());
        assert_eq!(a3.to_fin_string(), a4.to_fin_string());



    }

}
