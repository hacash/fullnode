//! Port of sphlib `hamsi` — Hamsi-512 (the big 512-bit state path with
//! `SPH_HAMSI_EXPAND_BIG == 8` message expansion), specialized to a single
//! 32-byte message: four 8-byte P blocks, the 0x80 pad block, and the final
//! length block. Verified against the C reference vectors.

include!("hamsi_tables.rs");

/// The 32 permutation cells map to `{m0..mF, c0..cF}` in the fixed interleaved
/// order the C `s00..s1F` defines use; cell index == `s` number.
const M_CELLS: [usize; 16] = [0, 1, 4, 5, 10, 11, 14, 15, 16, 17, 20, 21, 26, 27, 30, 31];
const C_CELL: [usize; 16] = [
    0x02, 0x03, 0x06, 0x07, 0x08, 0x09, 0x0C, 0x0D, 0x12, 0x13, 0x16, 0x17, 0x18, 0x19, 0x1C,
    0x1D,
];

macro_rules! sbox {
    ($s:expr, $a:literal, $b:literal, $c:literal, $d:literal) => {{
        let t = $s[$a];
        $s[$a] &= $s[$c];
        $s[$a] ^= $s[$d];
        $s[$c] ^= $s[$b];
        $s[$c] ^= $s[$a];
        $s[$d] |= t;
        $s[$d] ^= $s[$b];
        let t = t ^ $s[$c];
        $s[$b] = $s[$d];
        $s[$d] |= t;
        $s[$d] ^= $s[$a];
        $s[$a] &= $s[$b];
        let t = t ^ $s[$a];
        $s[$b] ^= $s[$d];
        $s[$b] ^= t;
        $s[$a] = $s[$c];
        $s[$c] = $s[$b];
        $s[$b] = $s[$d];
        $s[$d] = !t;
    }};
}

macro_rules! hamsi_l {
    ($s:expr, $a:literal, $b:literal, $c:literal, $d:literal) => {{
        $s[$a] = $s[$a].rotate_left(13);
        $s[$c] = $s[$c].rotate_left(3);
        $s[$b] ^= $s[$a] ^ $s[$c];
        $s[$d] ^= $s[$c] ^ ($s[$a] << 3);
        $s[$b] = $s[$b].rotate_left(1);
        $s[$d] = $s[$d].rotate_left(7);
        $s[$a] ^= $s[$b] ^ $s[$d];
        $s[$c] ^= $s[$d] ^ ($s[$b] << 7);
        $s[$a] = $s[$a].rotate_left(5);
        $s[$c] = $s[$c].rotate_left(22);
    }};
}

/// sphlib `ROUND_BIG(rc, alpha)`.
macro_rules! round_big {
    ($s:expr, $rc:expr, $alpha:expr) => {{
        for i in 0..32 {
            $s[i] ^= $alpha[i];
        }
        $s[1] ^= $rc;
        sbox!($s, 0, 8, 16, 24);
        sbox!($s, 1, 9, 17, 25);
        sbox!($s, 2, 10, 18, 26);
        sbox!($s, 3, 11, 19, 27);
        sbox!($s, 4, 12, 20, 28);
        sbox!($s, 5, 13, 21, 29);
        sbox!($s, 6, 14, 22, 30);
        sbox!($s, 7, 15, 23, 31);
        hamsi_l!($s, 0, 9, 18, 27);
        hamsi_l!($s, 1, 10, 19, 28);
        hamsi_l!($s, 2, 11, 20, 29);
        hamsi_l!($s, 3, 12, 21, 30);
        hamsi_l!($s, 4, 13, 22, 31);
        hamsi_l!($s, 5, 14, 23, 24);
        hamsi_l!($s, 6, 15, 16, 25);
        hamsi_l!($s, 7, 8, 17, 26);
        hamsi_l!($s, 0, 2, 5, 7);
        hamsi_l!($s, 16, 19, 21, 22);
        hamsi_l!($s, 9, 11, 12, 14);
        hamsi_l!($s, 25, 26, 28, 31);
    }};
}

/// One `hamsi_big` block: read state, `INPUT_BIG` expansion, then `P_BIG`
/// (6 rounds, normal) or `PF_BIG` (12 rounds, final), then `T_BIG`.
fn block(h: &mut [u32; 16], msg8: &[u8; 8], final_block: bool) {
    let mut s = [0u32; 32];
    for (i, &cell) in C_CELL.iter().enumerate() {
        s[cell] = h[i];
    }
    // INPUT_BIG: m0..mF = XOR of the eight per-byte table rows (T512_0,
    // T512_8, ... T512_56, one per message byte).
    {
        const TABLES: [&[[u32; 16]; 256]; 8] = [
            &T512_0, &T512_8, &T512_16, &T512_24, &T512_32, &T512_40, &T512_48, &T512_56,
        ];
        let mut m = [0u32; 16];
        for k in 0..8 {
            let row = &TABLES[k][msg8[k] as usize];
            for j in 0..16 {
                m[j] ^= row[j];
            }
        }
        for (i, &cell) in M_CELLS.iter().enumerate() {
            s[cell] = m[i];
        }
    }
    if final_block {
        for r in 0..12u32 {
            round_big!(s, r, alpha_f);
        }
    } else {
        for r in 0..6u32 {
            round_big!(s, r, alpha_n);
        }
    }
    // T_BIG: the C pairs h[i] with cells s00..s07 then s10..s17 — note the
    // second row skips s08..s0F — and stores each XOR result into the
    // corresponding c-variable, which the next block's READ_STATE re-reads
    // (captured here by re-initializing from h at block entry).
    const T_CELL: [usize; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15,
        0x16, 0x17,
    ];
    for (i, &cell) in T_CELL.iter().enumerate() {
        h[i] ^= s[cell];
    }
}

/// Full 64-byte Hamsi-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    let mut h = HAMSI_IV512;
    for chunk in msg.chunks_exact(8) {
        block(&mut h, chunk.try_into().unwrap(), false);
    }
    // close: pad block 0x80 || zeros, then the big-endian bit count 256.
    block(&mut h, &[0x80, 0, 0, 0, 0, 0, 0, 0], false);
    block(&mut h, &[0, 0, 0, 0, 0, 0, 1, 0], true);

    let mut out = [0u8; 64];
    for (i, w) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}
