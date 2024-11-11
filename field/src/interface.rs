use sys::*;

pub trait Serialize {
    fn serialize(&self) -> Vec<u8> { unimplemented!() }
    fn size(&self) -> usize { unimplemented!() }
}

pub trait Parse {
    // return: use length
    fn parse(&mut self, _buf: &[u8]) -> Ret<usize> { unimplemented!() }
}

pub trait Hex {
    fn to_hex(&self) -> String { unimplemented!() }
    fn from_hex(_buf: &[u8]) -> Self where Self: Sized { unimplemented!() }
    fn parse_hex(&mut self, _buf: &[u8]) -> Rerr { unimplemented!() }
}

pub trait Base64 {
    fn to_base64(&self) -> String { unimplemented!() }
    fn from_base64(_buf: &[u8]) -> Self where Self: Sized { unimplemented!() }
    fn parse_base64(&mut self, _buf: &[u8]) -> Rerr { unimplemented!() }
}

pub trait Json {
    fn to_json(&self) -> String { unimplemented!() }
    fn from_json(_buf: &[u8]) -> Self where Self: Sized { unimplemented!() }
    fn parse_json(&mut self, _: &[u8]) -> Rerr { unimplemented!() }
}

pub trait Readable {
    fn to_readable(&self) -> String { unimplemented!(); }
    fn to_readable_left(&self) -> String { unimplemented!(); }
    fn to_readable_or_hex(&self) -> String { unimplemented!(); }
    fn from_readable(_: &[u8]) -> Self where Self: Sized { unimplemented!(); }
    fn build_readable(_: &[u8]) -> Ret<Self> where Self: Sized { unimplemented!(); }
    fn create_readable(_: &[u8]) -> Ret<(Self, usize)> where Self: Sized { unimplemented!(); }
}

pub trait Uint {
    fn to_u8(&self) -> u8 { unimplemented!(); }
    fn to_u16(&self) -> u16 { unimplemented!(); }
    fn to_u32(&self) -> u32 { unimplemented!(); }
    fn to_u64(&self) -> u64 { unimplemented!(); }
    fn to_usize(&self) -> usize { unimplemented!(); }
    fn from_u8(_: u8) -> Self where Self: Sized { unimplemented!(); } // panic
    fn from_u16(_: u16) -> Self where Self: Sized { unimplemented!(); } // panic
    fn from_u32(_: u32) -> Self where Self: Sized { unimplemented!(); } // panic
    fn from_u64(_: u64) -> Self where Self: Sized { unimplemented!(); } // panic
    fn from_usize(_: usize) -> Self where Self: Sized { unimplemented!(); } // panic
    fn parse_u8(&mut self, _: u8) -> Rerr { unimplemented!(); } // panic
    fn parse_u16(&mut self, _: u16) -> Rerr { unimplemented!(); } // panic
    fn parse_u32(&mut self, _: u32) -> Rerr { unimplemented!(); } // panic
    fn parse_u64(&mut self, _: u64) -> Rerr { unimplemented!(); } // panic
    fn parse_usize(&mut self, _: usize) -> Rerr { unimplemented!(); } // panic

}

pub trait Float {
    fn to_f32(&self) -> f32 { unimplemented!(); }
    fn to_f64(&self) -> f64 { unimplemented!(); }
    fn from_f32(_: f32) -> Self where Self: Sized { unimplemented!(); } // panic
    fn from_f64(_: f64) -> Self where Self: Sized { unimplemented!(); } // panic
    fn parse_f32(&mut self, _: f32) -> Rerr { unimplemented!(); } // panic
    fn parse_f64(&mut self, _: f64) -> Rerr { unimplemented!(); } // panic
}



/*
* #[derive(Default)]
*/
pub trait Field : Serialize + Parse + Default { 
    // customizable new() func will change something
    fn new() -> Self where Self: Sized { Self::default() }

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





