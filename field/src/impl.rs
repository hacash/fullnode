
use interface::*;


impl Parse for Vec<u8> {}
impl Serialize for Vec<u8> {}
impl Field for Vec<u8> {}

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
