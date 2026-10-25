//! Port of sphlib `blake` — BLAKE-512 over exactly one 32-byte message (the
//! only shape the X16RS chain feeds it): one padded 128-byte block, 16
//! rounds, big-endian 64-byte digest. Faithful to sphlib's `SPH_64`
//! little-endian path; verified against vectors generated from the C
//! reference (`tests/pure_vectors.rs`).

/// BLAKE-512 initial state (sphlib `BLAKE_IV512`).
const IV: [u64; 8] = [
    0x6A09E667F3BCC908,
    0xBB67AE8584CAA73B,
    0x3C6EF372FE94F82B,
    0xA54FF53A5F1D36F1,
    0x510E527FADE682D1,
    0x9B05688C2B3E6C1F,
    0x1F83D9ABFB41BD6B,
    0x5BE0CD19137E2179,
];

/// sphlib `CB0..CBF` (BLAKE-512 round constants).
const CB: [u64; 16] = [
    0x243F6A8885A308D3,
    0x13198A2E03707344,
    0xA4093822299F31D0,
    0x082EFA98EC4E6C89,
    0x452821E638D01377,
    0xBE5466CF34E90C6C,
    0xC0AC29B7C97C50DD,
    0x3F84D5B5B5470917,
    0x9216D5D98979FB1B,
    0xD1310BA698DFB5AC,
    0x2FFD72DBD01ADFB7,
    0xB8E1AFED6A267E96,
    0xBA7C9045F12C7F99,
    0x24A19947B3916CF7,
    0x0801F2E2858EFC16,
    0x636920D871574E69,
];

/// sphlib message-word schedule (ten rounds, then repeats).
const SIGMA: [[u8; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

/// `V8..VF` initializers: salt is zero and the 128-bit bit counter is
/// (th=0, tl=256) for a 32-byte message, so `VC..VF = t ^ CB4..CB7`.
const VCB: [u64; 4] = [CB[4] ^ 256, CB[5] ^ 256, CB[6], CB[7]];

macro_rules! gb {
    ($m0:expr, $m1:expr, $c0:expr, $c1:expr, $v:expr, $a:literal, $b:literal, $c:literal, $d:literal) => {{
        $v[$a] = $v[$a].wrapping_add($v[$b]).wrapping_add($m0 ^ $c1);
        $v[$d] = ($v[$d] ^ $v[$a]).rotate_right(32);
        $v[$c] = $v[$c].wrapping_add($v[$d]);
        $v[$b] = ($v[$b] ^ $v[$c]).rotate_right(25);
        $v[$a] = $v[$a].wrapping_add($v[$b]).wrapping_add($m1 ^ $c0);
        $v[$d] = ($v[$d] ^ $v[$a]).rotate_right(16);
        $v[$c] = $v[$c].wrapping_add($v[$d]);
        $v[$b] = ($v[$b] ^ $v[$c]).rotate_right(11);
    }};
}

macro_rules! round_b {
    ($m:ident, $v:ident, $r:literal) => {{
        let s = SIGMA[$r];
        gb!($m[s[0] as usize], $m[s[1] as usize], CB[s[0] as usize], CB[s[1] as usize], $v, 0, 4, 8, 12);
        gb!($m[s[2] as usize], $m[s[3] as usize], CB[s[2] as usize], CB[s[3] as usize], $v, 1, 5, 9, 13);
        gb!($m[s[4] as usize], $m[s[5] as usize], CB[s[4] as usize], CB[s[5] as usize], $v, 2, 6, 10, 14);
        gb!($m[s[6] as usize], $m[s[7] as usize], CB[s[6] as usize], CB[s[7] as usize], $v, 3, 7, 11, 15);
        gb!($m[s[8] as usize], $m[s[9] as usize], CB[s[8] as usize], CB[s[9] as usize], $v, 0, 5, 10, 15);
        gb!($m[s[10] as usize], $m[s[11] as usize], CB[s[10] as usize], CB[s[11] as usize], $v, 1, 6, 11, 12);
        gb!($m[s[12] as usize], $m[s[13] as usize], CB[s[12] as usize], CB[s[13] as usize], $v, 2, 7, 8, 13);
        gb!($m[s[14] as usize], $m[s[15] as usize], CB[s[14] as usize], CB[s[15] as usize], $v, 3, 4, 9, 14);
    }};
}

/// Full 64-byte BLAKE-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    // Padded block exactly as `blake64_close` builds it: message, 0x80,
    // zeros, the "final" bit 1 at byte 111, and the 128-bit bit counter
    // (th=0, tl=256).
    let mut m = [0u64; 16];
    m[0] = u64::from_be_bytes(msg[0..8].try_into().unwrap());
    m[1] = u64::from_be_bytes(msg[8..16].try_into().unwrap());
    m[2] = u64::from_be_bytes(msg[16..24].try_into().unwrap());
    m[3] = u64::from_be_bytes(msg[24..32].try_into().unwrap());
    m[4] = 0x80u64 << 56;
    m[13] = 1;
    m[15] = 256;

    let mut v = [0u64; 16];
    v[..8].copy_from_slice(&IV);
    v[8] = CB[0];
    v[9] = CB[1];
    v[10] = CB[2];
    v[11] = CB[3];
    v[12] = VCB[0];
    v[13] = VCB[1];
    v[14] = VCB[2];
    v[15] = VCB[3];

    round_b!(m, v, 0);
    round_b!(m, v, 1);
    round_b!(m, v, 2);
    round_b!(m, v, 3);
    round_b!(m, v, 4);
    round_b!(m, v, 5);
    round_b!(m, v, 6);
    round_b!(m, v, 7);
    round_b!(m, v, 8);
    round_b!(m, v, 9);
    round_b!(m, v, 0);
    round_b!(m, v, 1);
    round_b!(m, v, 2);
    round_b!(m, v, 3);
    round_b!(m, v, 4);
    round_b!(m, v, 5);

    // h = IV ^ V ^ V8.. (salt is zero in sphlib blake512)
    let mut out = [0u8; 64];
    for i in 0..8 {
        let h = IV[i] ^ v[i] ^ v[i + 8];
        out[i * 8..i * 8 + 8].copy_from_slice(&h.to_be_bytes());
    }
    out
}
