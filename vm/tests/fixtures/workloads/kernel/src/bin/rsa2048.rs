#![no_std]
#![no_main]
use crypto_bigint::modular::runtime_mod::{DynResidue, DynResidueParams};
use crypto_bigint::{Encoding, U2048, U64};

fn kernel(_input: &[u8], out: &mut [u8]) -> usize {
    let modulus = U2048::from_be_slice(&[0xff; 256]);
    let params = DynResidueParams::new(&modulus);
    let base = DynResidue::new(&U2048::from_u64(3), params);
    let exp = U64::from_u64(65537);
    let y = base.pow_bounded_exp(&exp, 17).retrieve();
    let bytes = y.to_be_bytes();
    out[..32].copy_from_slice(&bytes[224..256]);
    32
}

include!("../boot.rs");
