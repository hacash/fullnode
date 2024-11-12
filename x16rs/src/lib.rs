use sha2::Sha256;
use sha3::{Digest, Sha3_256};
use ripemd::Ripemd160;

pub const H32S: usize = 32;

include!{"x16rs.rs"}
include!{"hash.rs"}
include!{"block.rs"}
include!{"diamond.rs"}


