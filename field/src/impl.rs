
use interface::*;


impl Parse for Vec<u8> {
    fn parse(&mut self, s: &[u8]) -> Ret<usize> {
        let sl = self.len();
        if sl > s.len() {
            return errf!("buffer too short")
        } 
        Ok(sl)
    }
}


impl Serialize for Vec<u8> {
    fn serialize(&self) -> Vec<u8> {
        self.clone()
    }
    fn size(&self) -> usize { 
        self.len()
    }
}


impl Field for Vec<u8> {
    fn new() -> Self {
        Vec::new()
    }
}




impl Hex for Vec<u8> {
    fn to_hex(&self) -> String {
        hex::encode(self)
    }
}

impl Base64 for Vec<u8> {
    fn to_base64(&self) -> String {
        BASE64_STANDARD.encode(self)
    }
}
