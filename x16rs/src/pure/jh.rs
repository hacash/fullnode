//! Port of sphlib `jh` — JH-512 over exactly one 32-byte message (the only
//! shape the X16RS chain feeds it). The 32 bytes buffer in `jh_core`, and
//! `jh_close` pads a 96-byte trailer, so the whole hash is two E8
//! compressions: block A = msg || 0x80 || zeros, block B = zeros || the
//! 128-bit big-endian bit length. Faithful to the sphlib `SPH_64` +
//! `SPH_JH_64` + `SPH_LITTLE_ENDIAN` non-`SPH_SMALL_FOOTPRINT` path (fully
//! unrolled E8, 8xu64 state); verified against vectors generated from the C
//! reference (`tests/pure_vectors.rs`).

/// `C64e` from jh.c: the round constants are byte-swapped under
/// `SPH_LITTLE_ENDIAN`. Applied at compile time so the literals below stay
/// exactly the ones in the C table.
const fn c64e(x: u64) -> u64 {
    x.swap_bytes()
}

/// sphlib `C[]` round constants (42 rounds x 4 words: even-hi, even-lo,
/// odd-hi, odd-lo), stored byte-swapped exactly as the C initializer.
const C: [u64; 168] = [
    c64e(0x72d5dea2df15f867),
    c64e(0x7b84150ab7231557),
    c64e(0x81abd6904d5a87f6),
    c64e(0x4e9f4fc5c3d12b40),
    c64e(0xea983ae05c45fa9c),
    c64e(0x03c5d29966b2999a),
    c64e(0x660296b4f2bb538a),
    c64e(0xb556141a88dba231),
    c64e(0x03a35a5c9a190edb),
    c64e(0x403fb20a87c14410),
    c64e(0x1c051980849e951d),
    c64e(0x6f33ebad5ee7cddc),
    c64e(0x10ba139202bf6b41),
    c64e(0xdc786515f7bb27d0),
    c64e(0x0a2c813937aa7850),
    c64e(0x3f1abfd2410091d3),
    c64e(0x422d5a0df6cc7e90),
    c64e(0xdd629f9c92c097ce),
    c64e(0x185ca70bc72b44ac),
    c64e(0xd1df65d663c6fc23),
    c64e(0x976e6c039ee0b81a),
    c64e(0x2105457e446ceca8),
    c64e(0xeef103bb5d8e61fa),
    c64e(0xfd9697b294838197),
    c64e(0x4a8e8537db03302f),
    c64e(0x2a678d2dfb9f6a95),
    c64e(0x8afe7381f8b8696c),
    c64e(0x8ac77246c07f4214),
    c64e(0xc5f4158fbdc75ec4),
    c64e(0x75446fa78f11bb80),
    c64e(0x52de75b7aee488bc),
    c64e(0x82b8001e98a6a3f4),
    c64e(0x8ef48f33a9a36315),
    c64e(0xaa5f5624d5b7f989),
    c64e(0xb6f1ed207c5ae0fd),
    c64e(0x36cae95a06422c36),
    c64e(0xce2935434efe983d),
    c64e(0x533af974739a4ba7),
    c64e(0xd0f51f596f4e8186),
    c64e(0x0e9dad81afd85a9f),
    c64e(0xa7050667ee34626a),
    c64e(0x8b0b28be6eb91727),
    c64e(0x47740726c680103f),
    c64e(0xe0a07e6fc67e487b),
    c64e(0x0d550aa54af8a4c0),
    c64e(0x91e3e79f978ef19e),
    c64e(0x8676728150608dd4),
    c64e(0x7e9e5a41f3e5b062),
    c64e(0xfc9f1fec4054207a),
    c64e(0xe3e41a00cef4c984),
    c64e(0x4fd794f59dfa95d8),
    c64e(0x552e7e1124c354a5),
    c64e(0x5bdf7228bdfe6e28),
    c64e(0x78f57fe20fa5c4b2),
    c64e(0x05897cefee49d32e),
    c64e(0x447e9385eb28597f),
    c64e(0x705f6937b324314a),
    c64e(0x5e8628f11dd6e465),
    c64e(0xc71b770451b920e7),
    c64e(0x74fe43e823d4878a),
    c64e(0x7d29e8a3927694f2),
    c64e(0xddcb7a099b30d9c1),
    c64e(0x1d1b30fb5bdc1be0),
    c64e(0xda24494ff29c82bf),
    c64e(0xa4e7ba31b470bfff),
    c64e(0x0d324405def8bc48),
    c64e(0x3baefc3253bbd339),
    c64e(0x459fc3c1e0298ba0),
    c64e(0xe5c905fdf7ae090f),
    c64e(0x947034124290f134),
    c64e(0xa271b701e344ed95),
    c64e(0xe93b8e364f2f984a),
    c64e(0x88401d63a06cf615),
    c64e(0x47c1444b8752afff),
    c64e(0x7ebb4af1e20ac630),
    c64e(0x4670b6c5cc6e8ce6),
    c64e(0xa4d5a456bd4fca00),
    c64e(0xda9d844bc83e18ae),
    c64e(0x7357ce453064d1ad),
    c64e(0xe8a6ce68145c2567),
    c64e(0xa3da8cf2cb0ee116),
    c64e(0x33e906589a94999a),
    c64e(0x1f60b220c26f847b),
    c64e(0xd1ceac7fa0d18518),
    c64e(0x32595ba18ddd19d3),
    c64e(0x509a1cc0aaa5b446),
    c64e(0x9f3d6367e4046bba),
    c64e(0xf6ca19ab0b56ee7e),
    c64e(0x1fb179eaa9282174),
    c64e(0xe9bdf7353b3651ee),
    c64e(0x1d57ac5a7550d376),
    c64e(0x3a46c2fea37d7001),
    c64e(0xf735c1af98a4d842),
    c64e(0x78edec209e6b6779),
    c64e(0x41836315ea3adba8),
    c64e(0xfac33b4d32832c83),
    c64e(0xa7403b1f1c2747f3),
    c64e(0x5940f034b72d769a),
    c64e(0xe73e4e6cd2214ffd),
    c64e(0xb8fd8d39dc5759ef),
    c64e(0x8d9b0c492b49ebda),
    c64e(0x5ba2d74968f3700d),
    c64e(0x7d3baed07a8d5584),
    c64e(0xf5a5e9f0e4f88e65),
    c64e(0xa0b8a2f436103b53),
    c64e(0x0ca8079e753eec5a),
    c64e(0x9168949256e8884f),
    c64e(0x5bb05c55f8babc4c),
    c64e(0xe3bb3b99f387947b),
    c64e(0x75daf4d6726b1c5d),
    c64e(0x64aeac28dc34b36d),
    c64e(0x6c34a550b828db71),
    c64e(0xf861e2f2108d512a),
    c64e(0xe3db643359dd75fc),
    c64e(0x1cacbcf143ce3fa2),
    c64e(0x67bbd13c02e843b0),
    c64e(0x330a5bca8829a175),
    c64e(0x7f34194db416535c),
    c64e(0x923b94c30e794d1e),
    c64e(0x797475d7b6eeaf3f),
    c64e(0xeaa8d4f7be1a3921),
    c64e(0x5cf47e094c232751),
    c64e(0x26a32453ba323cd2),
    c64e(0x44a3174a6da6d5ad),
    c64e(0xb51d3ea6aff2c908),
    c64e(0x83593d98916b3c56),
    c64e(0x4cf87ca17286604d),
    c64e(0x46e23ecc086ec7f6),
    c64e(0x2f9833b3b1bc765e),
    c64e(0x2bd666a5efc4e62a),
    c64e(0x06f4b6e8bec1d436),
    c64e(0x74ee8215bcef2163),
    c64e(0xfdc14e0df453c969),
    c64e(0xa77d5ac406585826),
    c64e(0x7ec1141606e0fa16),
    c64e(0x7e90af3d28639d3f),
    c64e(0xd2c9f2e3009bd20c),
    c64e(0x5faace30b7d40c30),
    c64e(0x742a5116f2e03298),
    c64e(0x0deb30d8e3cef89a),
    c64e(0x4bc59e7bb5f17992),
    c64e(0xff51e66e048668d3),
    c64e(0x9b234d57e6966731),
    c64e(0xcce6a6f3170a7505),
    c64e(0xb17681d913326cce),
    c64e(0x3c175284f805a262),
    c64e(0xf42bcbb378471547),
    c64e(0xff46548223936a48),
    c64e(0x38df58074e5e6565),
    c64e(0xf2fc7c89fc86508e),
    c64e(0x31702e44d00bca86),
    c64e(0xf04009a23078474e),
    c64e(0x65a0ee39d1f73883),
    c64e(0xf75ee937e42c3abd),
    c64e(0x2197b2260113f86f),
    c64e(0xa344edd1ef9fdee7),
    c64e(0x8ba0df15762592d9),
    c64e(0x3c85f7f612dc42be),
    c64e(0xd8a7ec7cab27b07e),
    c64e(0x538d7ddaaa3ea8de),
    c64e(0xaa25ce93bd0269d8),
    c64e(0x5af643fd1a7308f9),
    c64e(0xc05fefda174a19a5),
    c64e(0x974d66334cfd216a),
    c64e(0x35b49831db411570),
    c64e(0xea1e0fbbedcd549b),
    c64e(0x9ad063a151974072),
    c64e(0xf6759dbf91476fe2),
];

/// sphlib `JH_IV512`, byte-swapped like the C table.
const JH_IV512: [u64; 16] = [
    c64e(0x6fd14b963e00aa17),
    c64e(0x636a2e057a15d543),
    c64e(0x8a225e8d0c97ef0b),
    c64e(0xe9341259f2b3c361),
    c64e(0x891da0c1536f801e),
    c64e(0x2aa9056bea2b6d80),
    c64e(0x588eccdb2075baa6),
    c64e(0xa90f3a76baf83bf7),
    c64e(0x0169e60541e34a69),
    c64e(0x46b58a8e2e6fe65a),
    c64e(0x1047a7d0c1843c24),
    c64e(0x3b6e71b12d5ac199),
    c64e(0xcf57f6ec9db1f856),
    c64e(0xa706887c5716b156),
    c64e(0xe3c2fcdfe68517fb),
    c64e(0x545a4678cc8cdd4b),
];

macro_rules! sb {
    ($x0:ident, $x1:ident, $x2:ident, $x3:ident, $c:expr) => {{
        $x3 = !$x3;
        $x0 ^= $c & !$x2;
        let tmp: u64 = $c ^ ($x0 & $x1);
        $x0 ^= $x2 & $x3;
        $x3 ^= !$x1 & $x2;
        $x1 ^= $x0 & $x2;
        $x2 ^= $x0 & !$x3;
        $x0 ^= $x1 | $x3;
        $x3 ^= $x1 & $x2;
        $x1 ^= tmp & $x0;
        $x2 ^= tmp;
    }};
}

macro_rules! lb {
    ($x0:ident, $x1:ident, $x2:ident, $x3:ident, $x4:ident, $x5:ident, $x6:ident, $x7:ident) => {{
        $x4 ^= $x1;
        $x5 ^= $x2;
        $x6 ^= $x3 ^ $x0;
        $x7 ^= $x0;
        $x0 ^= $x5;
        $x1 ^= $x6;
        $x2 ^= $x7 ^ $x4;
        $x3 ^= $x4;
    }};
}

/// `Wz`: the bit-swapping linear step, applied to both words of an h pair.
macro_rules! wz {
    ($xh:ident, $xl:ident, $c:expr, $n:literal) => {{
        let t = ($xh & $c) << $n;
        $xh = (($xh >> $n) & $c) | t;
        let t = ($xl & $c) << $n;
        $xl = (($xl >> $n) & $c) | t;
    }};
}

macro_rules! w0 {
    ($xh:ident, $xl:ident) => {
        wz!($xh, $xl, 0x5555555555555555, 1)
    };
}
macro_rules! w1 {
    ($xh:ident, $xl:ident) => {
        wz!($xh, $xl, 0x3333333333333333, 2)
    };
}
macro_rules! w2 {
    ($xh:ident, $xl:ident) => {
        wz!($xh, $xl, 0x0F0F0F0F0F0F0F0F, 4)
    };
}
macro_rules! w3 {
    ($xh:ident, $xl:ident) => {
        wz!($xh, $xl, 0x00FF00FF00FF00FF, 8)
    };
}
macro_rules! w4 {
    ($xh:ident, $xl:ident) => {
        wz!($xh, $xl, 0x0000FFFF0000FFFF, 16)
    };
}
macro_rules! w5 {
    ($xh:ident, $xl:ident) => {
        wz!($xh, $xl, 0x00000000FFFFFFFF, 32)
    };
}
macro_rules! w6 {
    ($xh:ident, $xl:ident) => {{
        let t = $xh;
        $xh = $xl;
        $xl = t;
    }};
}

/// One `SLu(r, ro)` step: S-box on (h0,h2,h4,h6) with the even constants and
/// on (h1,h3,h5,h7) with the odd ones, then the linear L, then the `W{ro}`
/// bit swap on h1/h3/h5/h7. `C[($r << 2) + i]`: even-hi/even-lo/odd-hi/odd-lo.
macro_rules! slu {
    ($r:literal, $w:ident, $h0h:ident, $h0l:ident, $h1h:ident, $h1l:ident,
     $h2h:ident, $h2l:ident, $h3h:ident, $h3l:ident, $h4h:ident, $h4l:ident,
     $h5h:ident, $h5l:ident, $h6h:ident, $h6l:ident, $h7h:ident, $h7l:ident) => {{
        sb!($h0h, $h2h, $h4h, $h6h, C[($r << 2) + 0]);
        sb!($h0l, $h2l, $h4l, $h6l, C[($r << 2) + 1]);
        sb!($h1h, $h3h, $h5h, $h7h, C[($r << 2) + 2]);
        sb!($h1l, $h3l, $h5l, $h7l, C[($r << 2) + 3]);
        lb!($h0h, $h2h, $h4h, $h6h, $h1h, $h3h, $h5h, $h7h);
        lb!($h0l, $h2l, $h4l, $h6l, $h1l, $h3l, $h5l, $h7l);
        $w!($h1h, $h1l);
        $w!($h3h, $h3l);
        $w!($h5h, $h5l);
        $w!($h7h, $h7l);
    }};
}

/// The fully unrolled `E8` (the `SPH_JH_64` non-small branch: 42 rounds,
/// `W{r % 7}` bit swap on round r).
macro_rules! e8 {
    ($h0h:ident, $h0l:ident, $h1h:ident, $h1l:ident, $h2h:ident, $h2l:ident,
     $h3h:ident, $h3l:ident, $h4h:ident, $h4l:ident, $h5h:ident, $h5l:ident,
     $h6h:ident, $h6l:ident, $h7h:ident, $h7l:ident) => {{
        slu!(
            0, w0, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            1, w1, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            2, w2, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            3, w3, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            4, w4, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            5, w5, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            6, w6, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            7, w0, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            8, w1, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            9, w2, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            10, w3, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            11, w4, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            12, w5, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            13, w6, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            14, w0, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            15, w1, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            16, w2, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            17, w3, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            18, w4, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            19, w5, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            20, w6, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            21, w0, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            22, w1, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            23, w2, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            24, w3, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            25, w4, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            26, w5, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            27, w6, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            28, w0, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            29, w1, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            30, w2, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            31, w3, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            32, w4, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            33, w5, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            34, w6, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            35, w0, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            36, w1, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            37, w2, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            38, w3, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            39, w4, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            40, w5, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
        slu!(
            41, w6, $h0h, $h0l, $h1h, $h1l, $h2h, $h2l, $h3h, $h3l, $h4h, $h4l, $h5h, $h5l, $h6h,
            $h6l, $h7h, $h7l
        );
    }};
}

/// One `jh_core` compression over a 64-byte block: `INPUT_BUF1` XORs the
/// block into h[0..8], the E8 permutation runs, `INPUT_BUF2` XORs it into
/// h[8..16].
fn compress(h: &mut [u64; 16], m: &[u64; 8]) {
    let m0h = m[0];
    let m0l = m[1];
    let m1h = m[2];
    let m1l = m[3];
    let m2h = m[4];
    let m2l = m[5];
    let m3h = m[6];
    let m3l = m[7];
    let mut h0h = h[0];
    let mut h0l = h[1];
    let mut h1h = h[2];
    let mut h1l = h[3];
    let mut h2h = h[4];
    let mut h2l = h[5];
    let mut h3h = h[6];
    let mut h3l = h[7];
    let mut h4h = h[8];
    let mut h4l = h[9];
    let mut h5h = h[10];
    let mut h5l = h[11];
    let mut h6h = h[12];
    let mut h6l = h[13];
    let mut h7h = h[14];
    let mut h7l = h[15];
    // INPUT_BUF1
    h0h ^= m0h;
    h0l ^= m0l;
    h1h ^= m1h;
    h1l ^= m1l;
    h2h ^= m2h;
    h2l ^= m2l;
    h3h ^= m3h;
    h3l ^= m3l;
    e8!(h0h, h0l, h1h, h1l, h2h, h2l, h3h, h3l, h4h, h4l, h5h, h5l, h6h, h6l, h7h, h7l);
    // INPUT_BUF2
    h4h ^= m0h;
    h4l ^= m0l;
    h5h ^= m1h;
    h5l ^= m1l;
    h6h ^= m2h;
    h6l ^= m2l;
    h7h ^= m3h;
    h7l ^= m3l;
    h[0] = h0h;
    h[1] = h0l;
    h[2] = h1h;
    h[3] = h1l;
    h[4] = h2h;
    h[5] = h2l;
    h[6] = h3h;
    h[7] = h3l;
    h[8] = h4h;
    h[9] = h4l;
    h[10] = h5h;
    h[11] = h5l;
    h[12] = h6h;
    h[13] = h6l;
    h[14] = h7h;
    h[15] = h7l;
}

/// Full 64-byte JH-512 digest of the 32-byte message.
pub fn hash64(msg: &[u8; 32]) -> [u8; 64] {
    let mut h = JH_IV512;
    // `jh_core` buffered the 32 bytes; `jh_close` builds the 96-byte trailer
    // 0x80 || 79 zeros || BE64(0) || BE64(256) and feeds it to `jh_core`,
    // which compresses the two resulting 64-byte blocks:
    // first msg || 0x80 || zeros,
    let mut m = [0u64; 8];
    m[0] = u64::from_le_bytes(msg[0..8].try_into().unwrap());
    m[1] = u64::from_le_bytes(msg[8..16].try_into().unwrap());
    m[2] = u64::from_le_bytes(msg[16..24].try_into().unwrap());
    m[3] = u64::from_le_bytes(msg[24..32].try_into().unwrap());
    m[4] = 0x80;
    compress(&mut h, &m);
    // then zeros up to the 128-bit big-endian bit length 256 (BE(256)
    // re-read as little-endian words).
    m = [0u64; 8];
    m[7] = u64::from_le_bytes([0, 0, 0, 0, 0, 0, 1, 0]);
    compress(&mut h, &m);
    // digest = H.wide[8..16], little-endian (enc64e = sph_enc64le)
    let mut out = [0u8; 64];
    for i in 0..8 {
        out[i * 8..i * 8 + 8].copy_from_slice(&h[8 + i].to_le_bytes());
    }
    out
}
