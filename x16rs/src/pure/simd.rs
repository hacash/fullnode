//! Port of sphlib `simd` — SIMD-512, specialized to a single 32-byte
//! message (two compressions in close: message block and count block).
//! Faithful to the non-small sphlib path; verified against the C
//! reference vectors.

const ALPHA_TAB: [u32; 256] = [
    0x00000001, 0x00000029, 0x0000008B, 0x0000002D, 0x0000002E, 0x00000057,
    0x000000E2, 0x0000000E, 0x0000003C, 0x00000093, 0x00000074, 0x00000082,
    0x000000BE, 0x00000050, 0x000000C4, 0x00000045, 0x00000002, 0x00000052,
    0x00000015, 0x0000005A, 0x0000005C, 0x000000AE, 0x000000C3, 0x0000001C,
    0x00000078, 0x00000025, 0x000000E8, 0x00000003, 0x0000007B, 0x000000A0,
    0x00000087, 0x0000008A, 0x00000004, 0x000000A4, 0x0000002A, 0x000000B4,
    0x000000B8, 0x0000005B, 0x00000085, 0x00000038, 0x000000F0, 0x0000004A,
    0x000000CF, 0x00000006, 0x000000F6, 0x0000003F, 0x0000000D, 0x00000013,
    0x00000008, 0x00000047, 0x00000054, 0x00000067, 0x0000006F, 0x000000B6,
    0x00000009, 0x00000070, 0x000000DF, 0x00000094, 0x0000009D, 0x0000000C,
    0x000000EB, 0x0000007E, 0x0000001A, 0x00000026, 0x00000010, 0x0000008E,
    0x000000A8, 0x000000CE, 0x000000DE, 0x0000006B, 0x00000012, 0x000000E0,
    0x000000BD, 0x00000027, 0x00000039, 0x00000018, 0x000000D5, 0x000000FC,
    0x00000034, 0x0000004C, 0x00000020, 0x0000001B, 0x0000004F, 0x0000009B,
    0x000000BB, 0x000000D6, 0x00000024, 0x000000BF, 0x00000079, 0x0000004E,
    0x00000072, 0x00000030, 0x000000A9, 0x000000F7, 0x00000068, 0x00000098,
    0x00000040, 0x00000036, 0x0000009E, 0x00000035, 0x00000075, 0x000000AB,
    0x00000048, 0x0000007D, 0x000000F2, 0x0000009C, 0x000000E4, 0x00000060,
    0x00000051, 0x000000ED, 0x000000D0, 0x0000002F, 0x00000080, 0x0000006C,
    0x0000003B, 0x0000006A, 0x000000EA, 0x00000055, 0x00000090, 0x000000FA,
    0x000000E3, 0x00000037, 0x000000C7, 0x000000C0, 0x000000A2, 0x000000D9,
    0x0000009F, 0x0000005E, 0x00000100, 0x000000D8, 0x00000076, 0x000000D4,
    0x000000D3, 0x000000AA, 0x0000001F, 0x000000F3, 0x000000C5, 0x0000006E,
    0x0000008D, 0x0000007F, 0x00000043, 0x000000B1, 0x0000003D, 0x000000BC,
    0x000000FF, 0x000000AF, 0x000000EC, 0x000000A7, 0x000000A5, 0x00000053,
    0x0000003E, 0x000000E5, 0x00000089, 0x000000DC, 0x00000019, 0x000000FE,
    0x00000086, 0x00000061, 0x0000007A, 0x00000077, 0x000000FD, 0x0000005D,
    0x000000D7, 0x0000004D, 0x00000049, 0x000000A6, 0x0000007C, 0x000000C9,
    0x00000011, 0x000000B7, 0x00000032, 0x000000FB, 0x0000000B, 0x000000C2,
    0x000000F4, 0x000000EE, 0x000000F9, 0x000000BA, 0x000000AD, 0x0000009A,
    0x00000092, 0x0000004B, 0x000000F8, 0x00000091, 0x00000022, 0x0000006D,
    0x00000064, 0x000000F5, 0x00000016, 0x00000083, 0x000000E7, 0x000000DB,
    0x000000F1, 0x00000073, 0x00000059, 0x00000033, 0x00000023, 0x00000096,
    0x000000EF, 0x00000021, 0x00000044, 0x000000DA, 0x000000C8, 0x000000E9,
    0x0000002C, 0x00000005, 0x000000CD, 0x000000B5, 0x000000E1, 0x000000E6,
    0x000000B2, 0x00000066, 0x00000046, 0x0000002B, 0x000000DD, 0x00000042,
    0x00000088, 0x000000B3, 0x0000008F, 0x000000D1, 0x00000058, 0x0000000A,
    0x00000099, 0x00000069, 0x000000C1, 0x000000CB, 0x00000063, 0x000000CC,
    0x0000008C, 0x00000056, 0x000000B9, 0x00000084, 0x0000000F, 0x00000065,
    0x0000001D, 0x000000A1, 0x000000B0, 0x00000014, 0x00000031, 0x000000D2,
    0x00000081, 0x00000095, 0x000000C6, 0x00000097, 0x00000017, 0x000000AC,
    0x00000071, 0x00000007, 0x0000001E, 0x000000CA, 0x0000003A, 0x00000041,
    0x0000005F, 0x00000028, 0x00000062, 0x000000A3,
];



const YOFF_B_N: [u32; 256] = [
    0x00000001, 0x000000A3, 0x00000062, 0x00000028, 0x0000005F, 0x00000041,
    0x0000003A, 0x000000CA, 0x0000001E, 0x00000007, 0x00000071, 0x000000AC,
    0x00000017, 0x00000097, 0x000000C6, 0x00000095, 0x00000081, 0x000000D2,
    0x00000031, 0x00000014, 0x000000B0, 0x000000A1, 0x0000001D, 0x00000065,
    0x0000000F, 0x00000084, 0x000000B9, 0x00000056, 0x0000008C, 0x000000CC,
    0x00000063, 0x000000CB, 0x000000C1, 0x00000069, 0x00000099, 0x0000000A,
    0x00000058, 0x000000D1, 0x0000008F, 0x000000B3, 0x00000088, 0x00000042,
    0x000000DD, 0x0000002B, 0x00000046, 0x00000066, 0x000000B2, 0x000000E6,
    0x000000E1, 0x000000B5, 0x000000CD, 0x00000005, 0x0000002C, 0x000000E9,
    0x000000C8, 0x000000DA, 0x00000044, 0x00000021, 0x000000EF, 0x00000096,
    0x00000023, 0x00000033, 0x00000059, 0x00000073, 0x000000F1, 0x000000DB,
    0x000000E7, 0x00000083, 0x00000016, 0x000000F5, 0x00000064, 0x0000006D,
    0x00000022, 0x00000091, 0x000000F8, 0x0000004B, 0x00000092, 0x0000009A,
    0x000000AD, 0x000000BA, 0x000000F9, 0x000000EE, 0x000000F4, 0x000000C2,
    0x0000000B, 0x000000FB, 0x00000032, 0x000000B7, 0x00000011, 0x000000C9,
    0x0000007C, 0x000000A6, 0x00000049, 0x0000004D, 0x000000D7, 0x0000005D,
    0x000000FD, 0x00000077, 0x0000007A, 0x00000061, 0x00000086, 0x000000FE,
    0x00000019, 0x000000DC, 0x00000089, 0x000000E5, 0x0000003E, 0x00000053,
    0x000000A5, 0x000000A7, 0x000000EC, 0x000000AF, 0x000000FF, 0x000000BC,
    0x0000003D, 0x000000B1, 0x00000043, 0x0000007F, 0x0000008D, 0x0000006E,
    0x000000C5, 0x000000F3, 0x0000001F, 0x000000AA, 0x000000D3, 0x000000D4,
    0x00000076, 0x000000D8, 0x00000100, 0x0000005E, 0x0000009F, 0x000000D9,
    0x000000A2, 0x000000C0, 0x000000C7, 0x00000037, 0x000000E3, 0x000000FA,
    0x00000090, 0x00000055, 0x000000EA, 0x0000006A, 0x0000003B, 0x0000006C,
    0x00000080, 0x0000002F, 0x000000D0, 0x000000ED, 0x00000051, 0x00000060,
    0x000000E4, 0x0000009C, 0x000000F2, 0x0000007D, 0x00000048, 0x000000AB,
    0x00000075, 0x00000035, 0x0000009E, 0x00000036, 0x00000040, 0x00000098,
    0x00000068, 0x000000F7, 0x000000A9, 0x00000030, 0x00000072, 0x0000004E,
    0x00000079, 0x000000BF, 0x00000024, 0x000000D6, 0x000000BB, 0x0000009B,
    0x0000004F, 0x0000001B, 0x00000020, 0x0000004C, 0x00000034, 0x000000FC,
    0x000000D5, 0x00000018, 0x00000039, 0x00000027, 0x000000BD, 0x000000E0,
    0x00000012, 0x0000006B, 0x000000DE, 0x000000CE, 0x000000A8, 0x0000008E,
    0x00000010, 0x00000026, 0x0000001A, 0x0000007E, 0x000000EB, 0x0000000C,
    0x0000009D, 0x00000094, 0x000000DF, 0x00000070, 0x00000009, 0x000000B6,
    0x0000006F, 0x00000067, 0x00000054, 0x00000047, 0x00000008, 0x00000013,
    0x0000000D, 0x0000003F, 0x000000F6, 0x00000006, 0x000000CF, 0x0000004A,
    0x000000F0, 0x00000038, 0x00000085, 0x0000005B, 0x000000B8, 0x000000B4,
    0x0000002A, 0x000000A4, 0x00000004, 0x0000008A, 0x00000087, 0x000000A0,
    0x0000007B, 0x00000003, 0x000000E8, 0x00000025, 0x00000078, 0x0000001C,
    0x000000C3, 0x000000AE, 0x0000005C, 0x0000005A, 0x00000015, 0x00000052,
    0x00000002, 0x00000045, 0x000000C4, 0x00000050, 0x000000BE, 0x00000082,
    0x00000074, 0x00000093, 0x0000003C, 0x0000000E, 0x000000E2, 0x00000057,
    0x0000002E, 0x0000002D, 0x0000008B, 0x00000029,
];

const YOFF_B_F: [u32; 256] = [
    0x00000002, 0x000000CB, 0x0000009C, 0x0000002F, 0x00000076, 0x000000D6,
    0x0000006B, 0x0000006A, 0x0000002D, 0x0000005D, 0x000000D4, 0x00000014,
    0x0000006F, 0x00000049, 0x000000A2, 0x000000FB, 0x00000061, 0x000000D7,
    0x000000F9, 0x00000035, 0x000000D3, 0x00000013, 0x00000003, 0x00000059,
    0x00000031, 0x000000CF, 0x00000065, 0x00000043, 0x00000097, 0x00000082,
    0x000000DF, 0x00000017, 0x000000BD, 0x000000CA, 0x000000B2, 0x000000EF,
    0x000000FD, 0x0000007F, 0x000000CC, 0x00000031, 0x0000004C, 0x000000EC,
    0x00000052, 0x00000089, 0x000000E8, 0x0000009D, 0x00000041, 0x0000004F,
    0x00000060, 0x000000A1, 0x000000B0, 0x00000082, 0x000000A1, 0x0000001E,
    0x0000002F, 0x00000009, 0x000000BD, 0x000000F7, 0x0000003D, 0x000000E2,
    0x000000F8, 0x0000005A, 0x0000006B, 0x00000040, 0x00000000, 0x00000058,
    0x00000083, 0x000000F3, 0x00000085, 0x0000003B, 0x00000071, 0x00000073,
    0x00000011, 0x000000EC, 0x00000021, 0x000000D5, 0x0000000C, 0x000000BF,
    0x0000006F, 0x00000013, 0x000000FB, 0x0000003D, 0x00000067, 0x000000D0,
    0x00000039, 0x00000023, 0x00000094, 0x000000F8, 0x0000002F, 0x00000074,
    0x00000041, 0x00000077, 0x000000F9, 0x000000B2, 0x0000008F, 0x00000028,
    0x000000BD, 0x00000081, 0x00000008, 0x000000A3, 0x000000CC, 0x000000E3,
    0x000000E6, 0x000000C4, 0x000000CD, 0x0000007A, 0x00000097, 0x0000002D,
    0x000000BB, 0x00000013, 0x000000E3, 0x00000048, 0x000000F7, 0x0000007D,
    0x0000006F, 0x00000079, 0x0000008C, 0x000000DC, 0x00000006, 0x0000006B,
    0x0000004D, 0x00000045, 0x0000000A, 0x00000065, 0x00000015, 0x00000041,
    0x00000095, 0x000000AB, 0x000000FF, 0x00000036, 0x00000065, 0x000000D2,
    0x0000008B, 0x0000002B, 0x00000096, 0x00000097, 0x000000D4, 0x000000A4,
    0x0000002D, 0x000000ED, 0x00000092, 0x000000B8, 0x0000005F, 0x00000006,
    0x000000A0, 0x0000002A, 0x00000008, 0x000000CC, 0x0000002E, 0x000000EE,
    0x000000FE, 0x000000A8, 0x000000D0, 0x00000032, 0x0000009C, 0x000000BE,
    0x0000006A, 0x0000007F, 0x00000022, 0x000000EA, 0x00000044, 0x00000037,
    0x0000004F, 0x00000012, 0x00000004, 0x00000082, 0x00000035, 0x000000D0,
    0x000000B5, 0x00000015, 0x000000AF, 0x00000078, 0x00000019, 0x00000064,
    0x000000C0, 0x000000B2, 0x000000A1, 0x00000060, 0x00000051, 0x0000007F,
    0x00000060, 0x000000E3, 0x000000D2, 0x000000F8, 0x00000044, 0x0000000A,
    0x000000C4, 0x0000001F, 0x00000009, 0x000000A7, 0x00000096, 0x000000C1,
    0x00000000, 0x000000A9, 0x0000007E, 0x0000000E, 0x0000007C, 0x000000C6,
    0x00000090, 0x0000008E, 0x000000F0, 0x00000015, 0x000000E0, 0x0000002C,
    0x000000F5, 0x00000042, 0x00000092, 0x000000EE, 0x00000006, 0x000000C4,
    0x0000009A, 0x00000031, 0x000000C8, 0x000000DE, 0x0000006D, 0x00000009,
    0x000000D2, 0x0000008D, 0x000000C0, 0x0000008A, 0x00000008, 0x0000004F,
    0x00000072, 0x000000D9, 0x00000044, 0x00000080, 0x000000F9, 0x0000005E,
    0x00000035, 0x0000001E, 0x0000001B, 0x0000003D, 0x00000034, 0x00000087,
    0x0000006A, 0x000000D4, 0x00000046, 0x000000EE, 0x0000001E, 0x000000B9,
    0x0000000A, 0x00000084, 0x00000092, 0x00000088, 0x00000075, 0x00000025,
    0x000000FB, 0x00000096, 0x000000B4, 0x000000BC, 0x000000F7, 0x0000009C,
    0x000000EC, 0x000000C0, 0x0000006C, 0x00000056,
];

const IV512: [u32; 32] = [
    0x0BA16B95, 0x72F999AD, 0x9FECC2AE, 0xBA3264FC, 0x5E894929, 0x8E9F30E5,
    0x2F1DAA37, 0xF0F2C558, 0xAC506643, 0xA90635A5, 0xE25B878B, 0xAAB7878F,
    0x88817F7A, 0x0A02892B, 0x559A7550, 0x598F657E, 0x7EEF60A1, 0x6B70E3E8,
    0x9C1714D1, 0xB958E2A8, 0xAB02675E, 0xED1C014F, 0xCD8D65BB, 0xFDB7A257,
    0x09254899, 0xD699C7BC, 0x9019B6DC, 0x2B9022E4, 0x8FA14956, 0x21BF9BD3,
    0xB94D0943, 0x6FFDDC22,
];


#[inline(always)]
fn reds1(x: i32) -> i32 {
    (x & 0xFF).wrapping_sub(x >> 8)
}

#[inline(always)]
fn reds2(x: i32) -> i32 {
    (x & 0xFFFF).wrapping_add(x >> 16)
}

/// sphlib `FFT_LOOP(rb, hk, as, id)`: butterfly over `q[rb..rb+2*hk]`.
/// Element 0 has no twiddle factor; element k uses `alpha_tab[k * as]`.
#[inline(always)]
fn fft_loop(q: &mut [i32], rb: usize, hk: usize, as_: usize) {
    let m = q[rb];
    let n = q[rb + hk];
    q[rb] = m.wrapping_add(n);
    q[rb + hk] = m.wrapping_sub(n);
    for k in 1..hk {
        let m = q[rb + k];
        let n = q[rb + k + hk];
        let t = reds2(n.wrapping_mul(ALPHA_TAB[k * as_] as i32));
        q[rb + k] = m.wrapping_add(t);
        q[rb + k + hk] = m.wrapping_sub(t);
    }
}

/// sphlib `FFT8`: 8-point FFT over 4 message bytes with the fixed
/// alpha-multiplier pattern, reading bytes at stride `xs` from `xb`.
#[inline(always)]
fn fft8(x: &[u8; 128], xb: usize, xs: usize) -> [i32; 8] {
    let x0 = x[xb] as i32;
    let x1 = x[xb + xs] as i32;
    let x2 = x[xb + 2 * xs] as i32;
    let x3 = x[xb + 3 * xs] as i32;
    let a0 = x0 + x2;
    let a1 = x0 + (x2 << 4);
    let a2 = x0 - x2;
    let a3 = x0 - (x2 << 4);
    let b0 = x1 + x3;
    let b1 = reds1((x1 << 2) + (x3 << 6));
    let b2 = (x1 << 4) - (x3 << 4);
    let b3 = reds1((x1 << 6) + (x3 << 2));
    [
        a0 + b0,
        a1 + b1,
        a2 + b2,
        a3 + b3,
        a0 - b0,
        a1 - b1,
        a2 - b2,
        a3 - b3,
    ]
}

/// sphlib `FFT16`.
#[inline(always)]
fn fft16(x: &[u8; 128], xb: usize, xs: usize, q: &mut [i32], rb: usize) {
    let d1 = fft8(x, xb, xs << 1);
    let d2 = fft8(x, xb + xs, xs << 1);
    for j in 0..8 {
        let s = d2[j] << j;
        q[rb + j] = d1[j].wrapping_add(s);
        q[rb + 8 + j] = d1[j].wrapping_sub(s);
    }
}

/// sphlib `FFT32`.
#[inline(always)]
fn fft32(x: &[u8; 128], xb: usize, xs: usize, q: &mut [i32], rb: usize) {
    fft16(x, xb, xs << 1, q, rb);
    fft16(x, xb + xs, xs << 1, q, rb + 16);
    fft_loop(q, rb, 16, 8);
}

/// sphlib `fft64` (the function layer the C adds to keep the unrolled FFT
/// within L1).
#[inline(always)]
fn fft64(x: &[u8; 128], xb: usize, xs: usize, q: &mut [i32], rb: usize) {
    let xd = xs << 1;
    fft32(x, xb, xd, q, rb);
    fft32(x, xb + xs, xd, q, rb + 32);
    fft_loop(q, rb, 32, 4);
}

/// sphlib `FFT256` over the 128-byte block: four `fft64` calls at byte
/// offsets 0, 2, 1, 3 (xs = 1), interleaved with the butterfly loops.
#[inline(always)]
fn fft256(x: &[u8; 128], q: &mut [i32]) {
    fft64(x, 0, 4, q, 0);
    fft64(x, 2, 4, q, 64);
    fft_loop(q, 0, 64, 2);
    fft64(x, 1, 4, q, 128);
    fft64(x, 3, 4, q, 192);
    fft_loop(q, 128, 64, 2);
    fft_loop(q, 0, 128, 1);
}

#[inline(always)]
fn inner(l: i32, h: i32, mm: i32) -> u32 {
    ((l.wrapping_mul(mm) as u32) & 0xFFFF)
        .wrapping_add((h.wrapping_mul(mm) as u32) << 16)
}

/// sphlib `IF`.
#[inline(always)]
fn simd_if(x: u32, y: u32, z: u32) -> u32 {
    ((y ^ z) & x) ^ z
}

/// sphlib `SIMD_MAJ`.
#[inline(always)]
fn simd_maj(x: u32, y: u32, z: u32) -> u32 {
    (x & y) | ((x | y) & z)
}

/// sphlib `STEP_BIG`: the eight A/B/C/D registers as four arrays.
#[inline(always)]
fn step_big(
    a: &mut [u32; 8],
    b: &mut [u32; 8],
    c: &mut [u32; 8],
    d: &mut [u32; 8],
    w: &[u32; 8],
    fun: u8,
    r: u32,
    s: u32,
    base: usize,
) {
    let ta: [u32; 8] = core::array::from_fn(|i| a[i].rotate_left(r));
    for n in 0..8 {
        let f = if fun == 0 {
            simd_if(a[n], b[n], c[n])
        } else {
            simd_maj(a[n], b[n], c[n])
        };
        let tt = d[n].wrapping_add(w[n]).wrapping_add(f);
        a[n] = tt.rotate_left(s).wrapping_add(ta[base ^ n]);
        d[n] = c[n];
        c[n] = b[n];
        b[n] = ta[n];
    }
}


/// Step permutation XOR bases per round/isp. The C selects a `PP8_<m>_`
/// macro whose *name* is not its value: `PP8_0_j = 1^j`, `PP8_1_j = 6^j`,
/// `PP8_2_j = 2^j`, `PP8_3_j = 3^j`, `PP8_4_j = 5^j`, `PP8_5_j = 7^j`,
/// `PP8_6_j = j`. Rows below map the `SIMD_M7_k_isp` names through that

/// (fun, r, s) per step: `ONE_ROUND_BIG(ri, isp, p0..p3)` interleaves

/// sphlib `simd_compress_big` for the 128-byte buffer `x`.
fn compress_big(state: &mut [u32; 32], x: &[u8; 128], last: bool) {
    let mut q = [0i32; 256];
    fft256(x, &mut q);
    let yoff = if last { &YOFF_B_F } else { &YOFF_B_N };
    for i in 0..256 {
        let mut tq = q[i].wrapping_add(yoff[i] as i32);
        tq = reds2(tq);
        tq = reds1(tq);
        tq = reds1(tq);
        q[i] = if tq <= 128 { tq } else { tq - 257 };
    }

    let mut a = [0u32; 8];
    let mut b = [0u32; 8];
    let mut c = [0u32; 8];
    let mut d = [0u32; 8];
    for i in 0..8 {
        a[i] = state[i] ^ u32::from_le_bytes(x[i * 4..i * 4 + 4].try_into().unwrap());
        b[i] = state[8 + i] ^ u32::from_le_bytes(x[32 + i * 4..32 + i * 4 + 4].try_into().unwrap());
        c[i] = state[16 + i] ^ u32::from_le_bytes(x[64 + i * 4..64 + i * 4 + 4].try_into().unwrap());
        d[i] = state[24 + i] ^ u32::from_le_bytes(x[96 + i * 4..96 + i * 4 + 4].try_into().unwrap());
    }
    {
        let w = [
            inner(q[64], q[65], 185),
            inner(q[66], q[67], 185),
            inner(q[68], q[69], 185),
            inner(q[70], q[71], 185),
            inner(q[72], q[73], 185),
            inner(q[74], q[75], 185),
            inner(q[76], q[77], 185),
            inner(q[78], q[79], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 3, 23, 1);
    }
    {
        let w = [
            inner(q[96], q[97], 185),
            inner(q[98], q[99], 185),
            inner(q[100], q[101], 185),
            inner(q[102], q[103], 185),
            inner(q[104], q[105], 185),
            inner(q[106], q[107], 185),
            inner(q[108], q[109], 185),
            inner(q[110], q[111], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 23, 17, 6);
    }
    {
        let w = [
            inner(q[0], q[1], 185),
            inner(q[2], q[3], 185),
            inner(q[4], q[5], 185),
            inner(q[6], q[7], 185),
            inner(q[8], q[9], 185),
            inner(q[10], q[11], 185),
            inner(q[12], q[13], 185),
            inner(q[14], q[15], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 17, 27, 2);
    }
    {
        let w = [
            inner(q[32], q[33], 185),
            inner(q[34], q[35], 185),
            inner(q[36], q[37], 185),
            inner(q[38], q[39], 185),
            inner(q[40], q[41], 185),
            inner(q[42], q[43], 185),
            inner(q[44], q[45], 185),
            inner(q[46], q[47], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 27, 3, 3);
    }
    {
        let w = [
            inner(q[112], q[113], 185),
            inner(q[114], q[115], 185),
            inner(q[116], q[117], 185),
            inner(q[118], q[119], 185),
            inner(q[120], q[121], 185),
            inner(q[122], q[123], 185),
            inner(q[124], q[125], 185),
            inner(q[126], q[127], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 3, 23, 5);
    }
    {
        let w = [
            inner(q[80], q[81], 185),
            inner(q[82], q[83], 185),
            inner(q[84], q[85], 185),
            inner(q[86], q[87], 185),
            inner(q[88], q[89], 185),
            inner(q[90], q[91], 185),
            inner(q[92], q[93], 185),
            inner(q[94], q[95], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 23, 17, 7);
    }
    {
        let w = [
            inner(q[48], q[49], 185),
            inner(q[50], q[51], 185),
            inner(q[52], q[53], 185),
            inner(q[54], q[55], 185),
            inner(q[56], q[57], 185),
            inner(q[58], q[59], 185),
            inner(q[60], q[61], 185),
            inner(q[62], q[63], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 17, 27, 4);
    }
    {
        let w = [
            inner(q[16], q[17], 185),
            inner(q[18], q[19], 185),
            inner(q[20], q[21], 185),
            inner(q[22], q[23], 185),
            inner(q[24], q[25], 185),
            inner(q[26], q[27], 185),
            inner(q[28], q[29], 185),
            inner(q[30], q[31], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 27, 3, 1);
    }
    {
        let w = [
            inner(q[240], q[241], 185),
            inner(q[242], q[243], 185),
            inner(q[244], q[245], 185),
            inner(q[246], q[247], 185),
            inner(q[248], q[249], 185),
            inner(q[250], q[251], 185),
            inner(q[252], q[253], 185),
            inner(q[254], q[255], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 28, 19, 6);
    }
    {
        let w = [
            inner(q[176], q[177], 185),
            inner(q[178], q[179], 185),
            inner(q[180], q[181], 185),
            inner(q[182], q[183], 185),
            inner(q[184], q[185], 185),
            inner(q[186], q[187], 185),
            inner(q[188], q[189], 185),
            inner(q[190], q[191], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 19, 22, 2);
    }
    {
        let w = [
            inner(q[192], q[193], 185),
            inner(q[194], q[195], 185),
            inner(q[196], q[197], 185),
            inner(q[198], q[199], 185),
            inner(q[200], q[201], 185),
            inner(q[202], q[203], 185),
            inner(q[204], q[205], 185),
            inner(q[206], q[207], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 22, 7, 3);
    }
    {
        let w = [
            inner(q[128], q[129], 185),
            inner(q[130], q[131], 185),
            inner(q[132], q[133], 185),
            inner(q[134], q[135], 185),
            inner(q[136], q[137], 185),
            inner(q[138], q[139], 185),
            inner(q[140], q[141], 185),
            inner(q[142], q[143], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 7, 28, 5);
    }
    {
        let w = [
            inner(q[144], q[145], 185),
            inner(q[146], q[147], 185),
            inner(q[148], q[149], 185),
            inner(q[150], q[151], 185),
            inner(q[152], q[153], 185),
            inner(q[154], q[155], 185),
            inner(q[156], q[157], 185),
            inner(q[158], q[159], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 28, 19, 7);
    }
    {
        let w = [
            inner(q[208], q[209], 185),
            inner(q[210], q[211], 185),
            inner(q[212], q[213], 185),
            inner(q[214], q[215], 185),
            inner(q[216], q[217], 185),
            inner(q[218], q[219], 185),
            inner(q[220], q[221], 185),
            inner(q[222], q[223], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 19, 22, 4);
    }
    {
        let w = [
            inner(q[160], q[161], 185),
            inner(q[162], q[163], 185),
            inner(q[164], q[165], 185),
            inner(q[166], q[167], 185),
            inner(q[168], q[169], 185),
            inner(q[170], q[171], 185),
            inner(q[172], q[173], 185),
            inner(q[174], q[175], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 22, 7, 1);
    }
    {
        let w = [
            inner(q[224], q[225], 185),
            inner(q[226], q[227], 185),
            inner(q[228], q[229], 185),
            inner(q[230], q[231], 185),
            inner(q[232], q[233], 185),
            inner(q[234], q[235], 185),
            inner(q[236], q[237], 185),
            inner(q[238], q[239], 185),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 7, 28, 6);
    }
    {
        let w = [
            inner(q[16], q[144], 233),
            inner(q[18], q[146], 233),
            inner(q[20], q[148], 233),
            inner(q[22], q[150], 233),
            inner(q[24], q[152], 233),
            inner(q[26], q[154], 233),
            inner(q[28], q[156], 233),
            inner(q[30], q[158], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 29, 9, 2);
    }
    {
        let w = [
            inner(q[32], q[160], 233),
            inner(q[34], q[162], 233),
            inner(q[36], q[164], 233),
            inner(q[38], q[166], 233),
            inner(q[40], q[168], 233),
            inner(q[42], q[170], 233),
            inner(q[44], q[172], 233),
            inner(q[46], q[174], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 9, 15, 3);
    }
    {
        let w = [
            inner(q[112], q[240], 233),
            inner(q[114], q[242], 233),
            inner(q[116], q[244], 233),
            inner(q[118], q[246], 233),
            inner(q[120], q[248], 233),
            inner(q[122], q[250], 233),
            inner(q[124], q[252], 233),
            inner(q[126], q[254], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 15, 5, 5);
    }
    {
        let w = [
            inner(q[64], q[192], 233),
            inner(q[66], q[194], 233),
            inner(q[68], q[196], 233),
            inner(q[70], q[198], 233),
            inner(q[72], q[200], 233),
            inner(q[74], q[202], 233),
            inner(q[76], q[204], 233),
            inner(q[78], q[206], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 5, 29, 7);
    }
    {
        let w = [
            inner(q[96], q[224], 233),
            inner(q[98], q[226], 233),
            inner(q[100], q[228], 233),
            inner(q[102], q[230], 233),
            inner(q[104], q[232], 233),
            inner(q[106], q[234], 233),
            inner(q[108], q[236], 233),
            inner(q[110], q[238], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 29, 9, 4);
    }
    {
        let w = [
            inner(q[80], q[208], 233),
            inner(q[82], q[210], 233),
            inner(q[84], q[212], 233),
            inner(q[86], q[214], 233),
            inner(q[88], q[216], 233),
            inner(q[90], q[218], 233),
            inner(q[92], q[220], 233),
            inner(q[94], q[222], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 9, 15, 1);
    }
    {
        let w = [
            inner(q[0], q[128], 233),
            inner(q[2], q[130], 233),
            inner(q[4], q[132], 233),
            inner(q[6], q[134], 233),
            inner(q[8], q[136], 233),
            inner(q[10], q[138], 233),
            inner(q[12], q[140], 233),
            inner(q[14], q[142], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 15, 5, 6);
    }
    {
        let w = [
            inner(q[48], q[176], 233),
            inner(q[50], q[178], 233),
            inner(q[52], q[180], 233),
            inner(q[54], q[182], 233),
            inner(q[56], q[184], 233),
            inner(q[58], q[186], 233),
            inner(q[60], q[188], 233),
            inner(q[62], q[190], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 5, 29, 2);
    }
    {
        let w = [
            inner(q[97], q[225], 233),
            inner(q[99], q[227], 233),
            inner(q[101], q[229], 233),
            inner(q[103], q[231], 233),
            inner(q[105], q[233], 233),
            inner(q[107], q[235], 233),
            inner(q[109], q[237], 233),
            inner(q[111], q[239], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 4, 13, 3);
    }
    {
        let w = [
            inner(q[1], q[129], 233),
            inner(q[3], q[131], 233),
            inner(q[5], q[133], 233),
            inner(q[7], q[135], 233),
            inner(q[9], q[137], 233),
            inner(q[11], q[139], 233),
            inner(q[13], q[141], 233),
            inner(q[15], q[143], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 13, 10, 5);
    }
    {
        let w = [
            inner(q[17], q[145], 233),
            inner(q[19], q[147], 233),
            inner(q[21], q[149], 233),
            inner(q[23], q[151], 233),
            inner(q[25], q[153], 233),
            inner(q[27], q[155], 233),
            inner(q[29], q[157], 233),
            inner(q[31], q[159], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 10, 25, 7);
    }
    {
        let w = [
            inner(q[113], q[241], 233),
            inner(q[115], q[243], 233),
            inner(q[117], q[245], 233),
            inner(q[119], q[247], 233),
            inner(q[121], q[249], 233),
            inner(q[123], q[251], 233),
            inner(q[125], q[253], 233),
            inner(q[127], q[255], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, 25, 4, 4);
    }
    {
        let w = [
            inner(q[49], q[177], 233),
            inner(q[51], q[179], 233),
            inner(q[53], q[181], 233),
            inner(q[55], q[183], 233),
            inner(q[57], q[185], 233),
            inner(q[59], q[187], 233),
            inner(q[61], q[189], 233),
            inner(q[63], q[191], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 4, 13, 1);
    }
    {
        let w = [
            inner(q[81], q[209], 233),
            inner(q[83], q[211], 233),
            inner(q[85], q[213], 233),
            inner(q[87], q[215], 233),
            inner(q[89], q[217], 233),
            inner(q[91], q[219], 233),
            inner(q[93], q[221], 233),
            inner(q[95], q[223], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 13, 10, 6);
    }
    {
        let w = [
            inner(q[65], q[193], 233),
            inner(q[67], q[195], 233),
            inner(q[69], q[197], 233),
            inner(q[71], q[199], 233),
            inner(q[73], q[201], 233),
            inner(q[75], q[203], 233),
            inner(q[77], q[205], 233),
            inner(q[79], q[207], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 10, 25, 2);
    }
    {
        let w = [
            inner(q[33], q[161], 233),
            inner(q[35], q[163], 233),
            inner(q[37], q[165], 233),
            inner(q[39], q[167], 233),
            inner(q[41], q[169], 233),
            inner(q[43], q[171], 233),
            inner(q[45], q[173], 233),
            inner(q[47], q[175], 233),
        ];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 1, 25, 4, 3);
    }

    // Final four STEP_BIGs over the saved (pre-round) state.
    let saved = *state;
    for half in 0..4 {
        let mut w = [0u32; 8];
        w.copy_from_slice(&saved[half * 8..half * 8 + 8]);
        // PP8_4_/5_/6_/0_ names → xor bases 5/7/4/1 (see ROUND_BASE note).
        let (base, r, s) = [(5usize, 4u32, 13u32), (7, 13, 10), (4, 10, 25), (1, 25, 4)][half];
        step_big(&mut a, &mut b, &mut c, &mut d, &w, 0, r, s, base);
    }

    for i in 0..8 {
        state[i] = a[i];
        state[8 + i] = b[i];
        state[16 + i] = c[i];
        state[24 + i] = d[i];
    }
}

/// Full 64-byte SIMD-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    let mut state = IV512;
    let mut buf = [0u8; 128];

    // finalize_big with 32 buffered bytes: one non-final compress over
    // msg || zeros, then a final compress over the count block
    // (little-endian bit count 256 at bytes 0..8).
    buf[..32].copy_from_slice(msg);
    compress_big(&mut state, &buf, false);
    buf = [0u8; 128];
    buf[0..4].copy_from_slice(&256u32.to_le_bytes());
    compress_big(&mut state, &buf, true);

    let mut out = [0u8; 64];
    for (i, w) in state[..16].iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    out
}
