//! Port of sphlib `luffa` — Luffa-512 over exactly one 32-byte message (the
//! only shape the X16RS chain feeds it): five 256-bit pipelines, one full
//! message block, then the three close blocks (`0x80` padding block followed
//! by two all-zero blocks), with the digest gathered after the last two.
//! Faithful to the C configuration selected on x86-64 (`SPH_64_TRUE` implies
//! `SPH_LUFFA_PARALLEL`, i.e. the packed-u64 round function for pipelines
//! 0..3 and a plain 32-bit round function for pipeline 4); verified against
//! vectors generated from the C reference (`tests/pure_vectors.rs`).

/// sphlib `V_INIT` (one row per pipeline).
const V_INIT: [[u32; 8]; 5] = [
    [
        0x6d251e69, 0x44b051e0, 0x4eaa6fb4, 0xdbf78465, 0x6e292011, 0x90152df4, 0xee058139,
        0xdef610bb,
    ],
    [
        0xc3b44b95, 0xd9d2f256, 0x70eee9a0, 0xde099fa3, 0x5d9b0557, 0x8fc944b3, 0xcf1ccf0e,
        0x746cd581,
    ],
    [
        0xf7efc89d, 0x5dba5781, 0x04016ce5, 0xad659c05, 0x0306194f, 0x666d1836, 0x24aa230a,
        0x8b264ae7,
    ],
    [
        0x858075d5, 0x36d79cce, 0xe571f7d7, 0x204b1f67, 0x35870c6a, 0x57e9e923, 0x14bcb808,
        0x7cde72ce,
    ],
    [
        0x6c68e9be, 0x5ec41e22, 0xc825b7c7, 0xaffb4363, 0xf5df3999, 0x0fc688f1, 0xb07224cc,
        0x03e86cea,
    ],
];

/// sphlib `RCW010` (packed `RC00 | RC10 << 32`).
const RCW010: [u64; 8] = [
    0xb6de10ed303994a6,
    0x70f47aaec0e65299,
    0x0707a3d46cc33a12,
    0x1c1e8f51dc56983e,
    0x707a3d451e00108f,
    0xaeb285627800423d,
    0xbaca15898f5b7882,
    0x40a46f3e96e1db12,
];

/// sphlib `RCW014` (packed `RC04 | RC14 << 32`).
const RCW014: [u64; 8] = [
    0x01685f3de0337818,
    0x05a17cf4441ba90d,
    0xbd09caca7f34d442,
    0xf4272b289389217f,
    0x144ae5cce5a8bce6,
    0xfaa7ae2b5274baf4,
    0x2e48f1c126889ba7,
    0xb923c7049a226e9d,
];

/// sphlib `RCW230` (packed `RC20 | RC30 << 32`).
const RCW230: [u64; 8] = [
    0xb213afa5fc20d9d2,
    0xc84ebe9534552e25,
    0x4e608a227ad8818f,
    0x56d858fe8438764a,
    0x343b138fbb6de032,
    0xd0ec4e3dedb780c8,
    0x2ceb4882d9847356,
    0xb3ad2208a2c78434,
];

/// sphlib `RCW234` (packed `RC24 | RC34 << 32`).
const RCW234: [u64; 8] = [
    0xe028c9bfe25e72c1,
    0x44756f91e623bb72,
    0x7e8fce325c58a4a4,
    0x956548be1e38e2e7,
    0xfe191be278e38b9d,
    0x3cb226e527586719,
    0x5944a28e36eda57f,
    0xa1c4c355703aace7,
];

/// sphlib `RC40`.
const RC40: [u32; 8] = [
    0xf0d2e9e3, 0xac11d7fa, 0x1bcb66f2, 0x6f2d9bc9, 0x78602649, 0x8edae952, 0x3b6ba548, 0xedae9520,
];

/// sphlib `RC44`.
const RC44: [u32; 8] = [
    0x5090d577, 0x2d1925ab, 0xb46496ac, 0xd1925ab0, 0x29131ab6, 0x0fc053c3, 0x3f014f0c, 0xfc053c31,
];

/// sphlib `M2` on eight-word rows: each source word is read before its slot
/// is overwritten, so the in-place sequential order is exact (also when
/// `d` and `s` are the same row, as the C relies on).
macro_rules! m2 {
    ($d:ident, $s:ident) => {{
        let t = $s[7];
        $d[7] = $s[6];
        $d[6] = $s[5];
        $d[5] = $s[4];
        $d[4] = $s[3] ^ t;
        $d[3] = $s[2] ^ t;
        $d[2] = $s[1];
        $d[1] = $s[0] ^ t;
        $d[0] = t;
    }};
}

/// sphlib `XOR` on eight-word rows.
macro_rules! xor8 {
    ($d:ident, $s1:ident, $s2:ident) => {{
        $d[0] = $s1[0] ^ $s2[0];
        $d[1] = $s1[1] ^ $s2[1];
        $d[2] = $s1[2] ^ $s2[2];
        $d[3] = $s1[3] ^ $s2[3];
        $d[4] = $s1[4] ^ $s2[4];
        $d[5] = $s1[5] ^ $s2[5];
        $d[6] = $s1[6] ^ $s2[6];
        $d[7] = $s1[7] ^ $s2[7];
    }};
}

/// sphlib `SUB_CRUMBW` (the 64-bit lane version used by
/// `SPH_LUFFA_PARALLEL`).
macro_rules! sub_crumbw {
    ($a0:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        let mut tmp = $a0;
        $a0 |= $a1;
        $a2 ^= $a3;
        $a1 = !$a1;
        $a0 ^= $a3;
        $a3 &= tmp;
        $a1 ^= $a3;
        $a3 ^= $a2;
        $a2 &= $a0;
        $a0 = !$a0;
        $a2 ^= $a1;
        $a1 |= $a3;
        tmp ^= $a1;
        $a3 ^= $a2;
        $a2 &= $a1;
        $a1 ^= $a0;
        $a0 = tmp;
    }};
}

/// sphlib `MIX_WORDW` (packed-u64 mix: both 32-bit halves mixed exactly as
/// `MIX_WORD` would).
macro_rules! mix_wordw {
    ($u:expr, $v:expr) => {{
        $v ^= $u;
        let mut ul = $u as u32;
        let mut uh = ($u >> 32) as u32;
        let mut vl = $v as u32;
        let mut vh = ($v >> 32) as u32;
        ul = ul.rotate_left(2) ^ vl;
        vl = vl.rotate_left(14) ^ ul;
        ul = ul.rotate_left(10) ^ vl;
        vl = vl.rotate_left(1);
        uh = uh.rotate_left(2) ^ vh;
        vh = vh.rotate_left(14) ^ uh;
        uh = uh.rotate_left(10) ^ vh;
        vh = vh.rotate_left(1);
        $u = (ul as u64) | ((uh as u64) << 32);
        $v = (vl as u64) | ((vh as u64) << 32);
    }};
}

/// sphlib `SUB_CRUMB` (plain 32-bit version, used for pipeline 4).
macro_rules! sub_crumb {
    ($a0:expr, $a1:expr, $a2:expr, $a3:expr) => {{
        let mut tmp = $a0;
        $a0 |= $a1;
        $a2 ^= $a3;
        $a1 = !$a1;
        $a0 ^= $a3;
        $a3 &= tmp;
        $a1 ^= $a3;
        $a3 ^= $a2;
        $a2 &= $a0;
        $a0 = !$a0;
        $a2 ^= $a1;
        $a1 |= $a3;
        tmp ^= $a1;
        $a3 ^= $a2;
        $a2 &= $a1;
        $a1 ^= $a0;
        $a0 = tmp;
    }};
}

/// sphlib `MIX_WORD` (plain 32-bit version).
macro_rules! mix_word {
    ($u:expr, $v:expr) => {{
        $v ^= $u;
        $u = $u.rotate_left(2) ^ $v;
        $v = $v.rotate_left(14) ^ $u;
        $u = $u.rotate_left(10) ^ $v;
        $v = $v.rotate_left(1);
    }};
}

/// sphlib `MI5`: message-block injection. The 32 message bytes are read as
/// eight BIG-endian u32 words (`sph_dec32be_aligned`).
macro_rules! mi5 {
    ($buf:ident, $m:ident, $a:ident, $b:ident,
     $v0:ident, $v1:ident, $v2:ident, $v3:ident, $v4:ident) => {{
        $m[0] = u32::from_be_bytes([$buf[0], $buf[1], $buf[2], $buf[3]]);
        $m[1] = u32::from_be_bytes([$buf[4], $buf[5], $buf[6], $buf[7]]);
        $m[2] = u32::from_be_bytes([$buf[8], $buf[9], $buf[10], $buf[11]]);
        $m[3] = u32::from_be_bytes([$buf[12], $buf[13], $buf[14], $buf[15]]);
        $m[4] = u32::from_be_bytes([$buf[16], $buf[17], $buf[18], $buf[19]]);
        $m[5] = u32::from_be_bytes([$buf[20], $buf[21], $buf[22], $buf[23]]);
        $m[6] = u32::from_be_bytes([$buf[24], $buf[25], $buf[26], $buf[27]]);
        $m[7] = u32::from_be_bytes([$buf[28], $buf[29], $buf[30], $buf[31]]);
        xor8!($a, $v0, $v1);
        xor8!($b, $v2, $v3);
        xor8!($a, $a, $b);
        xor8!($a, $a, $v4);
        m2!($a, $a);
        xor8!($v0, $a, $v0);
        xor8!($v1, $a, $v1);
        xor8!($v2, $a, $v2);
        xor8!($v3, $a, $v3);
        xor8!($v4, $a, $v4);
        m2!($b, $v0);
        xor8!($b, $b, $v1);
        m2!($v1, $v1);
        xor8!($v1, $v1, $v2);
        m2!($v2, $v2);
        xor8!($v2, $v2, $v3);
        m2!($v3, $v3);
        xor8!($v3, $v3, $v4);
        m2!($v4, $v4);
        xor8!($v4, $v4, $v0);
        m2!($v0, $b);
        xor8!($v0, $v0, $v4);
        m2!($v4, $v4);
        xor8!($v4, $v4, $v3);
        m2!($v3, $v3);
        xor8!($v3, $v3, $v2);
        m2!($v2, $v2);
        xor8!($v2, $v2, $v1);
        m2!($v1, $v1);
        xor8!($v1, $v1, $b);
        xor8!($v0, $v0, $m);
        m2!($m, $m);
        xor8!($v1, $v1, $m);
        m2!($m, $m);
        xor8!($v2, $v2, $m);
        m2!($m, $m);
        xor8!($v3, $v3, $m);
        m2!($m, $m);
        xor8!($v4, $v4, $m);
    }};
}

/// sphlib `TWEAK5` + `P5_kuffa` (parallel branch): eight rounds over
/// pipelines 0/1 packed into u64, eight over pipelines 2/3 packed into u64,
/// eight over pipeline 4 in plain 32-bit.
macro_rules! p5 {
    ($v0:ident, $v1:ident, $v2:ident, $v3:ident, $v4:ident) => {{
        // TWEAK5
        $v1[4] = $v1[4].rotate_left(1);
        $v1[5] = $v1[5].rotate_left(1);
        $v1[6] = $v1[6].rotate_left(1);
        $v1[7] = $v1[7].rotate_left(1);
        $v2[4] = $v2[4].rotate_left(2);
        $v2[5] = $v2[5].rotate_left(2);
        $v2[6] = $v2[6].rotate_left(2);
        $v2[7] = $v2[7].rotate_left(2);
        $v3[4] = $v3[4].rotate_left(3);
        $v3[5] = $v3[5].rotate_left(3);
        $v3[6] = $v3[6].rotate_left(3);
        $v3[7] = $v3[7].rotate_left(3);
        $v4[4] = $v4[4].rotate_left(4);
        $v4[5] = $v4[5].rotate_left(4);
        $v4[6] = $v4[6].rotate_left(4);
        $v4[7] = $v4[7].rotate_left(4);
        let mut w = [0u64; 8];
        // Pipelines 0 (low half) and 1 (high half).
        for j in 0..8 {
            w[j] = ($v0[j] as u64) | (($v1[j] as u64) << 32);
        }
        for r in 0..8 {
            sub_crumbw!(w[0], w[1], w[2], w[3]);
            sub_crumbw!(w[5], w[6], w[7], w[4]);
            mix_wordw!(w[0], w[4]);
            mix_wordw!(w[1], w[5]);
            mix_wordw!(w[2], w[6]);
            mix_wordw!(w[3], w[7]);
            w[0] ^= RCW010[r];
            w[4] ^= RCW014[r];
        }
        for j in 0..8 {
            $v0[j] = w[j] as u32;
            $v1[j] = (w[j] >> 32) as u32;
        }
        // Pipelines 2 (low half) and 3 (high half).
        for j in 0..8 {
            w[j] = ($v2[j] as u64) | (($v3[j] as u64) << 32);
        }
        for r in 0..8 {
            sub_crumbw!(w[0], w[1], w[2], w[3]);
            sub_crumbw!(w[5], w[6], w[7], w[4]);
            mix_wordw!(w[0], w[4]);
            mix_wordw!(w[1], w[5]);
            mix_wordw!(w[2], w[6]);
            mix_wordw!(w[3], w[7]);
            w[0] ^= RCW230[r];
            w[4] ^= RCW234[r];
        }
        for j in 0..8 {
            $v2[j] = w[j] as u32;
            $v3[j] = (w[j] >> 32) as u32;
        }
        // Pipeline 4.
        for r in 0..8 {
            sub_crumb!($v4[0], $v4[1], $v4[2], $v4[3]);
            sub_crumb!($v4[5], $v4[6], $v4[7], $v4[4]);
            mix_word!($v4[0], $v4[4]);
            mix_word!($v4[1], $v4[5]);
            mix_word!($v4[2], $v4[6]);
            mix_word!($v4[3], $v4[7]);
            $v4[0] ^= RC40[r];
            $v4[4] ^= RC44[r];
        }
    }};
}

/// Full 64-byte digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    // `sph_luffa512_init`.
    let mut v0 = V_INIT[0];
    let mut v1 = V_INIT[1];
    let mut v2 = V_INIT[2];
    let mut v3 = V_INIT[3];
    let mut v4 = V_INIT[4];
    let mut m = [0u32; 8];
    let mut a = [0u32; 8];
    let mut b = [0u32; 8];
    let mut buf = [0u8; 32];

    // `luffa5`: the 32-byte message fills one whole block -> MI5 + P5 once.
    buf.copy_from_slice(msg);
    mi5!(buf, m, a, b, v0, v1, v2, v3, v4);
    p5!(v0, v1, v2, v3, v4);

    // `luffa5_close` (ub=0, n=0): the padded block `0x80 || 0x00 * 31`,
    // then two all-zero blocks, gathering digest words after each.
    buf = [0u8; 32];
    buf[0] = 0x80;
    mi5!(buf, m, a, b, v0, v1, v2, v3, v4);
    p5!(v0, v1, v2, v3, v4);

    buf = [0u8; 32];
    mi5!(buf, m, a, b, v0, v1, v2, v3, v4);
    p5!(v0, v1, v2, v3, v4);
    let mut out = [0u8; 64];
    out[0..4].copy_from_slice(&(v0[0] ^ v1[0] ^ v2[0] ^ v3[0] ^ v4[0]).to_be_bytes());
    out[4..8].copy_from_slice(&(v0[1] ^ v1[1] ^ v2[1] ^ v3[1] ^ v4[1]).to_be_bytes());
    out[8..12].copy_from_slice(&(v0[2] ^ v1[2] ^ v2[2] ^ v3[2] ^ v4[2]).to_be_bytes());
    out[12..16].copy_from_slice(&(v0[3] ^ v1[3] ^ v2[3] ^ v3[3] ^ v4[3]).to_be_bytes());
    out[16..20].copy_from_slice(&(v0[4] ^ v1[4] ^ v2[4] ^ v3[4] ^ v4[4]).to_be_bytes());
    out[20..24].copy_from_slice(&(v0[5] ^ v1[5] ^ v2[5] ^ v3[5] ^ v4[5]).to_be_bytes());
    out[24..28].copy_from_slice(&(v0[6] ^ v1[6] ^ v2[6] ^ v3[6] ^ v4[6]).to_be_bytes());
    out[28..32].copy_from_slice(&(v0[7] ^ v1[7] ^ v2[7] ^ v3[7] ^ v4[7]).to_be_bytes());

    mi5!(buf, m, a, b, v0, v1, v2, v3, v4);
    p5!(v0, v1, v2, v3, v4);
    out[32..36].copy_from_slice(&(v0[0] ^ v1[0] ^ v2[0] ^ v3[0] ^ v4[0]).to_be_bytes());
    out[36..40].copy_from_slice(&(v0[1] ^ v1[1] ^ v2[1] ^ v3[1] ^ v4[1]).to_be_bytes());
    out[40..44].copy_from_slice(&(v0[2] ^ v1[2] ^ v2[2] ^ v3[2] ^ v4[2]).to_be_bytes());
    out[44..48].copy_from_slice(&(v0[3] ^ v1[3] ^ v2[3] ^ v3[3] ^ v4[3]).to_be_bytes());
    out[48..52].copy_from_slice(&(v0[4] ^ v1[4] ^ v2[4] ^ v3[4] ^ v4[4]).to_be_bytes());
    out[52..56].copy_from_slice(&(v0[5] ^ v1[5] ^ v2[5] ^ v3[5] ^ v4[5]).to_be_bytes());
    out[56..60].copy_from_slice(&(v0[6] ^ v1[6] ^ v2[6] ^ v3[6] ^ v4[6]).to_be_bytes());
    out[60..64].copy_from_slice(&(v0[7] ^ v1[7] ^ v2[7] ^ v3[7] ^ v4[7]).to_be_bytes());
    out
}
