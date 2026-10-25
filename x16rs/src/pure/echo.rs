//! Port of sphlib `echo` — ECHO-512 (the `SPH_ECHO_64` big-state path),
//! specialized to a single 32-byte message: one padded 128-byte block, one
//! 10-round compression. Verified against the C reference vectors.

use super::shavite::{aes_rnk, AES0, AES1, AES2, AES3};

/// sphlib `AES_ROUND_LE` (keyed AES round on a 4-word column).
#[inline(always)]
fn aes_round_le(x: [u32; 4], k: [u32; 4]) -> [u32; 4] {
    [
        AES0[(x[0] & 0xFF) as usize]
            ^ AES1[((x[1] >> 8) & 0xFF) as usize]
            ^ AES2[((x[2] >> 16) & 0xFF) as usize]
            ^ AES3[((x[3] >> 24) & 0xFF) as usize]
            ^ k[0],
        AES0[(x[1] & 0xFF) as usize]
            ^ AES1[((x[2] >> 8) & 0xFF) as usize]
            ^ AES2[((x[3] >> 16) & 0xFF) as usize]
            ^ AES3[((x[0] >> 24) & 0xFF) as usize]
            ^ k[1],
        AES0[(x[2] & 0xFF) as usize]
            ^ AES1[((x[3] >> 8) & 0xFF) as usize]
            ^ AES2[((x[0] >> 16) & 0xFF) as usize]
            ^ AES3[((x[1] >> 24) & 0xFF) as usize]
            ^ k[2],
        AES0[(x[3] & 0xFF) as usize]
            ^ AES1[((x[0] >> 8) & 0xFF) as usize]
            ^ AES2[((x[1] >> 16) & 0xFF) as usize]
            ^ AES3[((x[2] >> 24) & 0xFF) as usize]
            ^ k[3],
    ]
}

/// sphlib `AES_2ROUNDS` on state pair `n`, advancing the 256-bit counter.
#[inline(always)]
fn aes_2rounds(w: &mut [[u64; 2]; 16], n: usize, k: &mut [u32; 4]) {
    let x = [
        w[n][0] as u32,
        (w[n][0] >> 32) as u32,
        w[n][1] as u32,
        (w[n][1] >> 32) as u32,
    ];
    let y = aes_round_le(x, *k);
    let x = aes_rnk(y);
    w[n][0] = x[0] as u64 | ((x[1] as u64) << 32);
    w[n][1] = x[2] as u64 | ((x[3] as u64) << 32);
    k[0] = k[0].wrapping_add(1);
    if k[0] == 0 {
        k[1] = k[1].wrapping_add(1);
        if k[1] == 0 {
            k[2] = k[2].wrapping_add(1);
            if k[2] == 0 {
                k[3] = k[3].wrapping_add(1);
            }
        }
    }
}

/// sphlib `SHIFT_ROW1`: W[a]←W[b]←W[c]←W[d]←old W[a].
#[inline(always)]
fn shift_row1(w: &mut [[u64; 2]; 16], a: usize, b: usize, c: usize, d: usize) {
    w.swap(a, b);
    w.swap(b, c);
    w.swap(c, d);
}

/// sphlib `SHIFT_ROW2`: swap a↔c and b↔d.
#[inline(always)]
fn shift_row2(w: &mut [[u64; 2]; 16], a: usize, b: usize, c: usize, d: usize) {
    w.swap(a, c);
    w.swap(b, d);
}

/// sphlib `SHIFT_ROW3(a,b,c,d)` = `SHIFT_ROW1(d,c,b,a)`: W[a]←W[d]←…
#[inline(always)]
fn shift_row3(w: &mut [[u64; 2]; 16], a: usize, b: usize, c: usize, d: usize) {
    w.swap(c, d);
    w.swap(b, c);
    w.swap(a, b);
}

/// AES xtime over a 64-bit word (bytewise, per sphlib `MIX_COLUMN1`).
#[inline(always)]
fn xtime64(v: u64) -> u64 {
    (((v & 0x8080808080808080) >> 7).wrapping_mul(27)) ^ ((v & 0x7F7F7F7F7F7F7F7F) << 1)
}

/// sphlib `MIX_COLUMN` (both 64-bit components).
#[inline(always)]
fn mix_column(w: &mut [[u64; 2]; 16], ia: usize, ib: usize, ic: usize, id: usize) {
    for n in 0..2 {
        let a = w[ia][n];
        let b = w[ib][n];
        let c = w[ic][n];
        let d = w[id][n];
        let ab = a ^ b;
        let bc = b ^ c;
        let cd = c ^ d;
        let abx = xtime64(ab);
        let bcx = xtime64(bc);
        let cdx = xtime64(cd);
        w[ia][n] = abx ^ bc ^ d;
        w[ib][n] = bcx ^ a ^ cd;
        w[ic][n] = cdx ^ ab ^ d;
        w[id][n] = abx ^ bcx ^ cdx ^ ab ^ c;
    }
}

/// Full 64-byte ECHO-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    // echo_big_init(512): pairs 0..7 = (512, 0); the rest is never read
    // before being overwritten.
    let mut v = [[0u64; 2]; 16];
    for p in &mut v[..8] {
        p[0] = 512;
    }

    // echo_big_close padding for a 32-byte buffered message: 0x80, zeros,
    // the 16-bit output-size marker (16 << 5 = 512) at byte 110, then the
    // little-endian 256-bit bit counter (256, 0, 0, 0) at bytes 112..128.
    let mut buf = [0u8; 128];
    buf[..32].copy_from_slice(msg);
    buf[32] = 0x80;
    buf[110..112].copy_from_slice(&(512u16).to_le_bytes());
    buf[112..116].copy_from_slice(&256u32.to_le_bytes());

    // COMPRESS_BIG_echo with K0..K3 = C0..C3 = (256, 0, 0, 0).
    let mut w = [[0u64; 2]; 16];
    w[..8].copy_from_slice(&v[..8]);
    for u in 0..8 {
        w[u + 8][0] = u64::from_le_bytes(buf[16 * u..16 * u + 8].try_into().unwrap());
        w[u + 8][1] = u64::from_le_bytes(buf[16 * u + 8..16 * u + 16].try_into().unwrap());
    }
    let mut k = [256u32, 0, 0, 0];
    macro_rules! big_round {
        () => {
            aes_2rounds(&mut w, 0, &mut k);
            aes_2rounds(&mut w, 1, &mut k);
            aes_2rounds(&mut w, 2, &mut k);
            aes_2rounds(&mut w, 3, &mut k);
            aes_2rounds(&mut w, 4, &mut k);
            aes_2rounds(&mut w, 5, &mut k);
            aes_2rounds(&mut w, 6, &mut k);
            aes_2rounds(&mut w, 7, &mut k);
            aes_2rounds(&mut w, 8, &mut k);
            aes_2rounds(&mut w, 9, &mut k);
            aes_2rounds(&mut w, 10, &mut k);
            aes_2rounds(&mut w, 11, &mut k);
            aes_2rounds(&mut w, 12, &mut k);
            aes_2rounds(&mut w, 13, &mut k);
            aes_2rounds(&mut w, 14, &mut k);
            aes_2rounds(&mut w, 15, &mut k);
            shift_row1(&mut w, 1, 5, 9, 13);
            shift_row2(&mut w, 2, 6, 10, 14);
            shift_row3(&mut w, 3, 7, 11, 15);
            mix_column(&mut w, 0, 1, 2, 3);
            mix_column(&mut w, 4, 5, 6, 7);
            mix_column(&mut w, 8, 9, 10, 11);
            mix_column(&mut w, 12, 13, 14, 15);
        };
    }
    big_round!();
    big_round!();
    big_round!();
    big_round!();
    big_round!();
    big_round!();
    big_round!();
    big_round!();
    big_round!();
    big_round!();

    // FINAL_BIG: flat u64 indexing — VV[u] ^= buf word ^ WW[u] ^ WW[u+16].
    for u in 0..16usize {
        let bufw = u64::from_le_bytes(buf[u * 8..u * 8 + 8].try_into().unwrap());
        v[u >> 1][u & 1] ^= bufw ^ w[u >> 1][u & 1] ^ w[(u >> 1) + 8][u & 1];
    }

    // Digest: VV[0..7] little-endian (pairs 0..3).
    let mut out = [0u8; 64];
    for i in 0..8 {
        out[i * 8..i * 8 + 8].copy_from_slice(&v[i >> 1][i & 1].to_le_bytes());
    }
    out
}
