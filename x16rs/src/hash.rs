
// sha3-256
pub fn sha3(stuff: impl AsRef<[u8]>) -> [u8; H32S] {

    let mut hasher = Sha3_256::new();
    hasher.update(stuff);
    let result = hasher.finalize();
    let result: [u8; H32S] = result[..].try_into().unwrap();
    result

}


// sha2-256
pub fn sha2(stuff: impl AsRef<[u8]>) -> [u8; H32S] {

    let mut hasher = Sha256::new();
    hasher.update(stuff);
    let result = hasher.finalize();
    let result: [u8; H32S] = result[..].try_into().unwrap();
    result

}


pub fn ripemd160(stuff: impl AsRef<[u8]>) -> [u8; 20] {

    let mut hasher = Ripemd160::new();
    hasher.update(stuff);
    // to [u8; 20]
    let result = hasher.finalize();
    let result: [u8; 20] = result[..].try_into().unwrap();
    result
}


pub fn calculate_hash(stuff: impl AsRef<[u8]>) -> [u8; H32S] {
    sha3(stuff)
}

