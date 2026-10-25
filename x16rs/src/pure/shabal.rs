//! Port of sphlib `shabal` — Shabal-512 over exactly one 32-byte message (the
//! only shape the X16RS chain feeds it). `shabal_core` with len=32 only
//! buffers (64-byte block, ptr 0 -> 32), so the whole compression happens in
//! `shabal_close`: the padded block (0x80 at byte 32, then zeros) runs
//! INPUT_BLOCK_ADD / XOR_W / APPLY_P, followed by three SWAP_BC / XOR_W /
//! APPLY_P rounds, and the digest is B0..BF little-endian (the `size_words ==
//! 16` branch of the output switch). Verified against vectors generated from
//! the C reference (`tests/pure_vectors.rs`).

/// sphlib `A_init_512`.
const A_INIT: [u32; 12] = [
    0x20728DFD, 0x46C0BD53, 0xE782B699, 0x55304632, 0x71B4EF90, 0x0EA9E82C, 0xDBB930F1, 0xFAD06B8B,
    0xBE0CAE40, 0x8BD14410, 0x76D2ADAC, 0x28ACAB7F,
];

/// sphlib `B_init_512`.
const B_INIT: [u32; 16] = [
    0xC1099CB7, 0x07B385F3, 0xE7442C26, 0xCC8AD640, 0xEB6F56C7, 0x1EA81AA9, 0x73B9D314, 0x1DE85D08,
    0x48910A5A, 0x893B22DB, 0xC5A0DF44, 0xBBC4324E, 0x72D2F240, 0x75941D99, 0x6D8BDE82, 0xA1A7502B,
];

/// sphlib `C_init_512`.
const C_INIT: [u32; 16] = [
    0xD9BF68D1, 0x58BAD750, 0x56028CB2, 0x8134F359, 0xB5D469D8, 0x941A8CC2, 0x418B2A6E, 0x04052780,
    0x7F07D787, 0x5194358F, 0x3C60D665, 0xBE97D79A, 0x950C3434, 0xAED9A06D, 0x2537DC8D, 0x7CDB5969,
];

/// sphlib `APPLY_P`, fully unrolled with constant indices (the C expands
/// `PERM_STEP_0/1/2` x `PERM_ELT` as macros, so every index is a literal).
#[inline(always)]
fn apply_p(a: &mut [u32; 12], b: &mut [u32; 16], c: &[u32; 16], m: &[u32; 16]) {
    for x in b.iter_mut() {
        *x = x.rotate_left(17);
    }
    // PERM_STEP_0
    { // PERM_ELT k=0
        let v = (a[0] ^ a[11].rotate_left(15).wrapping_mul(5) ^ c[8])
            .wrapping_mul(3)
            ^ b[13] ^ (b[9] & !b[6]) ^ m[0];
        a[0] = v;
        b[0] = !(b[0].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=1
        let v = (a[1] ^ a[0].rotate_left(15).wrapping_mul(5) ^ c[7])
            .wrapping_mul(3)
            ^ b[14] ^ (b[10] & !b[7]) ^ m[1];
        a[1] = v;
        b[1] = !(b[1].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=2
        let v = (a[2] ^ a[1].rotate_left(15).wrapping_mul(5) ^ c[6])
            .wrapping_mul(3)
            ^ b[15] ^ (b[11] & !b[8]) ^ m[2];
        a[2] = v;
        b[2] = !(b[2].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=3
        let v = (a[3] ^ a[2].rotate_left(15).wrapping_mul(5) ^ c[5])
            .wrapping_mul(3)
            ^ b[0] ^ (b[12] & !b[9]) ^ m[3];
        a[3] = v;
        b[3] = !(b[3].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=4
        let v = (a[4] ^ a[3].rotate_left(15).wrapping_mul(5) ^ c[4])
            .wrapping_mul(3)
            ^ b[1] ^ (b[13] & !b[10]) ^ m[4];
        a[4] = v;
        b[4] = !(b[4].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=5
        let v = (a[5] ^ a[4].rotate_left(15).wrapping_mul(5) ^ c[3])
            .wrapping_mul(3)
            ^ b[2] ^ (b[14] & !b[11]) ^ m[5];
        a[5] = v;
        b[5] = !(b[5].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=6
        let v = (a[6] ^ a[5].rotate_left(15).wrapping_mul(5) ^ c[2])
            .wrapping_mul(3)
            ^ b[3] ^ (b[15] & !b[12]) ^ m[6];
        a[6] = v;
        b[6] = !(b[6].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=7
        let v = (a[7] ^ a[6].rotate_left(15).wrapping_mul(5) ^ c[1])
            .wrapping_mul(3)
            ^ b[4] ^ (b[0] & !b[13]) ^ m[7];
        a[7] = v;
        b[7] = !(b[7].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=8
        let v = (a[8] ^ a[7].rotate_left(15).wrapping_mul(5) ^ c[0])
            .wrapping_mul(3)
            ^ b[5] ^ (b[1] & !b[14]) ^ m[8];
        a[8] = v;
        b[8] = !(b[8].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=9
        let v = (a[9] ^ a[8].rotate_left(15).wrapping_mul(5) ^ c[15])
            .wrapping_mul(3)
            ^ b[6] ^ (b[2] & !b[15]) ^ m[9];
        a[9] = v;
        b[9] = !(b[9].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=10
        let v = (a[10] ^ a[9].rotate_left(15).wrapping_mul(5) ^ c[14])
            .wrapping_mul(3)
            ^ b[7] ^ (b[3] & !b[0]) ^ m[10];
        a[10] = v;
        b[10] = !(b[10].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=11
        let v = (a[11] ^ a[10].rotate_left(15).wrapping_mul(5) ^ c[13])
            .wrapping_mul(3)
            ^ b[8] ^ (b[4] & !b[1]) ^ m[11];
        a[11] = v;
        b[11] = !(b[11].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=12
        let v = (a[0] ^ a[11].rotate_left(15).wrapping_mul(5) ^ c[12])
            .wrapping_mul(3)
            ^ b[9] ^ (b[5] & !b[2]) ^ m[12];
        a[0] = v;
        b[12] = !(b[12].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=13
        let v = (a[1] ^ a[0].rotate_left(15).wrapping_mul(5) ^ c[11])
            .wrapping_mul(3)
            ^ b[10] ^ (b[6] & !b[3]) ^ m[13];
        a[1] = v;
        b[13] = !(b[13].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=14
        let v = (a[2] ^ a[1].rotate_left(15).wrapping_mul(5) ^ c[10])
            .wrapping_mul(3)
            ^ b[11] ^ (b[7] & !b[4]) ^ m[14];
        a[2] = v;
        b[14] = !(b[14].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=15
        let v = (a[3] ^ a[2].rotate_left(15).wrapping_mul(5) ^ c[9])
            .wrapping_mul(3)
            ^ b[12] ^ (b[8] & !b[5]) ^ m[15];
        a[3] = v;
        b[15] = !(b[15].rotate_left(1) ^ v);
    }
    // PERM_STEP_1
    { // PERM_ELT k=0
        let v = (a[4] ^ a[3].rotate_left(15).wrapping_mul(5) ^ c[8])
            .wrapping_mul(3)
            ^ b[13] ^ (b[9] & !b[6]) ^ m[0];
        a[4] = v;
        b[0] = !(b[0].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=1
        let v = (a[5] ^ a[4].rotate_left(15).wrapping_mul(5) ^ c[7])
            .wrapping_mul(3)
            ^ b[14] ^ (b[10] & !b[7]) ^ m[1];
        a[5] = v;
        b[1] = !(b[1].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=2
        let v = (a[6] ^ a[5].rotate_left(15).wrapping_mul(5) ^ c[6])
            .wrapping_mul(3)
            ^ b[15] ^ (b[11] & !b[8]) ^ m[2];
        a[6] = v;
        b[2] = !(b[2].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=3
        let v = (a[7] ^ a[6].rotate_left(15).wrapping_mul(5) ^ c[5])
            .wrapping_mul(3)
            ^ b[0] ^ (b[12] & !b[9]) ^ m[3];
        a[7] = v;
        b[3] = !(b[3].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=4
        let v = (a[8] ^ a[7].rotate_left(15).wrapping_mul(5) ^ c[4])
            .wrapping_mul(3)
            ^ b[1] ^ (b[13] & !b[10]) ^ m[4];
        a[8] = v;
        b[4] = !(b[4].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=5
        let v = (a[9] ^ a[8].rotate_left(15).wrapping_mul(5) ^ c[3])
            .wrapping_mul(3)
            ^ b[2] ^ (b[14] & !b[11]) ^ m[5];
        a[9] = v;
        b[5] = !(b[5].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=6
        let v = (a[10] ^ a[9].rotate_left(15).wrapping_mul(5) ^ c[2])
            .wrapping_mul(3)
            ^ b[3] ^ (b[15] & !b[12]) ^ m[6];
        a[10] = v;
        b[6] = !(b[6].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=7
        let v = (a[11] ^ a[10].rotate_left(15).wrapping_mul(5) ^ c[1])
            .wrapping_mul(3)
            ^ b[4] ^ (b[0] & !b[13]) ^ m[7];
        a[11] = v;
        b[7] = !(b[7].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=8
        let v = (a[0] ^ a[11].rotate_left(15).wrapping_mul(5) ^ c[0])
            .wrapping_mul(3)
            ^ b[5] ^ (b[1] & !b[14]) ^ m[8];
        a[0] = v;
        b[8] = !(b[8].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=9
        let v = (a[1] ^ a[0].rotate_left(15).wrapping_mul(5) ^ c[15])
            .wrapping_mul(3)
            ^ b[6] ^ (b[2] & !b[15]) ^ m[9];
        a[1] = v;
        b[9] = !(b[9].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=10
        let v = (a[2] ^ a[1].rotate_left(15).wrapping_mul(5) ^ c[14])
            .wrapping_mul(3)
            ^ b[7] ^ (b[3] & !b[0]) ^ m[10];
        a[2] = v;
        b[10] = !(b[10].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=11
        let v = (a[3] ^ a[2].rotate_left(15).wrapping_mul(5) ^ c[13])
            .wrapping_mul(3)
            ^ b[8] ^ (b[4] & !b[1]) ^ m[11];
        a[3] = v;
        b[11] = !(b[11].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=12
        let v = (a[4] ^ a[3].rotate_left(15).wrapping_mul(5) ^ c[12])
            .wrapping_mul(3)
            ^ b[9] ^ (b[5] & !b[2]) ^ m[12];
        a[4] = v;
        b[12] = !(b[12].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=13
        let v = (a[5] ^ a[4].rotate_left(15).wrapping_mul(5) ^ c[11])
            .wrapping_mul(3)
            ^ b[10] ^ (b[6] & !b[3]) ^ m[13];
        a[5] = v;
        b[13] = !(b[13].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=14
        let v = (a[6] ^ a[5].rotate_left(15).wrapping_mul(5) ^ c[10])
            .wrapping_mul(3)
            ^ b[11] ^ (b[7] & !b[4]) ^ m[14];
        a[6] = v;
        b[14] = !(b[14].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=15
        let v = (a[7] ^ a[6].rotate_left(15).wrapping_mul(5) ^ c[9])
            .wrapping_mul(3)
            ^ b[12] ^ (b[8] & !b[5]) ^ m[15];
        a[7] = v;
        b[15] = !(b[15].rotate_left(1) ^ v);
    }
    // PERM_STEP_2
    { // PERM_ELT k=0
        let v = (a[8] ^ a[7].rotate_left(15).wrapping_mul(5) ^ c[8])
            .wrapping_mul(3)
            ^ b[13] ^ (b[9] & !b[6]) ^ m[0];
        a[8] = v;
        b[0] = !(b[0].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=1
        let v = (a[9] ^ a[8].rotate_left(15).wrapping_mul(5) ^ c[7])
            .wrapping_mul(3)
            ^ b[14] ^ (b[10] & !b[7]) ^ m[1];
        a[9] = v;
        b[1] = !(b[1].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=2
        let v = (a[10] ^ a[9].rotate_left(15).wrapping_mul(5) ^ c[6])
            .wrapping_mul(3)
            ^ b[15] ^ (b[11] & !b[8]) ^ m[2];
        a[10] = v;
        b[2] = !(b[2].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=3
        let v = (a[11] ^ a[10].rotate_left(15).wrapping_mul(5) ^ c[5])
            .wrapping_mul(3)
            ^ b[0] ^ (b[12] & !b[9]) ^ m[3];
        a[11] = v;
        b[3] = !(b[3].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=4
        let v = (a[0] ^ a[11].rotate_left(15).wrapping_mul(5) ^ c[4])
            .wrapping_mul(3)
            ^ b[1] ^ (b[13] & !b[10]) ^ m[4];
        a[0] = v;
        b[4] = !(b[4].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=5
        let v = (a[1] ^ a[0].rotate_left(15).wrapping_mul(5) ^ c[3])
            .wrapping_mul(3)
            ^ b[2] ^ (b[14] & !b[11]) ^ m[5];
        a[1] = v;
        b[5] = !(b[5].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=6
        let v = (a[2] ^ a[1].rotate_left(15).wrapping_mul(5) ^ c[2])
            .wrapping_mul(3)
            ^ b[3] ^ (b[15] & !b[12]) ^ m[6];
        a[2] = v;
        b[6] = !(b[6].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=7
        let v = (a[3] ^ a[2].rotate_left(15).wrapping_mul(5) ^ c[1])
            .wrapping_mul(3)
            ^ b[4] ^ (b[0] & !b[13]) ^ m[7];
        a[3] = v;
        b[7] = !(b[7].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=8
        let v = (a[4] ^ a[3].rotate_left(15).wrapping_mul(5) ^ c[0])
            .wrapping_mul(3)
            ^ b[5] ^ (b[1] & !b[14]) ^ m[8];
        a[4] = v;
        b[8] = !(b[8].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=9
        let v = (a[5] ^ a[4].rotate_left(15).wrapping_mul(5) ^ c[15])
            .wrapping_mul(3)
            ^ b[6] ^ (b[2] & !b[15]) ^ m[9];
        a[5] = v;
        b[9] = !(b[9].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=10
        let v = (a[6] ^ a[5].rotate_left(15).wrapping_mul(5) ^ c[14])
            .wrapping_mul(3)
            ^ b[7] ^ (b[3] & !b[0]) ^ m[10];
        a[6] = v;
        b[10] = !(b[10].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=11
        let v = (a[7] ^ a[6].rotate_left(15).wrapping_mul(5) ^ c[13])
            .wrapping_mul(3)
            ^ b[8] ^ (b[4] & !b[1]) ^ m[11];
        a[7] = v;
        b[11] = !(b[11].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=12
        let v = (a[8] ^ a[7].rotate_left(15).wrapping_mul(5) ^ c[12])
            .wrapping_mul(3)
            ^ b[9] ^ (b[5] & !b[2]) ^ m[12];
        a[8] = v;
        b[12] = !(b[12].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=13
        let v = (a[9] ^ a[8].rotate_left(15).wrapping_mul(5) ^ c[11])
            .wrapping_mul(3)
            ^ b[10] ^ (b[6] & !b[3]) ^ m[13];
        a[9] = v;
        b[13] = !(b[13].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=14
        let v = (a[10] ^ a[9].rotate_left(15).wrapping_mul(5) ^ c[10])
            .wrapping_mul(3)
            ^ b[11] ^ (b[7] & !b[4]) ^ m[14];
        a[10] = v;
        b[14] = !(b[14].rotate_left(1) ^ v);
    }
    { // PERM_ELT k=15
        let v = (a[11] ^ a[10].rotate_left(15).wrapping_mul(5) ^ c[9])
            .wrapping_mul(3)
            ^ b[12] ^ (b[8] & !b[5]) ^ m[15];
        a[11] = v;
        b[15] = !(b[15].rotate_left(1) ^ v);
    }
    // The 36 `A += C` lines, unrolled.
    a[11] = a[11].wrapping_add(c[6]);
    a[10] = a[10].wrapping_add(c[5]);
    a[9] = a[9].wrapping_add(c[4]);
    a[8] = a[8].wrapping_add(c[3]);
    a[7] = a[7].wrapping_add(c[2]);
    a[6] = a[6].wrapping_add(c[1]);
    a[5] = a[5].wrapping_add(c[0]);
    a[4] = a[4].wrapping_add(c[15]);
    a[3] = a[3].wrapping_add(c[14]);
    a[2] = a[2].wrapping_add(c[13]);
    a[1] = a[1].wrapping_add(c[12]);
    a[0] = a[0].wrapping_add(c[11]);
    a[11] = a[11].wrapping_add(c[10]);
    a[10] = a[10].wrapping_add(c[9]);
    a[9] = a[9].wrapping_add(c[8]);
    a[8] = a[8].wrapping_add(c[7]);
    a[7] = a[7].wrapping_add(c[6]);
    a[6] = a[6].wrapping_add(c[5]);
    a[5] = a[5].wrapping_add(c[4]);
    a[4] = a[4].wrapping_add(c[3]);
    a[3] = a[3].wrapping_add(c[2]);
    a[2] = a[2].wrapping_add(c[1]);
    a[1] = a[1].wrapping_add(c[0]);
    a[0] = a[0].wrapping_add(c[15]);
    a[11] = a[11].wrapping_add(c[14]);
    a[10] = a[10].wrapping_add(c[13]);
    a[9] = a[9].wrapping_add(c[12]);
    a[8] = a[8].wrapping_add(c[11]);
    a[7] = a[7].wrapping_add(c[10]);
    a[6] = a[6].wrapping_add(c[9]);
    a[5] = a[5].wrapping_add(c[8]);
    a[4] = a[4].wrapping_add(c[7]);
    a[3] = a[3].wrapping_add(c[6]);
    a[2] = a[2].wrapping_add(c[5]);
    a[1] = a[1].wrapping_add(c[4]);
    a[0] = a[0].wrapping_add(c[3]);
}

/// Full 64-byte Shabal-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    let mut a = A_INIT;
    let mut b = B_INIT;
    let mut c = C_INIT;
    // `shabal_init` sets W = (Wlow = 1, Whigh = 0); neither `shabal_core`
    // (no full block) nor `shabal_close` (no INCR_W) changes it.
    let wlow: u32 = 1;
    let whigh: u32 = 0;

    // The padded 64-byte block as 16 little-endian words: message, then
    // buf[32] = 0x80, then zeros.
    let mut m = [0u32; 16];
    for i in 0..8 {
        m[i] = u32::from_le_bytes(msg[i * 4..i * 4 + 4].try_into().unwrap());
    }
    m[8] = 0x80;

    // INPUT_BLOCK_ADD; XOR_W; APPLY_P.
    for i in 0..16 {
        b[i] = b[i].wrapping_add(m[i]);
    }
    a[0] ^= wlow;
    a[1] ^= whigh;
    apply_p(&mut a, &mut b, &c, &m);

    // 3 x (SWAP_BC; XOR_W; APPLY_P).
    for _ in 0..3 {
        for i in 0..16 {
            core::mem::swap(&mut b[i], &mut c[i]);
        }
        a[0] ^= wlow;
        a[1] ^= whigh;
        apply_p(&mut a, &mut b, &c, &m);
    }

    // `shabal_close` size_words = 16: B0..BF, little-endian, in order.
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&b[i].to_le_bytes());
    }
    out
}
