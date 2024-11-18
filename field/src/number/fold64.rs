
const FOLDU64SX1: u64 = 32; // 2^^5       // 1byte :                    32
const FOLDU64SX2: u64 = FOLDU64SX1 * 256; // 2byte :                  8192
const FOLDU64SX3: u64 = FOLDU64SX2 * 256; // 3byte :               2097152
const FOLDU64SX4: u64 = FOLDU64SX3 * 256; // 4byte :            5_36870912
const FOLDU64SX5: u64 = FOLDU64SX4 * 256; // 5byte :         1374_38953472
const FOLDU64SX6: u64 = FOLDU64SX5 * 256; // 6byte :       351843_72088832
const FOLDU64SX7: u64 = FOLDU64SX6 * 256; // 7byte :     90071992_54740992
const FOLDU64SX8: u64 = FOLDU64SX7 * 256; // 8byte : 230_58430092_13693952

const FOLDU64XLIST: [u64; 8] = [FOLDU64SX1, FOLDU64SX2, FOLDU64SX3, FOLDU64SX4, FOLDU64SX5, FOLDU64SX6, FOLDU64SX7, FOLDU64SX8];





#[derive(Default, Debug, Hash, Copy, Clone, PartialEq, Eq)]
pub struct Fold64 {
    value: u64,
}


impl Display for Fold64 {
    fn fmt(&self,f: &mut std::fmt::Formatter) -> std::fmt::Result{
        write!(f,"{}", self.value)
    }
}

impl Deref for Fold64 {
    type Target = u64;
    fn deref(&self) -> &u64 {
        &self.value
    }
}


ord_impl!{Fold64, value}
compute_impl!{Fold64, value, u64}
from_uint_all!{Fold64, value, u64}


impl Parse for Fold64 {

    fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
        let bt = bufeatone(buf)?;
        let tl = bt >> 5;
        if tl == 8 { // error
            return Err(s!("Fold64 format error"))
        }
        let mut body = vec![bt & 0b00011111];
        let n = tl as usize;
        if n > 0 {
            let tail = bufeat(&buf[1..], n)?;
            body = [body, tail].concat();
        }
        let bn = body.len();
        if bn < 8 {
            body = [vec![0u8; 8-bn], body].concat();
        }
        self.value = u64::from_be_bytes(body.try_into().unwrap());
        Ok(1 + n)
    }

}


impl Serialize for Fold64 {

    fn serialize(&self) -> Vec<u8> {
        if self.value > Fold64::MAX {
            unimplemented!() // fatal error!!!
        }
        let vs = self.size() as u8;
        let head = vec![(vs - 1) << 5];
        let mut data = self.value.to_be_bytes().to_vec();
        let mv = 8 - vs as usize;
        data = data[mv..].to_vec();
        data[0] ^= head[0];
        data
    }

    fn size(&self) -> usize {
        let v = self.value;
        let mut s = 1;
        for k in FOLDU64XLIST {
            if v < k {
                break; // ok
            }else{
                s += 1;
            }
        }
        s
    }

}


impl_field_only_new!{Fold64}


macro_rules! from_uint_fold64 {
    ($ty:ident) => (
        from_uint_ty!{$ty, u64}
    )
}


macro_rules! parse_uint_fold64 {
    ($ty:ident) => (
        parse_uint_ty!{$ty, u64}
    )
}


impl Uint for Fold64 {

    fn to_u64(&self) -> u64 {
        self.value
    }

    fn as_u64(&self) -> &u64 {
        &self.value
    }

    from_uint_fold64!{u64}
    from_uint_fold64!{u32}
    from_uint_fold64!{u16}
    from_uint_fold64!{u8}
    from_uint_fold64!{usize}

    parse_uint_fold64!{u64}
    parse_uint_fold64!{u32}
    parse_uint_fold64!{u16}
    parse_uint_fold64!{u8}
    parse_uint_fold64!{usize}
}


impl Fold64 {

    pub const MAX: u64 = FOLDU64SX8 - 1;

    pub const fn from(v: u64) -> Self {
        Self{ value: v }
    }

    pub fn uint(&self) -> u64 {
        self.value
    }
    
}





/************************ test ************************/





#[cfg(test)]
mod fold64_tests {
    use super::*;

    /*
    #[test]
    fn test1() {
        for i in 0..=Fold64::MAX {
            do_t_one(i);
        }
    }
    */

    #[test]
    fn test2() {
        do_t_one(0);
        do_t_one(1);
        do_t_one(2);

        do_t_one(                    30);
        do_t_one(                    31);
        do_t_one(                    32);
        do_t_one(                    33);
        do_t_one(                    34);

        do_t_one(                  8190);
        do_t_one(                  8191);
        do_t_one(                  8192);
        do_t_one(                  8193);
        do_t_one(                  8194);
        
        do_t_one(               2097151);
        do_t_one(               2097152);
        do_t_one(               2097153);
        
        do_t_one(            5_36870911);
        do_t_one(            5_36870912);
        do_t_one(            5_36870913);

        do_t_one(         1374_38953471);
        do_t_one(         1374_38953472);
        do_t_one(         1374_38953473);
        
        do_t_one(       351843_72088831);
        do_t_one(       351843_72088832);
        do_t_one(       351843_72088833);
        
        do_t_one(     90071992_54740991);
        do_t_one(     90071992_54740992);
        do_t_one(     90071992_54740993);
        
        do_t_one( 230_58430092_13693951); // MAX

        // do_t_one( 230_58430992_13693952); // overflow error
        // do_t_one( 230_58430992_13693953); // overflow error

    }

    fn do_t_one(n: u64) {
        let fu = Fold64::from(n);
        let mut fu2 = Fold64::from(0);
        let _ = fu2.parse(&fu.serialize());
        assert_eq!(fu, fu2);
    }
    
}













