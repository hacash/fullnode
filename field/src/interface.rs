use sys::*;

pub trait Serialize {
    fn serialize(&self) -> Vec<u8> { never!() }
    fn size(&self) -> usize { never!() }
}

pub trait Parse {
    // return: use length
    fn parse(&mut self, _: &[u8]) -> Ret<usize> { never!() }
}


/*
* #[derive(Default)]
*/
pub trait Field : Serialize + Parse { 
    // customizable new() func will change something
    fn new() -> Self where Self: Sized { never!() }

    fn must(buf: &[u8]) -> Self where Self: Sized {
        let mut v = Self::new();
        let res = v.parse(buf);
        match res {
            Ok(_) => v,
            Err(e) => panic!("{}", e),
        }
    }

    fn build(buf: &[u8]) -> Ret<Self> where Self: Sized {
        let mut v = Self::new();
        let res = v.parse(buf);
        res.map(|_|v)
    }
    
    fn create(buf: &[u8]) -> Ret<(Self, usize)> where Self: Sized {
        let mut v = Self::new();
        let res = v.parse(buf);
        res.map(|s|(v,s))
    }

}




pub trait Hex : Field {
    fn to_hex(&self) -> String { never!() }
    fn from_hex(_: &[u8]) -> Ret<Self> where Self: Sized { never!() }
    fn parse_hex(&mut self, _buf: &[u8]) -> Rerr { never!() }
}

pub trait Base64 : Field {
    fn to_base64(&self) -> String { never!() }
    fn from_base64(_: &[u8]) -> Ret<Self> where Self: Sized { never!() }
    fn parse_base64(&mut self, _buf: &[u8]) -> Rerr { never!() }
}

pub trait Json : Field {
    fn to_json(&self) -> String { never!() }
    fn from_json(_: &[u8]) -> Ret<Self> where Self: Sized { never!() }
    fn parse_json(&mut self, _: &[u8]) -> Rerr { never!() }
}

pub trait Readable : Field {
    fn to_readable(&self) -> String { never!(); }
    fn to_readable_left(&self) -> String { never!(); }
    fn to_readable_or_hex(&self) -> String { never!(); }
    fn from_readable(_: &[u8]) -> Ret<Self> where Self: Sized { never!(); }
    fn create_readable(_: &[u8]) -> Ret<(Self, usize)> where Self: Sized { never!(); }
}

/*
pub trait Uintttt : Field {
    fn to_u8(&self) -> u8 { never!(); }
    fn to_u16(&self) -> u16 { never!(); }
    fn to_u32(&self) -> u32 { never!(); }
    fn to_u64(&self) -> u64 { never!(); }
    fn to_usize(&self) -> usize { never!(); }
    fn as_u8(&self) -> &u8 { never!(); }
    fn as_u16(&self) -> &u16 { never!(); }
    fn as_u32(&self) -> &u32 { never!(); }
    fn as_u64(&self) -> &u64 { never!(); }
    fn as_usize(&self) -> &usize { never!(); }
    fn from_u8(_: u8) -> Self where Self: Sized { never!(); } // panic
    fn from_u16(_: u16) -> Self where Self: Sized { never!(); } // panic
    fn from_u32(_: u32) -> Self where Self: Sized { never!(); } // panic
    fn from_u64(_: u64) -> Self where Self: Sized { never!(); } // panic
    fn from_usize(_: usize) -> Self where Self: Sized { never!(); } // panic
    fn parse_u8(&mut self, _: u8) -> Rerr { never!(); } // panic
    fn parse_u16(&mut self, _: u16) -> Rerr { never!(); } // panic
    fn parse_u32(&mut self, _: u32) -> Rerr { never!(); } // panic
    fn parse_u64(&mut self, _: u64) -> Rerr { never!(); } // panic
    fn parse_usize(&mut self, _: usize) -> Rerr { never!(); } // panic
}
*/

pub trait Float : Field {
    fn to_f32(&self) -> f32 { never!(); }
    fn to_f64(&self) -> f64 { never!(); }
    fn from_f32(_: f32) -> Ret<Self> where Self: Sized { never!(); }
    fn from_f64(_: f64) -> Ret<Self> where Self: Sized { never!(); }
    fn parse_f32(&mut self, _: f32) -> Rerr { never!(); }
    fn parse_f64(&mut self, _: f64) -> Rerr { never!(); }
}





