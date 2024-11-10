
pub fn bytes_from_hex(stuff: &[u8], len: usize) -> Ret<Vec<u8>> {
    let rsz = stuff.len();
    let tsz = len * 2;
    if rsz < tsz {
        return Err("FieldHex".to_owned()+" from_hex size error need "+&rsz.to_string()+" but got "+&rsz.to_string())
    }
    let bts = hex::decode(stuff);
    match bts {
        Ok(b) => Ok(b[0..len].to_vec()),
        Err(e) => Err(e.to_string()),
    }
}



//////////////////////////


pub trait ToHex {
    fn hex(&self) -> String;
}


impl ToHex for Vec<u8> {

    fn hex(&self) -> String {
        hex::encode(self)
    }

}