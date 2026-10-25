//! Port of sphlib `bmw` — BMW-512 over exactly one 32-byte message (the only
//! shape the X16RS chain feeds it), on the `SPH_64` non-`SPH_SMALL_FOOTPRINT`
//! code path. The chain is `bmw64_init(BMW_IV512)` + `bmw64(data, 32)` (which
//! only buffers — 32 < 128, no compression yet) + `bmw64_close`: one
//! compression over the padded 128-byte block, a second compression whose
//! message is the first output and whose "H" is `final_b`, then the last 8 of
//! the 16 result words in little-endian. Verified against vectors generated
//! from the C reference (`tests/pure_vectors.rs`).

/// sphlib `BMW_IV512`.
const IV512: [u64; 16] = [
    0x8081828384858687,
    0x88898A8B8C8D8E8F,
    0x9091929394959697,
    0x98999A9B9C9D9E9F,
    0xA0A1A2A3A4A5A6A7,
    0xA8A9AAABACADAEAF,
    0xB0B1B2B3B4B5B6B7,
    0xB8B9BABBBCBDBEBF,
    0xC0C1C2C3C4C5C6C7,
    0xC8C9CACBCCCDCECF,
    0xD0D1D2D3D4D5D6D7,
    0xD8D9DADBDCDDDEDF,
    0xE0E1E2E3E4E5E6E7,
    0xE8E9EAEBECEDEEEF,
    0xF0F1F2F3F4F5F6F7,
    0xF8F9FAFBFCFDFEFF,
];

/// sphlib `final_b` (the fixed second-finalization block).
const FINAL_B: [u64; 16] = [
    0xAAAAAAAAAAAAAAA0,
    0xAAAAAAAAAAAAAAA1,
    0xAAAAAAAAAAAAAAA2,
    0xAAAAAAAAAAAAAAA3,
    0xAAAAAAAAAAAAAAA4,
    0xAAAAAAAAAAAAAAA5,
    0xAAAAAAAAAAAAAAA6,
    0xAAAAAAAAAAAAAAA7,
    0xAAAAAAAAAAAAAAA8,
    0xAAAAAAAAAAAAAAA9,
    0xAAAAAAAAAAAAAAAA,
    0xAAAAAAAAAAAAAAAB,
    0xAAAAAAAAAAAAAAAC,
    0xAAAAAAAAAAAAAAAD,
    0xAAAAAAAAAAAAAAAE,
    0xAAAAAAAAAAAAAAAF,
];

/// sphlib `M16_16..M16_31`: for expansion step `j` (16..31), the tuple holds
/// `(j0m, j1m, j3m, j4m, j7m, j10m, j11m)`; positions 0/2/5 are message-word
/// indices (all <= 15), positions 1/3/6 are rotate amounts (<= 16), position
/// 4 is the H index.
const M16: [[usize; 7]; 16] = [
    [0, 1, 3, 4, 7, 10, 11],
    [1, 2, 4, 5, 8, 11, 12],
    [2, 3, 5, 6, 9, 12, 13],
    [3, 4, 6, 7, 10, 13, 14],
    [4, 5, 7, 8, 11, 14, 15],
    [5, 6, 8, 9, 12, 15, 16],
    [6, 7, 9, 10, 13, 0, 1],
    [7, 8, 10, 11, 14, 1, 2],
    [8, 9, 11, 12, 15, 2, 3],
    [9, 10, 12, 13, 0, 3, 4],
    [10, 11, 13, 14, 1, 4, 5],
    [11, 12, 14, 15, 2, 5, 6],
    [12, 13, 15, 16, 3, 6, 7],
    [13, 14, 0, 1, 4, 7, 8],
    [14, 15, 1, 2, 5, 8, 9],
    [15, 16, 2, 3, 6, 9, 10],
];

/// sphlib `Wb0..Wb15` (identical index pattern to `Ws0..Ws15`): each entry is
/// the five `(M(i) ^ H(i))` operand indices and a bitmask whose bit k set
/// means the k-th combining op (between operand k and k+1) is a subtraction
/// instead of an addition.
const WTAB: [([usize; 5], u8); 16] = [
    ([5, 7, 10, 13, 14], 0b0001),
    ([6, 8, 11, 14, 15], 0b1001),
    ([0, 7, 9, 12, 15], 0b0100),
    ([0, 1, 8, 10, 13], 0b0101),
    ([1, 2, 9, 11, 14], 0b1100),
    ([3, 2, 10, 12, 15], 0b0101),
    ([4, 0, 3, 11, 13], 0b0111),
    ([1, 4, 5, 12, 14], 0b1111),
    ([2, 5, 6, 13, 15], 0b1011),
    ([0, 3, 6, 7, 14], 0b0101),
    ([8, 1, 4, 7, 15], 0b0111),
    ([8, 0, 2, 5, 9], 0b0111),
    ([1, 3, 6, 9, 10], 0b0110),
    ([2, 4, 7, 10, 11], 0b0000),
    ([3, 5, 8, 11, 12], 0b1101),
    ([12, 4, 6, 9, 13], 0b0111),
];

#[inline(always)]
fn sb0(x: u64) -> u64 {
    (x >> 1) ^ (x << 3) ^ x.rotate_left(4) ^ x.rotate_left(37)
}

#[inline(always)]
fn sb1(x: u64) -> u64 {
    (x >> 1) ^ (x << 2) ^ x.rotate_left(13) ^ x.rotate_left(43)
}

#[inline(always)]
fn sb2(x: u64) -> u64 {
    (x >> 2) ^ (x << 1) ^ x.rotate_left(19) ^ x.rotate_left(53)
}

#[inline(always)]
fn sb3(x: u64) -> u64 {
    (x >> 2) ^ (x << 2) ^ x.rotate_left(28) ^ x.rotate_left(59)
}

#[inline(always)]
fn sb4(x: u64) -> u64 {
    (x >> 1) ^ x
}

#[inline(always)]
fn sb5(x: u64) -> u64 {
    (x >> 2) ^ x
}

#[inline(always)]
fn rb1(x: u64) -> u64 {
    x.rotate_left(5)
}

#[inline(always)]
fn rb2(x: u64) -> u64 {
    x.rotate_left(11)
}

#[inline(always)]
fn rb3(x: u64) -> u64 {
    x.rotate_left(27)
}

#[inline(always)]
fn rb4(x: u64) -> u64 {
    x.rotate_left(32)
}

#[inline(always)]
fn rb5(x: u64) -> u64 {
    x.rotate_left(37)
}

#[inline(always)]
fn rb6(x: u64) -> u64 {
    x.rotate_left(43)
}

#[inline(always)]
fn rb7(x: u64) -> u64 {
    x.rotate_left(53)
}

/// sphlib `Kb(j)`.
#[inline(always)]
fn kb(j: u64) -> u64 {
    j.wrapping_mul(0x0555555555555555)
}

/// sphlib `MAKE_W` for word `j`: `T64((M(i0)^H(i0)) op01 (M(i1)^H(i1)) op12
/// (M(i2)^H(i2)) op23 (M(i3)^H(i3)) op34 (M(i4)^H(i4)))`.
#[inline(always)]
fn wb(j: usize, m: &[u64; 16], h: &[u64; 16]) -> u64 {
    let (ix, ops) = &WTAB[j];
    let x = |i: usize| m[i] ^ h[i];
    let mut acc = x(ix[0]);
    for k in 0..4 {
        let t = x(ix[k + 1]);
        acc = if ops & (1 << k) != 0 {
            acc.wrapping_sub(t)
        } else {
            acc.wrapping_add(t)
        };
    }
    acc
}

/// sphlib `add_elt_b(mf, hf, j0m, j1m, j3m, j4m, j7m, j10m, j11m, j16)`.
#[inline(always)]
fn add_elt_b(m: &[u64; 16], h: &[u64; 16], j: usize, iy: [usize; 7]) -> u64 {
    m[iy[0]]
        .rotate_left(iy[1] as u32)
        .wrapping_add(m[iy[2]].rotate_left(iy[3] as u32))
        .wrapping_sub(m[iy[5]].rotate_left(iy[6] as u32))
        .wrapping_add(kb(j as u64))
        ^ h[iy[4]]
}

/// sphlib `expand1b(Qb, M, H, j)` for j in 16..18: `I16_j` is simply
/// `j-16 .. j-1`, and the sigma pattern is `sb1 sb2 sb3 sb0` repeating.
#[inline(always)]
fn expand1(q: &[u64; 32], m: &[u64; 16], h: &[u64; 16], j: usize) -> u64 {
    let b = j - 16;
    sb1(q[b])
        .wrapping_add(sb2(q[b + 1]))
        .wrapping_add(sb3(q[b + 2]))
        .wrapping_add(sb0(q[b + 3]))
        .wrapping_add(sb1(q[b + 4]))
        .wrapping_add(sb2(q[b + 5]))
        .wrapping_add(sb3(q[b + 6]))
        .wrapping_add(sb0(q[b + 7]))
        .wrapping_add(sb1(q[b + 8]))
        .wrapping_add(sb2(q[b + 9]))
        .wrapping_add(sb3(q[b + 10]))
        .wrapping_add(sb0(q[b + 11]))
        .wrapping_add(sb1(q[b + 12]))
        .wrapping_add(sb2(q[b + 13]))
        .wrapping_add(sb3(q[b + 14]))
        .wrapping_add(sb0(q[b + 15]))
        .wrapping_add(add_elt_b(m, h, j, M16[j - 16]))
}

/// sphlib `expand2b(Qb, M, H, j)` for j in 18..32.
#[inline(always)]
fn expand2(q: &[u64; 32], m: &[u64; 16], h: &[u64; 16], j: usize) -> u64 {
    let b = j - 16;
    q[b].wrapping_add(rb1(q[b + 1]))
        .wrapping_add(q[b + 2])
        .wrapping_add(rb2(q[b + 3]))
        .wrapping_add(q[b + 4])
        .wrapping_add(rb3(q[b + 5]))
        .wrapping_add(q[b + 6])
        .wrapping_add(rb4(q[b + 7]))
        .wrapping_add(q[b + 8])
        .wrapping_add(rb5(q[b + 9]))
        .wrapping_add(q[b + 10])
        .wrapping_add(rb6(q[b + 11]))
        .wrapping_add(q[b + 12])
        .wrapping_add(rb7(q[b + 13]))
        .wrapping_add(sb4(q[b + 14]))
        .wrapping_add(sb5(q[b + 15]))
        .wrapping_add(add_elt_b(m, h, j, M16[j - 16]))
}

/// sphlib `FOLDb`: one BMW-512 compression of message block `m` into state
/// `h`, returning the new state `dH`.
fn fold(h: &[u64; 16], m: &[u64; 16]) -> [u64; 16] {
    let mut qt = [0u64; 32];

    // MAKE_Qab: qt[j] = sb_(j%5)(Wb_j) + H((j+1) & 15).
    macro_rules! qab {
        ($j:literal, $sb:ident) => {
            qt[$j] = $sb(wb($j, m, h)).wrapping_add(h[($j + 1) & 15]);
        };
    }
    qab!(0, sb0);
    qab!(1, sb1);
    qab!(2, sb2);
    qab!(3, sb3);
    qab!(4, sb4);
    qab!(5, sb0);
    qab!(6, sb1);
    qab!(7, sb2);
    qab!(8, sb3);
    qab!(9, sb4);
    qab!(10, sb0);
    qab!(11, sb1);
    qab!(12, sb2);
    qab!(13, sb3);
    qab!(14, sb4);
    qab!(15, sb0);

    // MAKE_Qbb.
    qt[16] = expand1(&qt, m, h, 16);
    qt[17] = expand1(&qt, m, h, 17);
    qt[18] = expand2(&qt, m, h, 18);
    qt[19] = expand2(&qt, m, h, 19);
    qt[20] = expand2(&qt, m, h, 20);
    qt[21] = expand2(&qt, m, h, 21);
    qt[22] = expand2(&qt, m, h, 22);
    qt[23] = expand2(&qt, m, h, 23);
    qt[24] = expand2(&qt, m, h, 24);
    qt[25] = expand2(&qt, m, h, 25);
    qt[26] = expand2(&qt, m, h, 26);
    qt[27] = expand2(&qt, m, h, 27);
    qt[28] = expand2(&qt, m, h, 28);
    qt[29] = expand2(&qt, m, h, 29);
    qt[30] = expand2(&qt, m, h, 30);
    qt[31] = expand2(&qt, m, h, 31);

    let xl = qt[16] ^ qt[17] ^ qt[18] ^ qt[19] ^ qt[20] ^ qt[21] ^ qt[22] ^ qt[23];
    let xh = xl ^ qt[24] ^ qt[25] ^ qt[26] ^ qt[27] ^ qt[28] ^ qt[29] ^ qt[30] ^ qt[31];

    let mut dh = [0u64; 16];
    dh[0] = ((xh << 5) ^ (qt[16] >> 5) ^ m[0]).wrapping_add(xl ^ qt[24] ^ qt[0]);
    dh[1] = ((xh >> 7) ^ (qt[17] << 8) ^ m[1]).wrapping_add(xl ^ qt[25] ^ qt[1]);
    dh[2] = ((xh >> 5) ^ (qt[18] << 5) ^ m[2]).wrapping_add(xl ^ qt[26] ^ qt[2]);
    dh[3] = ((xh >> 1) ^ (qt[19] << 5) ^ m[3]).wrapping_add(xl ^ qt[27] ^ qt[3]);
    dh[4] = ((xh >> 3) ^ (qt[20] << 0) ^ m[4]).wrapping_add(xl ^ qt[28] ^ qt[4]);
    dh[5] = ((xh << 6) ^ (qt[21] >> 6) ^ m[5]).wrapping_add(xl ^ qt[29] ^ qt[5]);
    dh[6] = ((xh >> 4) ^ (qt[22] << 6) ^ m[6]).wrapping_add(xl ^ qt[30] ^ qt[6]);
    dh[7] = ((xh >> 11) ^ (qt[23] << 2) ^ m[7]).wrapping_add(xl ^ qt[31] ^ qt[7]);
    dh[8] = dh[4]
        .rotate_left(9)
        .wrapping_add(xh ^ qt[24] ^ m[8])
        .wrapping_add((xl << 8) ^ qt[23] ^ qt[8]);
    dh[9] = dh[5]
        .rotate_left(10)
        .wrapping_add(xh ^ qt[25] ^ m[9])
        .wrapping_add((xl >> 6) ^ qt[16] ^ qt[9]);
    dh[10] = dh[6]
        .rotate_left(11)
        .wrapping_add(xh ^ qt[26] ^ m[10])
        .wrapping_add((xl << 6) ^ qt[17] ^ qt[10]);
    dh[11] = dh[7]
        .rotate_left(12)
        .wrapping_add(xh ^ qt[27] ^ m[11])
        .wrapping_add((xl << 4) ^ qt[18] ^ qt[11]);
    dh[12] = dh[0]
        .rotate_left(13)
        .wrapping_add(xh ^ qt[28] ^ m[12])
        .wrapping_add((xl >> 3) ^ qt[19] ^ qt[12]);
    dh[13] = dh[1]
        .rotate_left(14)
        .wrapping_add(xh ^ qt[29] ^ m[13])
        .wrapping_add((xl >> 4) ^ qt[20] ^ qt[13]);
    dh[14] = dh[2]
        .rotate_left(15)
        .wrapping_add(xh ^ qt[30] ^ m[14])
        .wrapping_add((xl >> 7) ^ qt[21] ^ qt[14]);
    dh[15] = dh[3]
        .rotate_left(16)
        .wrapping_add(xh ^ qt[31] ^ m[15])
        .wrapping_add((xl >> 2) ^ qt[22] ^ qt[15]);
    dh
}

/// Full 64-byte BMW-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    // `bmw64` with len=32 only buffers (128-byte block); `bmw64_close` pads:
    // message, 0x80, zeros, and the 64-bit bit length (32 * 8 = 256) LE at
    // the end.
    let mut m1 = [0u64; 16];
    for i in 0..4 {
        m1[i] = u64::from_le_bytes(msg[i * 8..i * 8 + 8].try_into().unwrap());
    }
    m1[4] = 0x80;
    m1[15] = 256;

    // First compression over the padded block (state is still the IV since
    // no full block was processed), then the finalization compression whose
    // message is the first result and whose H is `final_b`.
    let h2 = fold(&IV512, &m1);
    let h1 = fold(&FINAL_B, &h2);

    // out_size_w64 = 8: words h1[8..16], little-endian.
    let mut out = [0u8; 64];
    for i in 0..8 {
        out[i * 8..i * 8 + 8].copy_from_slice(&h1[i + 8].to_le_bytes());
    }
    out
}
