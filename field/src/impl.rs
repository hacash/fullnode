use base64::prelude::*;

use interface::*;

use sys::*;


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


impl Uint for Vec<u8> {

    fn to_u8(&self) -> u8 {
        match self.len() == 1 {
            true => self[0],
            false => panic!("{}", s!("length error")),
        }
    }

    

}
