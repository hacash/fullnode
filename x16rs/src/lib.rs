use sha2::Sha256;
use sha3::{Digest, Sha3_256};
use ripemd::Ripemd160;
use x16rs_sys::x16rs_hash;

pub const H32S: usize = 32;

include!{"hash.rs"}
include!{"block.rs"}
include!{"diamond.rs"}


