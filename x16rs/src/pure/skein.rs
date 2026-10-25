//! Port of sphlib `skein` — Skein-512/512 over exactly one 32-byte message
//! (the only shape the X16RS chain feeds it). `skein_big_core` just buffers
//! the 32 bytes, so the whole hash is the two UBI compressions performed by
//! `skein_big_close`: the final message block (msg || zero padding) and the
//! all-zero output block. Faithful to the sphlib `SPH_64`
//! non-`SPH_SMALL_FOOTPRINT` path; verified against vectors generated from
//! the C reference (`tests/pure_vectors.rs`).

/// sphlib `SKEIN_IV512`.
const IV512: [u64; 8] = [
    0x4903ADFF749C51CE,
    0x0D95DE399746DF03,
    0x8FD1934127C79BCE,
    0x9A255629FF352CB1,
    0x5DB62599DF6CA7B0,
    0xEABE394CA9D5C3F4,
    0x991112C71A75B523,
    0xAE18A40B660FCC33,
];

macro_rules! mix {
    ($x0:ident, $x1:ident, $rc:literal) => {{
        $x0 = $x0.wrapping_add($x1);
        $x1 = $x1.rotate_left($rc) ^ $x0;
    }};
}

macro_rules! mix8 {
    ($p0:ident, $p1:ident, $p2:ident, $p3:ident, $p4:ident, $p5:ident, $p6:ident, $p7:ident,
     $rc0:literal, $rc1:literal, $rc2:literal, $rc3:literal) => {{
        mix!($p0, $p1, $rc0);
        mix!($p2, $p3, $rc1);
        mix!($p4, $p5, $rc2);
        mix!($p6, $p7, $rc3);
    }};
}

/// `TFBIG_ADDKEY`: `SKBI(k, s, i)` = `k[(s+i) % 9]`, `SKBT(t, s, v)` =
/// `t[(s+v) % 3]` (the non-small path never rotates the tweak locals; the
/// modular index does it).
macro_rules! addkey {
    ($p0:ident, $p1:ident, $p2:ident, $p3:ident, $p4:ident, $p5:ident, $p6:ident, $p7:ident,
     $k:ident, $t:ident, $s:literal) => {{
        $p0 = $p0.wrapping_add($k[($s + 0) % 9]);
        $p1 = $p1.wrapping_add($k[($s + 1) % 9]);
        $p2 = $p2.wrapping_add($k[($s + 2) % 9]);
        $p3 = $p3.wrapping_add($k[($s + 3) % 9]);
        $p4 = $p4.wrapping_add($k[($s + 4) % 9]);
        $p5 = $p5.wrapping_add($k[($s + 5) % 9]).wrapping_add($t[$s % 3]);
        $p6 = $p6
            .wrapping_add($k[($s + 6) % 9])
            .wrapping_add($t[($s + 1) % 3]);
        $p7 = $p7.wrapping_add($k[($s + 7) % 9]).wrapping_add($s as u64);
    }};
}

macro_rules! tfbig_4e {
    ($p0:ident, $p1:ident, $p2:ident, $p3:ident, $p4:ident, $p5:ident, $p6:ident, $p7:ident,
     $k:ident, $t:ident, $s:literal) => {{
        addkey!($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, $k, $t, $s);
        mix8!($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, 46, 36, 19, 37);
        mix8!($p2, $p1, $p4, $p7, $p6, $p5, $p0, $p3, 33, 27, 14, 42);
        mix8!($p4, $p1, $p6, $p3, $p0, $p5, $p2, $p7, 17, 49, 36, 39);
        mix8!($p6, $p1, $p0, $p7, $p2, $p5, $p4, $p3, 44, 9, 54, 56);
    }};
}

macro_rules! tfbig_4o {
    ($p0:ident, $p1:ident, $p2:ident, $p3:ident, $p4:ident, $p5:ident, $p6:ident, $p7:ident,
     $k:ident, $t:ident, $s:literal) => {{
        addkey!($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, $k, $t, $s);
        mix8!($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, 39, 30, 34, 24);
        mix8!($p2, $p1, $p4, $p7, $p6, $p5, $p0, $p3, 13, 50, 10, 17);
        mix8!($p4, $p1, $p6, $p3, $p0, $p5, $p2, $p7, 25, 29, 39, 43);
        mix8!($p6, $p1, $p0, $p7, $p2, $p5, $p4, $p3, 8, 35, 56, 22);
    }};
}

/// One UBI iteration over a 64-byte block (`UBI_BIG` in skein.c, expanded
/// for the fixed shape). `bcount` is 0 both times: the 32-byte message is a
/// single block whose UBI runs in close(), and `bcount` is reset before the
/// output block.
fn ubi(h: &mut [u64; 8], m: &[u64; 8], etype: u64, extra: u64) {
    let mut p0 = m[0];
    let mut p1 = m[1];
    let mut p2 = m[2];
    let mut p3 = m[3];
    let mut p4 = m[4];
    let mut p5 = m[5];
    let mut p6 = m[6];
    let mut p7 = m[7];
    let bcount: u64 = 0;
    let t0 = bcount.wrapping_shl(6).wrapping_add(extra);
    let t1 = (bcount >> 58).wrapping_add(etype << 55);
    let t2 = t0 ^ t1;
    // TFBIG_KINIT
    let mut k = [0u64; 9];
    k[..8].copy_from_slice(h);
    k[8] = ((k[0] ^ k[1]) ^ (k[2] ^ k[3])) ^ ((k[4] ^ k[5]) ^ (k[6] ^ k[7])) ^ 0x1BD11BDAA9FC1A22;
    let t = [t0, t1, t2];
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 0);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 1);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 2);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 3);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 4);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 5);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 6);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 7);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 8);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 9);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 10);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 11);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 12);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 13);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 14);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 15);
    tfbig_4e!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 16);
    tfbig_4o!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 17);
    addkey!(p0, p1, p2, p3, p4, p5, p6, p7, k, t, 18);
    h[0] = m[0] ^ p0;
    h[1] = m[1] ^ p1;
    h[2] = m[2] ^ p2;
    h[3] = m[3] ^ p3;
    h[4] = m[4] ^ p4;
    h[5] = m[5] ^ p5;
    h[6] = m[6] ^ p6;
    h[7] = m[7] ^ p7;
}

/// Full 64-byte Skein-512/512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    let mut h = IV512;
    // Message block: `skein_big_core` buffered the 32 bytes, `skein_big_close`
    // zero-pads to 64; et = 352 + ((bcount == 0) << 7) + (n != 0) = 480,
    // extra = ptr = 32.
    let mut m = [0u64; 8];
    m[0] = u64::from_le_bytes(msg[0..8].try_into().unwrap());
    m[1] = u64::from_le_bytes(msg[8..16].try_into().unwrap());
    m[2] = u64::from_le_bytes(msg[16..24].try_into().unwrap());
    m[3] = u64::from_le_bytes(msg[24..32].try_into().unwrap());
    ubi(&mut h, &m, 480, 32);
    // Output block: buf all zeros, bcount reset, et = 510, ptr = 8.
    ubi(&mut h, &[0u64; 8], 510, 8);
    let mut out = [0u8; 64];
    for (i, v) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    out
}
