//! Port of sphlib `cubehash` — CubeHash512 (CubeHash16/32) over exactly one
//! 32-byte message (the only shape the X16RS chain feeds it): init with the
//! IV, one full 32-byte message block through 16 rounds, then the close
//! block (`0x80` padding word) through 11 × 16 rounds with the final `1`
//! XORed into `xv` after the first 16 of them. Faithful to the C
//! `SPH_CUBEHASH_UNROLL == 0` full-unroll path (`SPH_SMALL_FOOTPRINT` off);
//! verified against vectors generated from the C reference
//! (`tests/pure_vectors.rs`).

/// sphlib `CUBE_IV512`.
const IV: [u32; 32] = [
    0x2AEA2A61, 0x50F494D4, 0x2D538B8B, 0x4167D83E, 0x3FEE2313, 0xC701CF8C, 0xCC39968E, 0x50AC5695,
    0x4D42C787, 0xA647A8B3, 0x97CF0BEF, 0x825B4537, 0xEEF864D2, 0xF22090C4, 0xD0E5CD33, 0xA23911AE,
    0xFCD398D9, 0x148FE485, 0x1B017BEF, 0xB6444532, 0x6A536159, 0x2FF5781C, 0x91FA7934, 0x0DBADEA9,
    0xD65C8A2B, 0xA5A70E75, 0xB1C62456, 0xBC796576, 0x1921C8F7, 0xE7989AF1, 0x7795D246, 0xD43E3B44,
];

/// sphlib `ROUND_EVEN`, transcribed statement-for-statement (`T32` ->
/// `wrapping_add`, `ROTL32` -> `rotate_left`).
macro_rules! round_even {
    ($x0:ident, $x1:ident, $x2:ident, $x3:ident, $x4:ident, $x5:ident, $x6:ident, $x7:ident,
     $x8:ident, $x9:ident, $xa:ident, $xb:ident, $xc:ident, $xd:ident, $xe:ident, $xf:ident,
     $xg:ident, $xh:ident, $xi:ident, $xj:ident, $xk:ident, $xl:ident, $xm:ident, $xn:ident,
     $xo:ident, $xp:ident, $xq:ident, $xr:ident, $xs:ident, $xt:ident, $xu:ident, $xv:ident) => {{
        $xg = $x0.wrapping_add($xg);
        $x0 = $x0.rotate_left(7);
        $xh = $x1.wrapping_add($xh);
        $x1 = $x1.rotate_left(7);
        $xi = $x2.wrapping_add($xi);
        $x2 = $x2.rotate_left(7);
        $xj = $x3.wrapping_add($xj);
        $x3 = $x3.rotate_left(7);
        $xk = $x4.wrapping_add($xk);
        $x4 = $x4.rotate_left(7);
        $xl = $x5.wrapping_add($xl);
        $x5 = $x5.rotate_left(7);
        $xm = $x6.wrapping_add($xm);
        $x6 = $x6.rotate_left(7);
        $xn = $x7.wrapping_add($xn);
        $x7 = $x7.rotate_left(7);
        $xo = $x8.wrapping_add($xo);
        $x8 = $x8.rotate_left(7);
        $xp = $x9.wrapping_add($xp);
        $x9 = $x9.rotate_left(7);
        $xq = $xa.wrapping_add($xq);
        $xa = $xa.rotate_left(7);
        $xr = $xb.wrapping_add($xr);
        $xb = $xb.rotate_left(7);
        $xs = $xc.wrapping_add($xs);
        $xc = $xc.rotate_left(7);
        $xt = $xd.wrapping_add($xt);
        $xd = $xd.rotate_left(7);
        $xu = $xe.wrapping_add($xu);
        $xe = $xe.rotate_left(7);
        $xv = $xf.wrapping_add($xv);
        $xf = $xf.rotate_left(7);
        $x8 ^= $xg;
        $x9 ^= $xh;
        $xa ^= $xi;
        $xb ^= $xj;
        $xc ^= $xk;
        $xd ^= $xl;
        $xe ^= $xm;
        $xf ^= $xn;
        $x0 ^= $xo;
        $x1 ^= $xp;
        $x2 ^= $xq;
        $x3 ^= $xr;
        $x4 ^= $xs;
        $x5 ^= $xt;
        $x6 ^= $xu;
        $x7 ^= $xv;
        $xi = $x8.wrapping_add($xi);
        $x8 = $x8.rotate_left(11);
        $xj = $x9.wrapping_add($xj);
        $x9 = $x9.rotate_left(11);
        $xg = $xa.wrapping_add($xg);
        $xa = $xa.rotate_left(11);
        $xh = $xb.wrapping_add($xh);
        $xb = $xb.rotate_left(11);
        $xm = $xc.wrapping_add($xm);
        $xc = $xc.rotate_left(11);
        $xn = $xd.wrapping_add($xn);
        $xd = $xd.rotate_left(11);
        $xk = $xe.wrapping_add($xk);
        $xe = $xe.rotate_left(11);
        $xl = $xf.wrapping_add($xl);
        $xf = $xf.rotate_left(11);
        $xq = $x0.wrapping_add($xq);
        $x0 = $x0.rotate_left(11);
        $xr = $x1.wrapping_add($xr);
        $x1 = $x1.rotate_left(11);
        $xo = $x2.wrapping_add($xo);
        $x2 = $x2.rotate_left(11);
        $xp = $x3.wrapping_add($xp);
        $x3 = $x3.rotate_left(11);
        $xu = $x4.wrapping_add($xu);
        $x4 = $x4.rotate_left(11);
        $xv = $x5.wrapping_add($xv);
        $x5 = $x5.rotate_left(11);
        $xs = $x6.wrapping_add($xs);
        $x6 = $x6.rotate_left(11);
        $xt = $x7.wrapping_add($xt);
        $x7 = $x7.rotate_left(11);
        $xc ^= $xi;
        $xd ^= $xj;
        $xe ^= $xg;
        $xf ^= $xh;
        $x8 ^= $xm;
        $x9 ^= $xn;
        $xa ^= $xk;
        $xb ^= $xl;
        $x4 ^= $xq;
        $x5 ^= $xr;
        $x6 ^= $xo;
        $x7 ^= $xp;
        $x0 ^= $xu;
        $x1 ^= $xv;
        $x2 ^= $xs;
        $x3 ^= $xt;
    }};
}

/// sphlib `ROUND_ODD`, transcribed statement-for-statement.
macro_rules! round_odd {
    ($x0:ident, $x1:ident, $x2:ident, $x3:ident, $x4:ident, $x5:ident, $x6:ident, $x7:ident,
     $x8:ident, $x9:ident, $xa:ident, $xb:ident, $xc:ident, $xd:ident, $xe:ident, $xf:ident,
     $xg:ident, $xh:ident, $xi:ident, $xj:ident, $xk:ident, $xl:ident, $xm:ident, $xn:ident,
     $xo:ident, $xp:ident, $xq:ident, $xr:ident, $xs:ident, $xt:ident, $xu:ident, $xv:ident) => {{
        $xj = $xc.wrapping_add($xj);
        $xc = $xc.rotate_left(7);
        $xi = $xd.wrapping_add($xi);
        $xd = $xd.rotate_left(7);
        $xh = $xe.wrapping_add($xh);
        $xe = $xe.rotate_left(7);
        $xg = $xf.wrapping_add($xg);
        $xf = $xf.rotate_left(7);
        $xn = $x8.wrapping_add($xn);
        $x8 = $x8.rotate_left(7);
        $xm = $x9.wrapping_add($xm);
        $x9 = $x9.rotate_left(7);
        $xl = $xa.wrapping_add($xl);
        $xa = $xa.rotate_left(7);
        $xk = $xb.wrapping_add($xk);
        $xb = $xb.rotate_left(7);
        $xr = $x4.wrapping_add($xr);
        $x4 = $x4.rotate_left(7);
        $xq = $x5.wrapping_add($xq);
        $x5 = $x5.rotate_left(7);
        $xp = $x6.wrapping_add($xp);
        $x6 = $x6.rotate_left(7);
        $xo = $x7.wrapping_add($xo);
        $x7 = $x7.rotate_left(7);
        $xv = $x0.wrapping_add($xv);
        $x0 = $x0.rotate_left(7);
        $xu = $x1.wrapping_add($xu);
        $x1 = $x1.rotate_left(7);
        $xt = $x2.wrapping_add($xt);
        $x2 = $x2.rotate_left(7);
        $xs = $x3.wrapping_add($xs);
        $x3 = $x3.rotate_left(7);
        $x4 ^= $xj;
        $x5 ^= $xi;
        $x6 ^= $xh;
        $x7 ^= $xg;
        $x0 ^= $xn;
        $x1 ^= $xm;
        $x2 ^= $xl;
        $x3 ^= $xk;
        $xc ^= $xr;
        $xd ^= $xq;
        $xe ^= $xp;
        $xf ^= $xo;
        $x8 ^= $xv;
        $x9 ^= $xu;
        $xa ^= $xt;
        $xb ^= $xs;
        $xh = $x4.wrapping_add($xh);
        $x4 = $x4.rotate_left(11);
        $xg = $x5.wrapping_add($xg);
        $x5 = $x5.rotate_left(11);
        $xj = $x6.wrapping_add($xj);
        $x6 = $x6.rotate_left(11);
        $xi = $x7.wrapping_add($xi);
        $x7 = $x7.rotate_left(11);
        $xl = $x0.wrapping_add($xl);
        $x0 = $x0.rotate_left(11);
        $xk = $x1.wrapping_add($xk);
        $x1 = $x1.rotate_left(11);
        $xn = $x2.wrapping_add($xn);
        $x2 = $x2.rotate_left(11);
        $xm = $x3.wrapping_add($xm);
        $x3 = $x3.rotate_left(11);
        $xp = $xc.wrapping_add($xp);
        $xc = $xc.rotate_left(11);
        $xo = $xd.wrapping_add($xo);
        $xd = $xd.rotate_left(11);
        $xr = $xe.wrapping_add($xr);
        $xe = $xe.rotate_left(11);
        $xq = $xf.wrapping_add($xq);
        $xf = $xf.rotate_left(11);
        $xt = $x8.wrapping_add($xt);
        $x8 = $x8.rotate_left(11);
        $xs = $x9.wrapping_add($xs);
        $x9 = $x9.rotate_left(11);
        $xv = $xa.wrapping_add($xv);
        $xa = $xa.rotate_left(11);
        $xu = $xb.wrapping_add($xu);
        $xb = $xb.rotate_left(11);
        $x0 ^= $xh;
        $x1 ^= $xg;
        $x2 ^= $xj;
        $x3 ^= $xi;
        $x4 ^= $xl;
        $x5 ^= $xk;
        $x6 ^= $xn;
        $x7 ^= $xm;
        $x8 ^= $xp;
        $x9 ^= $xo;
        $xa ^= $xr;
        $xb ^= $xq;
        $xc ^= $xt;
        $xd ^= $xs;
        $xe ^= $xv;
        $xf ^= $xu;
    }};
}

/// sphlib `SIXTEEN_ROUNDS` (full-unroll branch).
macro_rules! sixteen_rounds {
    ($x0:ident, $x1:ident, $x2:ident, $x3:ident, $x4:ident, $x5:ident, $x6:ident, $x7:ident,
     $x8:ident, $x9:ident, $xa:ident, $xb:ident, $xc:ident, $xd:ident, $xe:ident, $xf:ident,
     $xg:ident, $xh:ident, $xi:ident, $xj:ident, $xk:ident, $xl:ident, $xm:ident, $xn:ident,
     $xo:ident, $xp:ident, $xq:ident, $xr:ident, $xs:ident, $xt:ident, $xu:ident, $xv:ident) => {{
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_even!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
        round_odd!(
            $x0, $x1, $x2, $x3, $x4, $x5, $x6, $x7, $x8, $x9, $xa, $xb, $xc, $xd, $xe, $xf, $xg,
            $xh, $xi, $xj, $xk, $xl, $xm, $xn, $xo, $xp, $xq, $xr, $xs, $xt, $xu, $xv
        );
    }};
}

/// Full 64-byte digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    // `cubehash_init` with CUBE_IV512.
    let mut x0 = IV[0];
    let mut x1 = IV[1];
    let mut x2 = IV[2];
    let mut x3 = IV[3];
    let mut x4 = IV[4];
    let mut x5 = IV[5];
    let mut x6 = IV[6];
    let mut x7 = IV[7];
    let mut x8 = IV[8];
    let mut x9 = IV[9];
    let mut xa = IV[10];
    let mut xb = IV[11];
    let mut xc = IV[12];
    let mut xd = IV[13];
    let mut xe = IV[14];
    let mut xf = IV[15];
    let mut xg = IV[16];
    let mut xh = IV[17];
    let mut xi = IV[18];
    let mut xj = IV[19];
    let mut xk = IV[20];
    let mut xl = IV[21];
    let mut xm = IV[22];
    let mut xn = IV[23];
    let mut xo = IV[24];
    let mut xp = IV[25];
    let mut xq = IV[26];
    let mut xr = IV[27];
    let mut xs = IV[28];
    let mut xt = IV[29];
    let mut xu = IV[30];
    let mut xv = IV[31];

    // `cubehash_core`: the 32-byte message fills one whole block, so it goes
    // through `INPUT_BLOCK` + `SIXTEEN_ROUNDS` exactly once. The message is
    // XORed into the first eight words as little-endian u32.
    x0 ^= u32::from_le_bytes(msg[0..4].try_into().unwrap());
    x1 ^= u32::from_le_bytes(msg[4..8].try_into().unwrap());
    x2 ^= u32::from_le_bytes(msg[8..12].try_into().unwrap());
    x3 ^= u32::from_le_bytes(msg[12..16].try_into().unwrap());
    x4 ^= u32::from_le_bytes(msg[16..20].try_into().unwrap());
    x5 ^= u32::from_le_bytes(msg[20..24].try_into().unwrap());
    x6 ^= u32::from_le_bytes(msg[24..28].try_into().unwrap());
    x7 ^= u32::from_le_bytes(msg[28..32].try_into().unwrap());
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );

    // `cubehash_close` (ub=0, n=0): appends the block `0x80 || 0*`, i.e. a
    // single 0x00000080 LE word XORed into x0, then 11 more `SIXTEEN_ROUNDS`
    // with `xv ^= 1` injected after the first of them.
    x0 ^= 0x0000_0080;
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    xv ^= 1;
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );
    sixteen_rounds!(
        x0, x1, x2, x3, x4, x5, x6, x7, x8, x9, xa, xb, xc, xd, xe, xf, xg, xh, xi, xj, xk, xl, xm,
        xn, xo, xp, xq, xr, xs, xt, xu, xv
    );

    // Digest: the first 16 state words, little-endian (out_size_w32 = 16).
    let mut out = [0u8; 64];
    out[0..4].copy_from_slice(&x0.to_le_bytes());
    out[4..8].copy_from_slice(&x1.to_le_bytes());
    out[8..12].copy_from_slice(&x2.to_le_bytes());
    out[12..16].copy_from_slice(&x3.to_le_bytes());
    out[16..20].copy_from_slice(&x4.to_le_bytes());
    out[20..24].copy_from_slice(&x5.to_le_bytes());
    out[24..28].copy_from_slice(&x6.to_le_bytes());
    out[28..32].copy_from_slice(&x7.to_le_bytes());
    out[32..36].copy_from_slice(&x8.to_le_bytes());
    out[36..40].copy_from_slice(&x9.to_le_bytes());
    out[40..44].copy_from_slice(&xa.to_le_bytes());
    out[44..48].copy_from_slice(&xb.to_le_bytes());
    out[48..52].copy_from_slice(&xc.to_le_bytes());
    out[52..56].copy_from_slice(&xd.to_le_bytes());
    out[56..60].copy_from_slice(&xe.to_le_bytes());
    out[60..64].copy_from_slice(&xf.to_le_bytes());
    out
}
