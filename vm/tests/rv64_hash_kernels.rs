//! P3-KERNELS (A-25 milestone M2): the hash kernels promoted from the doc 74 /
//! doc 75 scratch sketches into production tests. This is the first
//! end-to-end exercise of the M1 compo-list v2 shapes through the REAL
//! rv64 lowering: whole kernels (not shape snippets) built in all three
//! modes, run on the rv64 engine, and knocked four ways against the
//! consensus interpreter on every input.
//!
//! Sections (one per sub-milestone):
//! * M2-a — sha256 single-block kernel (input = one already-padded 64B
//!   block, the doc 74 E10 contract): known-answer against a hand-written
//!   Rust reference (no external crate, the doc 75 k0 precedent) +
//!   interpreter×3-mode four-way agreement + image-budget measurement
//!   (code / pool / insts / static+dynamic cycles / packed container vs
//!   the 16383B `calc_image_max` admission, cap.rs:122) + determinism.
//! * M2-b — keccak scalar kernel + the iota round-constant LFSR migrated
//!   back to an M1 rodata table (doc 75 E4 measured -224B of code on the
//!   pre-M1 tree; both sides re-measured here on the M1 baseline).
//! * M2-c — multi-block sha256: outer block loop + `clear(w)` per-block
//!   rebuild + a..h cross-block scalar carry + IN-KERNEL padding
//!   (0x80, zero fill, 64-bit BE length suffix) over the raw message.
//! * M2-d — per-region size attribution and the optimization-balance
//!   ruling (recorded in doc 93).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{frontend, Interp, Rv64};
use vm::calc::CalcRunLimits;
use vm::ir::rv64::{lower_body_mode_arena, ARENA_SIZE_MAX, Rv64Mode};

/* ================================================================== */
/* Shared helpers.                                                     */
/* ================================================================== */

/// Deterministic xorshift64* RNG (no external crate; fixed seeds).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn byte_vec(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next_u64() >> 33) as u8).collect()
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Runnable-build envelope for the hash kernels: 64B output channel (32B
/// digests), the standard 4800B input channel and the 0x7000 arena — the
/// c3lib bigout precedent (the full 0x9D30 arena + input 4800 + output 64
/// needs 65 KiB of guest mem and overflows the 64 KiB window; the encoded
/// container does NOT grow with the arena, only the RW memsz does).
const RUN_ARENA: u64 = 0x7000;

/// Four-way agreement on EVERY input with the wide (64B) output envelope
/// the hash kernels need (32B digests): interpreter == all three rv64
/// modes, both Ok with identical bytes or Err with the identical code.
fn agree4_hash(body: &str, inputs: &[Vec<u8>]) {
    let rv = Rv64::try_build_out(body, 64, RUN_ARENA)
        .unwrap_or_else(|e| panic!("rv64 must build:\n{body}\n{e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build:\n{body}\n{e}"));
    for input in inputs {
        let want = it.run(input).map_err(|f| f.code as u64);
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got, want,
                "mode {mode:?} diverged on input of {} bytes, body:\n{body}",
                input.len()
            );
        }
    }
}

/// One mode's measured image: code mirror, rodata pool, instruction count,
/// static cycle estimate, locals, and the fully packed container size
/// (header + segment table + code + rodata + arena headers — the quantity
/// admitted against `SpaceCap::calc_image_max` = 16383, cap.rs:122 +
/// calc/mod.rs reject_image_len).
struct ModeImage {
    code: usize,
    pool: usize,
    insts: usize,
    cycles: u64,
    locals: usize,
    container: usize,
}

fn measure_modes(body: &str) -> Vec<(&'static str, ModeImage)> {
    let (ir, _) = frontend(body).unwrap_or_else(|e| panic!("frontend: {e}"));
    let cap = vm::calc::SpaceCap::new(0);
    let mut out = Vec::new();
    for &mode in c3lib::MODES.iter() {
        let lo = lower_body_mode_arena(&ir, true, mode, RUN_ARENA)
            .unwrap_or_else(|e| panic!("lower {mode:?}: {e}"));
        let profile = match mode {
            Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
            _ => vm::calc::container::CALC_PROFILE_RV64IMC,
        };
        let container = lo
            .pack_segments_profile(&cap, 64, 4800, 64, profile)
            .unwrap_or_else(|e| panic!("pack {mode:?}: {e}"))
            .encode()
            .unwrap_or_else(|e| panic!("encode {mode:?}: {e}"));
        out.push((
            match mode {
                Rv64Mode::Im => "Im",
                Rv64Mode::Imc => "Imc",
                Rv64Mode::ImcRemap => "ImcRemap",
            },
            ModeImage {
                code: lo.code.len(),
                pool: lo.pool.len(),
                insts: lo.stats.insts,
                cycles: lo.stats.cycles,
                locals: lo.stats.locals,
                container: container.len(),
            },
        ));
    }
    out
}

/// Dynamic cycles actually spent by the rv64 engine per mode on one input
/// (the c3lib Rv64 wrapper drops CalcExec::cycles_used; reach for it here).
fn dynamic_cycles(body: &str, input: &[u8]) -> Vec<(&'static str, u64)> {
    let rv = Rv64::try_build_out(body, 64, RUN_ARENA)
        .unwrap_or_else(|e| panic!("rv64 must build: {e}"));
    rv.engines
        .iter()
        .map(|&(mode, ref fno)| {
            let ex = fno.run(
                input,
                CalcRunLimits { cycles: 10_000_000, resource: -1, deadline: None },
            );
            let name = match mode {
                Rv64Mode::Im => "Im",
                Rv64Mode::Imc => "Imc",
                Rv64Mode::ImcRemap => "ImcRemap",
            };
            assert!(ex.result.is_ok(), "engine {name} must run the kernel");
            (name, ex.cycles_used)
        })
        .collect()
}

/// Compilation determinism: two independent builds must pack
/// byte-identical containers in every mode (red-line check, doc 92 E1 l40).
fn assert_deterministic(body: &str) {
    let a = Rv64::try_build_out(body, 64, RUN_ARENA).expect("build 1");
    let b = Rv64::try_build_out(body, 64, RUN_ARENA).expect("build 2");
    assert_eq!(a.containers.len(), b.containers.len());
    for ((ma, ca), (mb, cb)) in a.containers.iter().zip(b.containers.iter()) {
        assert_eq!(ma, mb);
        assert_eq!(ca, cb, "container for {ma:?} not byte-identical across builds");
    }
}

/* ================================================================== */
/* Hand-written Rust sha256 reference (no external crate).              */
/* ================================================================== */

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn ref_sha256(msg: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut data = msg.to_vec();
    let bit_len = (msg.len() as u64).wrapping_mul(8);
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in data.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let mj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(mj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/* ================================================================== */
/* M2-a: the sha256 single-block kernel source (doc 74 E10, promoted   */
/* from the rv74 scratch generator into tracked code).                 */
/* ================================================================== */

/// sha256 padded single block for a message ≤ 55 bytes.
fn pad_one_block(msg: &[u8]) -> Vec<u8> {
    assert!(msg.len() <= 55);
    let mut b = msg.to_vec();
    b.push(0x80);
    while b.len() < 56 {
        b.push(0);
    }
    b.extend_from_slice(&((msg.len() as u64) * 8).to_be_bytes());
    b
}

/// K constant table source (u64-regularized literals; the all-literal
/// PACKLIST the M1 plan lowers to a Rodata u64 table).
fn sha256_k_list_src() -> String {
    let items: Vec<String> = SHA256_K.iter().map(|&v| format!("{v}u64")).collect();
    format!("var k = [{}]\n", items.join(", "))
}

/// The sha256 single-block compression kernel: input = one already-padded
/// 64B block; output = the 32B digest. Shape surface = the M1 v2 set:
/// `var w = []` (NEWLIST→Growable) + append accumulation in the unpack and
/// schedule loops, `var k = [64 literals]` (PACKLIST→Rodata), ITEMGET
/// variable-index reads, everything else scalars.
fn sha256_single_block_src() -> String {
    let mut s = String::new();
    // C1: input unpack tax — 4×byte(data,·) per word, one loop emits 16 words
    s.push_str("var w = []\nvar o = 0u64\nwhile o < 64u64 {\n");
    s.push_str("  append(w, byte(data, o) * 16777216u64 + byte(data, o + 1u64) * 65536u64 + byte(data, o + 2u64) * 256u64 + byte(data, o + 3u64))\n");
    s.push_str("  o += 4u64\n}\n");
    // C2: K constant table (all-literal PACKLIST → rodata)
    s.push_str(&sha256_k_list_src());
    s.push_str("var m = 4294967295u64\n");
    s.push_str("var a = 1779033703u64\nvar b = 3144134277u64\nvar c = 1013904242u64\nvar d = 2773480762u64\n");
    s.push_str("var e = 1359893119u64\nvar f = 2600822924u64\nvar g = 528734635u64\nvar h = 1541459225u64\n");
    // C3a: message schedule — append accumulation, sliding-window reads
    s.push_str("var j = 16u64\nwhile j < 64u64 {\n");
    s.push_str("  var w15 = w[j - 15u64]\n  var w2 = w[j - 2u64]\n");
    s.push_str("  var s0 = (((w15 >> 7u64) | (w15 << 25u64)) & m) ^ (((w15 >> 18u64) | (w15 << 14u64)) & m) ^ (w15 >> 3u64)\n");
    s.push_str("  var s1 = (((w2 >> 17u64) | (w2 << 15u64)) & m) ^ (((w2 >> 19u64) | (w2 << 13u64)) & m) ^ (w2 >> 10u64)\n");
    s.push_str("  append(w, (w[j - 16u64] + s0 + w[j - 7u64] + s1) & m)\n");
    s.push_str("  j += 1u64\n}\n");
    // C3b: compression — a..h scalars, reads k[i] / w[i]
    s.push_str("var i = 0u64\nwhile i < 64u64 {\n");
    s.push_str("  var r1 = (((e >> 6u64) | (e << 26u64)) & m)\n");
    s.push_str("  var r2 = (((e >> 11u64) | (e << 21u64)) & m)\n");
    s.push_str("  var r3 = (((e >> 25u64) | (e << 7u64)) & m)\n");
    s.push_str("  var S1 = (r1 ^ r2 ^ r3)\n");
    s.push_str("  var ch = ((e & f) ^ ((m ^ e) & g))\n");
    s.push_str("  var t1 = (h + S1 + ch + k[i] + w[i])\n");
    s.push_str("  var r4 = (((a >> 2u64) | (a << 30u64)) & m)\n");
    s.push_str("  var r5 = (((a >> 13u64) | (a << 19u64)) & m)\n");
    s.push_str("  var r6 = (((a >> 22u64) | (a << 10u64)) & m)\n");
    s.push_str("  var S0 = (r4 ^ r5 ^ r6)\n");
    s.push_str("  var mj = ((a & b) ^ (a & c) ^ (b & c))\n");
    s.push_str("  var t2 = (S0 + mj)\n");
    s.push_str("  h = g\n  g = f\n  f = e\n");
    s.push_str("  e = (d + t1) & m\n");
    s.push_str("  d = c\n  c = b\n  b = a\n");
    s.push_str("  a = (t1 + t2) & m\n");
    s.push_str("  i += 1u64\n}\n");
    // final state + IV (the digest adds the initial vector — single-block
    // kernels must carry it themselves; pinned during the doc 74 sketch)
    s.push_str("a = (a + 1779033703u64) & m\nb = (b + 3144134277u64) & m\nc = (c + 1013904242u64) & m\nd = (d + 2773480762u64) & m\n");
    s.push_str("e = (e + 1359893119u64) & m\nf = (f + 2600822924u64) & m\ng = (g + 528734635u64) & m\nh = (h + 1541459225u64) & m\n");
    // output: 8 words BE, packed as 4×u64 via CAT
    s.push_str("var o1 = (a * 4294967296u64) + b\nvar o2 = (c * 4294967296u64) + d\n");
    s.push_str("var o3 = (e * 4294967296u64) + f\nvar o4 = (g * 4294967296u64) + h\n");
    s.push_str("return ((o1 as bytes) ++ (o2 as bytes)) ++ ((o3 as bytes) ++ (o4 as bytes))\n");
    s
}

/* ================================================================== */
/* M2-a tests.                                                         */
/* ================================================================== */

/// The hand-written reference against the public NIST FIPS 180-2 vectors
/// (independent of this repo's VM code; anchors the known-answer tests).
#[test]
fn h0_reference_sha256_anchors() {
    let cases: &[(&[u8], &str)] = &[
        (b"", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (b"abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            &[b'a'; 55][..],
            "9f4390f8d30c2dd92ec9f095b65e2b9ae9b0a925a5258e241c9f1e910f734318",
        ),
        (
            &[b'a'; 63][..],
            // system-sha256sum-verified (the ×63 anchor was mis-copied from
            // memory on the first pass; every anchor here is now external)
            "7d3e74a05d7db15bce4ad9ec0658ea98e3f06eeecf16b4c6fff2da457ddc2f34",
        ),
        (
            &[b'a'; 56][..],
            // cross-block boundary anchor (two padded blocks) for M2-c
            "b35439a4ac6f0948b6d6f9e3c6af0f5f590ce20f1bde7090ef7970686ec6738a",
        ),
    ];
    for (msg, want) in cases {
        let got = hex(&ref_sha256(msg));
        assert_eq!(&got, want, "reference sha256 diverged for len {}", msg.len());
    }
}

/// Known-answer: the interpreter running the kernel == the Rust reference.
/// NIST vectors + a random-message sweep across the single-block domain
/// (0 / 1 / 54 / 55 boundaries included; 56 needs two blocks — M2-c).
#[test]
fn h1_sha256_single_block_known_answer() {
    let src = sha256_single_block_src();
    let it = Interp::try_build(&src).unwrap_or_else(|e| panic!("interp build: {e}"));
    let mut msgs: Vec<Vec<u8>> = vec![b"".to_vec(), b"abc".to_vec(), vec![b'a'; 55]];
    let mut rng = Rng::new(0x5A25_0001);
    for &n in &[0usize, 1, 2, 7, 31, 54, 55] {
        msgs.push(rng.byte_vec(n));
    }
    for msg in &msgs {
        let block = pad_one_block(msg);
        let got = it
            .run(&block)
            .unwrap_or_else(|f| panic!("kernel run, msg len {}: {}", msg.len(), f.msg));
        let want = ref_sha256(msg);
        assert_eq!(
            hex(&got),
            hex(&want),
            "sha256 single-block kernel diverged for msg len {}",
            msg.len()
        );
    }
    println!("h1: {} known-answer messages OK (NIST 3 + random sweep)", msgs.len());
}

/// Four-way agreement: interpreter == Im == Imc == ImcRemap on every input
/// (properly padded blocks AND fully random 64B blocks — the kernel is a
/// pure function of its 64 input bytes).
#[test]
fn h2_sha256_single_block_agree4() {
    let src = sha256_single_block_src();
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for msg in [b"".to_vec(), b"abc".to_vec(), vec![b'a'; 55]] as [Vec<u8>; 3] {
        inputs.push(pad_one_block(&msg));
    }
    let mut rng = Rng::new(0x5A25_0002);
    for &n in &[1usize, 2, 31, 54, 55] {
        inputs.push(pad_one_block(&rng.byte_vec(n)));
    }
    for _ in 0..8 {
        inputs.push(rng.byte_vec(64));
    }
    agree4_hash(&src, &inputs);
    println!("h2: {} inputs × 3 rv64 modes all agree with the interpreter", inputs.len());
}

/// Image budget + cycles + determinism (numbers pinned into doc 93).
#[test]
fn h3_sha256_single_block_budget() {
    let src = sha256_single_block_src();
    for (name, img) in measure_modes(&src) {
        println!(
            "sha256-single {name}: code={} pool={} insts={} static-cycles={} locals={} container={} (vs 16383, headroom x{:.1})",
            img.code,
            img.pool,
            img.insts,
            img.cycles,
            img.locals,
            img.container,
            16383.0 / img.container as f64
        );
        assert!(img.container < 16383, "{name} container exceeds calc_image_max");
        // M2-d: the K table (all values < 2^32) packs u32 → 256B of pool
        assert!(img.pool >= 256, "K table must land in the rodata pool (64×u32)");
    }
    // dynamic cycles on one padded block
    let block = pad_one_block(b"abc");
    for (name, cyc) in dynamic_cycles(&src, &block) {
        println!("sha256-single {name}: dynamic cycles = {cyc} (budget 10M)");
        assert!(cyc < 10_000_000);
    }
    assert_deterministic(&src);
}

/* ================================================================== */
/* M2-b: hand-written Rust keccak-f[1600] reference (no external crate; */
/* promoted from the doc 75 k0 scratch). Anchors independently        */
/* cross-checked against pycryptodome on this machine (see doc 93).    */
/* ================================================================== */

fn keccak_rho_table() -> [u32; 25] {
    // FIPS 202 rho offsets derived from the (t+1)(t+2)/2 walk on the pi
    // successor (x,y) -> (y, (2x+3y)%5).
    let mut rho = [0u32; 25];
    let (mut x, mut y) = (1usize, 0usize);
    for t in 0..24 {
        rho[x + 5 * y] = (((t + 1) * (t + 2) / 2) % 64) as u32;
        let (nx, ny) = (y, (2 * x + 3 * y) % 5);
        x = nx;
        y = ny;
    }
    rho
}

fn keccak_rc_table() -> [u64; 24] {
    // FIPS 202 iota round constants from the 8-bit LFSR, feedback 0x71.
    let mut rc = [0u64; 24];
    let mut r: u64 = 1;
    for item in rc.iter_mut() {
        *item = 0;
        for t in 0..7u32 {
            *item |= (r & 1) << ((1u64 << t) - 1);
            r = ((r << 1) ^ ((r >> 7) * 0x71)) & 0xFF;
        }
    }
    rc
}

fn ref_keccak_f(st: &mut [u64; 25]) {
    let rc = keccak_rc_table();
    let rho = keccak_rho_table();
    for round in 0..24 {
        // theta
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = st[x] ^ st[x + 5] ^ st[x + 10] ^ st[x + 15] ^ st[x + 20];
        }
        let mut d = [0u64; 5];
        for x in 0..5 {
            d[x] = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
        }
        for i in 0..25 {
            st[i] ^= d[i % 5];
        }
        // rho + pi
        let mut b = [0u64; 25];
        for x in 0..5 {
            for y in 0..5 {
                let l = x + 5 * y;
                b[y + 5 * ((2 * x + 3 * y) % 5)] = st[l].rotate_left(rho[l]);
            }
        }
        // chi
        for y in 0..5 {
            for x in 0..5 {
                st[x + 5 * y] =
                    b[x + 5 * y] ^ ((!b[((x + 1) % 5) + 5 * y]) & b[((x + 2) % 5) + 5 * y]);
            }
        }
        // iota
        st[0] ^= rc[round];
    }
}

fn ref_keccak_absorb(st: &mut [u64; 25], block: &[u8]) {
    for i in 0..17 {
        let mut lane = 0u64;
        for j in 0..8 {
            lane |= (block[i * 8 + j] as u64) << (8 * j);
        }
        st[i] ^= lane;
    }
    ref_keccak_f(st);
}

/// keccak256 (original Keccak padding 0x01..0x80, rate 136).
fn ref_keccak256(msg: &[u8]) -> [u8; 32] {
    const RATE: usize = 136;
    let mut st = [0u64; 25];
    let mut off = 0usize;
    loop {
        let take = (msg.len() - off).min(RATE);
        let mut block = [0u8; RATE];
        block[..take].copy_from_slice(&msg[off..off + take]);
        if take == RATE && off + RATE < msg.len() {
            ref_keccak_absorb(&mut st, &block);
            off += RATE;
            continue;
        }
        block[take] = 0x01;
        block[RATE - 1] |= 0x80;
        ref_keccak_absorb(&mut st, &block);
        break;
    }
    let mut out = [0u8; 32];
    for i in 0..4 {
        out[i * 8..i * 8 + 8].copy_from_slice(&st[i].to_le_bytes());
    }
    out
}

/// keccak256 single-block contract: the already-padded 136B block (raw
/// message ≤ 135B).
fn pad_keccak_block(msg: &[u8]) -> Vec<u8> {
    assert!(msg.len() <= 135);
    let mut b = msg.to_vec();
    b.push(0x01);
    while b.len() < 136 {
        b.push(0);
    }
    let n = b.len();
    b[n - 1] |= 0x80;
    b
}

/// Anchors (doc 75 E3 ruling: the dispatch brief's keccak256("") tail
/// …85a456 was a typo; consensus is …85a470). All four independently
/// cross-checked against pycryptodome `Crypto.Hash.keccak` on this host.
const KECCAK_ANCHOR_EMPTY: &str = "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";
const KECCAK_ANCHOR_ABC: &str = "4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45";
const KECCAK_ANCHOR_RANGE135: &str = "cbdfd9dee5faad3818d6b06f95a219fd290b0e1706f6a82e5a595b9ce9faca62";
const KECCAK_ANCHOR_X134: &str = "245295a330b4906deddd9de5bade91af5e9bae48cf0432614f7c67865139c281";

/* ================================================================== */
/* M2-b: the keccak scalar kernel source (doc 75 E2, promoted from the */
/* rv75 scratch generator; `rc_from_table` selects the M2-b iota       */
/* round-constant form).                                              */
/* ================================================================== */

/// keccak-f[1600] scalar-decomposition kernel: 25 lanes = 25 scalar locals
/// s0..s24; per round = theta (c/d scalar temporaries) + theta apply +
/// rho-pi shadow scalars b0..b24 (rotation constants folded into the
/// statements) + chi reading b writing s + iota. `rc_from_table = false`:
/// the FIPS 202 LFSR derives the round constant in-kernel (the doc 75
/// baseline). `rc_from_table = true`: the M2-b migration — a 24-entry
/// all-literal PACKLIST the M1 plan lowers to a Rodata u64 table, read by
/// variable index `rc_t[r]`. No ADD anywhere (no checked-add divergence
/// surface); every shift amount is a compile-time constant in [1,63] (the
/// rho=0 lane 0 skips the rotate, avoiding the >>64 interpreter-vs-&63
/// divergence site); byte() results are `as u64`-widened before bitops.
fn keccak_scalar_src(rc_from_table: bool) -> String {
    let rho = keccak_rho_table();
    let mut s = String::new();
    // absorb: 17 lanes straight-line (lane byte order = LE)
    for k in 0..17usize {
        s.push_str(&format!("var s{} = byte(data, {}u64) as u64\n", k, 8 * k));
        for j in 1..8usize {
            s.push_str(&format!(
                "s{0} = s{0} | ((byte(data, {1}u64) as u64) << {2}u64)\n",
                k,
                8 * k + j,
                8 * j
            ));
        }
    }
    // lanes 17..24 stay zero through the single-block absorb (136B = 17 lanes)
    for k in 17..25usize {
        s.push_str(&format!("var s{0} = 0u64\n", k));
    }
    if rc_from_table {
        let rc = keccak_rc_table();
        let lits: Vec<String> = rc.iter().map(|&v| format!("{v}u64")).collect();
        s.push_str(&format!("var rc_t = [{}]\n", lits.join(", ")));
    }
    s.push_str("var m = 18446744073709551615u64\n");
    s.push_str("var r = 0u64\n");
    if !rc_from_table {
        s.push_str("var lr = 1u64\n");
    }
    s.push_str("while r < 24u64 {\n");
    if rc_from_table {
        s.push_str("  var rc = rc_t[r]\n");
    } else {
        // iota: RC per round from the LFSR (7 bits at positions
        // 0,1,3,7,15,31,63; one LFSR update per bit read)
        let pos = [0u64, 1, 3, 7, 15, 31, 63];
        s.push_str("  var rc = (lr & 1u64)\n");
        for i in 0..pos.len() {
            s.push_str("  lr = ((lr << 1u64) ^ ((lr >> 7u64) * 113u64)) & 255u64\n");
            if i + 1 < pos.len() {
                s.push_str(&format!("  rc = rc | ((lr & 1u64) << {}u64)\n", pos[i + 1]));
            }
        }
    }
    // theta C/D
    for x in 0..5usize {
        let l = |i: usize| format!("s{}", x + 5 * i);
        s.push_str(&format!(
            "  var c{0} = (({1} ^ {2}) ^ ({3} ^ ({4} ^ {5})))\n",
            x,
            l(0),
            l(1),
            l(2),
            l(3),
            l(4)
        ));
    }
    for x in 0..5usize {
        let cm = format!("c{}", (x + 4) % 5);
        let cp = format!("c{}", (x + 1) % 5);
        s.push_str(&format!(
            "  var d{0} = {1} ^ (({2} << 1u64) | ({2} >> 63u64))\n",
            x, cm, cp
        ));
    }
    // theta apply + rho + pi (lane 0: rho=0 → the theta value IS b0)
    let mut bstmts: Vec<(usize, String)> = Vec::new();
    for x in 0..5usize {
        for y in 0..5usize {
            let l = x + 5 * y;
            let t = y + 5 * ((2 * x + 3 * y) % 5);
            let rh = rho[l] as u64;
            if rh == 0 {
                bstmts.push((t, format!("var b{0} = (s{1} ^ d{2})", t, l, x)));
            } else {
                bstmts.push((
                    t,
                    format!("var b{0} = ((s{1} << {2}u64) | (s{1} >> {3}u64))", t, l, rh, 64 - rh),
                ));
            }
        }
    }
    for x in 0..5usize {
        for y in 0..5usize {
            let l = x + 5 * y;
            if rho[l] != 0 {
                s.push_str(&format!("  s{0} = s{0} ^ d{1}\n", l, x));
            }
        }
    }
    for (_, st) in &bstmts {
        s.push_str(&format!("  {}\n", st));
    }
    // chi (reads b writes s; lane 0 folds iota: s0 ^= rc)
    for x in 0..5usize {
        for y in 0..5usize {
            let l = x + 5 * y;
            let l1 = ((x + 1) % 5) + 5 * y;
            let l2 = ((x + 2) % 5) + 5 * y;
            if l == 0 {
                s.push_str("  s0 = (b0 ^ ((m ^ b1) & b2)) ^ rc\n");
            } else {
                s.push_str(&format!("  s{0} = b{0} ^ ((m ^ b{1}) & b{2})\n", l, l1, l2));
            }
        }
    }
    s.push_str("  r += 1u64\n}\n");
    // squeeze: 32B = lanes 0..3 LE bytes → byte-reversed into `as bytes` (BE)
    for k in 0..4usize {
        s.push_str(&format!(
            "var o{0} = ((s{0} & 255u64) << 56u64) | (((s{0} >> 8u64) & 255u64) << 48u64)\n",
            k
        ));
        s.push_str(&format!(
            "o{0} = o{0} | (((s{0} >> 16u64) & 255u64) << 40u64) | (((s{0} >> 24u64) & 255u64) << 32u64)\n",
            k
        ));
        s.push_str(&format!(
            "o{0} = o{0} | (((s{0} >> 32u64) & 255u64) << 24u64) | (((s{0} >> 40u64) & 255u64) << 16u64)\n",
            k
        ));
        s.push_str(&format!(
            "o{0} = o{0} | (((s{0} >> 48u64) & 255u64) << 8u64) | ((s{0} >> 56u64) & 255u64)\n",
            k
        ));
    }
    s.push_str("return ((o0 as bytes) ++ (o1 as bytes)) ++ ((o2 as bytes) ++ (o3 as bytes))\n");
    s
}

/* ================================================================== */
/* M2-b tests.                                                         */
/* ================================================================== */

#[test]
fn h4_keccak_reference_anchors() {
    let rc = keccak_rc_table();
    assert_eq!(rc[0], 1u64);
    assert_eq!(rc[1], 0x8082u64);
    assert_eq!(rc[2], 0x800000000000808au64);
    let range135: Vec<u8> = (0..135u8).collect();
    let cases: &[(&[u8], &str)] = &[
        (b"", KECCAK_ANCHOR_EMPTY),
        (b"abc", KECCAK_ANCHOR_ABC),
        (range135.as_slice(), KECCAK_ANCHOR_RANGE135),
        (&[b'x'; 134][..], KECCAK_ANCHOR_X134),
    ];
    for (msg, want) in cases {
        let got = hex(&ref_keccak256(msg));
        assert_eq!(&got, want, "reference keccak256 diverged for len {}", msg.len());
    }
}

/// Known-answer for BOTH iota forms (LFSR baseline and the M2-b rodata
/// migration): the interpreter running the kernel == the Rust reference.
#[test]
fn h5_keccak_scalar_known_answer() {
    for (name, rc_table) in [("LFSR", false), ("rodata-table", true)] {
        let src = keccak_scalar_src(rc_table);
        let it = Interp::try_build(&src).unwrap_or_else(|e| panic!("interp build ({name}): {e}"));
        let mut msgs: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"abc".to_vec(),
            (0..135u8).collect(),
            vec![b'x'; 134],
        ];
        let mut rng = Rng::new(0x5A25_0101);
        for &n in &[1usize, 2, 9, 63, 64, 65, 134, 135] {
            msgs.push(rng.byte_vec(n));
        }
        for msg in &msgs {
            let block = pad_keccak_block(msg);
            let got = it
                .run(&block)
                .unwrap_or_else(|f| panic!("kernel run ({name}), len {}: {}", msg.len(), f.msg));
            let want = ref_keccak256(msg);
            assert_eq!(
                hex(&got),
                hex(&want),
                "keccak scalar kernel ({name}) diverged for msg len {}",
                msg.len()
            );
        }
        println!("h5 [{name}]: {} known-answer messages OK", msgs.len());
    }
}

/// Four-way agreement for both iota forms: interpreter == Im == Imc ==
/// ImcRemap on every input (padded blocks AND fully random 136B blocks).
#[test]
fn h6_keccak_scalar_agree4() {
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    for msg in [b"".to_vec(), b"abc".to_vec(), (0..135u8).collect::<Vec<u8>>()] {
        inputs.push(pad_keccak_block(&msg));
    }
    let mut rng = Rng::new(0x5A25_0102);
    for &n in &[1usize, 64, 135] {
        inputs.push(pad_keccak_block(&rng.byte_vec(n)));
    }
    for _ in 0..6 {
        inputs.push(rng.byte_vec(136));
    }
    for (name, rc_table) in [("LFSR", false), ("rodata-table", true)] {
        agree4_hash(&keccak_scalar_src(rc_table), &inputs);
        println!("h6 [{name}]: {} inputs × 3 rv64 modes all agree", inputs.len());
    }
}

/// The M2-b size optimization: LFSR → rodata table. Doc 75 E4 measured
/// −224B of code for the round-loop region on the pre-M1 tree; both sides
/// re-measured here on the M1 baseline (whole kernel, all three modes,
/// code + pool + packed container). Known-answer green is asserted by h5.
#[test]
fn h7_keccak_rc_rodata_migration_measure() {
    let before = measure_modes(&keccak_scalar_src(false));
    let after = measure_modes(&keccak_scalar_src(true));
    for ((mb, ib), (ma, ia)) in before.iter().zip(after.iter()) {
        assert_eq!(mb, ma);
        println!(
            "keccak {mb}: code {} → {} ({:+}), pool {} → {} ({:+}), container {} → {} ({:+}), insts {} → {}, static-cycles {} → {}",
            ib.code, ia.code, ia.code as i64 - ib.code as i64,
            ib.pool, ia.pool, ia.pool as i64 - ib.pool as i64,
            ib.container, ia.container, ia.container as i64 - ib.container as i64,
            ib.insts, ia.insts, ib.cycles, ia.cycles
        );
        assert!(ia.container < 16383, "{mb} container exceeds calc_image_max");
    }
    // dynamic cycles: the LFSR spends 7 updates + 7 bit ors per round; the
    // table read is one guarded load
    let block = pad_keccak_block(b"abc");
    let cb = dynamic_cycles(&keccak_scalar_src(false), &block);
    let ca = dynamic_cycles(&keccak_scalar_src(true), &block);
    for ((mb, cb), (_, ca)) in cb.iter().zip(ca.iter()) {
        println!("keccak {mb}: dynamic cycles {cb} → {ca} ({:+})", *ca as i64 - *cb as i64);
    }
    assert_deterministic(&keccak_scalar_src(true));
}

/* ================================================================== */
/* M2-c: multi-block sha256 — raw message in, digest out. The outer     */
/* block loop rebuilds the w table via `clear(w)` each block (the doc   */
/* 75 E10 shape), a..h carry across blocks as scalars, and the padding  */
/* (0x80 marker, zero fill, 64-bit BE length suffix) is computed        */
/* IN-KERNEL from `size(data)`. General n-block while form.            */
/* ================================================================== */

/// One padded byte at global position `g`, as fitsh source fragments.
/// Pre-temps: `n` (message length), `tot` (padded total length, a
/// multiple of 64), `lb` (= n*8, loop-invariant), plus this byte's
/// `q{X} = tot - 1 - g{X}`. Value-position `if` chains are lazy in BOTH
/// engines (IRIFR — the pinned D9 family only diverges for eager CHOOSE),
/// so the faulting `byte(data, g)` arm is never evaluated at g ≥ n.
fn pad_byte_fragments(tag: usize, g: &str) -> (String, String) {
    let q = format!("q{tag}");
    let decls = format!("  var {q} = tot - 1u64 - {g}\n");
    let val = format!(
        "if ({g} < n) {{ byte(data, {g}) as u64 }} else {{ if ({g} == n) {{ 128u64 }} else {{ if ({q} < 8u64) {{ (lb >> ({q} * 8u64)) & 255u64 }} else {{ 0u64 }} }} }}"
    );
    (decls, val)
}

/// The multi-block sha256 kernel. Input = the RAW message (`data`);
/// output = the 32B digest. Works for any message the input channel can
/// carry (the tests exercise 0..200B; nothing in the kernel is
/// block-count-limited — `nblocks` is a runtime value).
fn sha256_multi_block_src() -> String {
    let mut s = String::new();
    s.push_str("var n = size(data) as u64\n");
    s.push_str("var tot = ((n + 72u64) / 64u64) * 64u64\n");
    s.push_str("var nblocks = tot / 64u64\n");
    s.push_str("var lb = n * 8u64\n");
    s.push_str(&sha256_k_list_src());
    s.push_str("var m = 4294967295u64\n");
    s.push_str("var a = 1779033703u64\nvar b = 3144134277u64\nvar c = 1013904242u64\nvar d = 2773480762u64\n");
    s.push_str("var e = 1359893119u64\nvar f = 2600822924u64\nvar g = 528734635u64\nvar h = 1541459225u64\n");
    // the w table is created ONCE (conditional list creation is outside the
    // M1 shape set) and rebuilt per block via clear + append
    s.push_str("var w = []\n");
    s.push_str("var blk = 0u64\n");
    s.push_str("while blk < nblocks {\n");
    // per-block Davies-Meyer feedforward: save the ENTRY state (block 0
    // enters with the IV constants; later blocks with the chained hash)
    s.push_str("  var ea = a\n  var eb = b\n  var ec = c\n  var ed = d\n  var ee = e\n  var ef = f\n  var eg = g\n  var eh = h\n");
    s.push_str("  clear(w)\n");
    // per-block absorb with in-kernel padding (4 padded bytes per word)
    s.push_str("  var o = 0u64\n");
    s.push_str("  while o < 64u64 {\n");
    s.push_str("    var g0 = blk * 64u64 + o\n");
    let (d0, v0) = pad_byte_fragments(0, "g0");
    s.push_str(&d0);
    s.push_str("    var p0 = ");
    s.push_str(&v0);
    s.push('\n');
    for t in 1..4usize {
        s.push_str(&format!("    var g{t} = g0 + {t}u64\n"));
        let (dt, vt) = pad_byte_fragments(t, &format!("g{t}"));
        s.push_str(&dt);
        s.push_str(&format!("    var p{t} = {vt}\n"));
    }
    s.push_str("    append(w, p0 * 16777216u64 + p1 * 65536u64 + p2 * 256u64 + p3)\n");
    s.push_str("    o += 4u64\n");
    s.push_str("  }\n");
    // per-block message schedule (same body as the single-block kernel)
    s.push_str("  var j = 16u64\n  while j < 64u64 {\n");
    s.push_str("    var w15 = w[j - 15u64]\n    var w2 = w[j - 2u64]\n");
    s.push_str("    var s0 = (((w15 >> 7u64) | (w15 << 25u64)) & m) ^ (((w15 >> 18u64) | (w15 << 14u64)) & m) ^ (w15 >> 3u64)\n");
    s.push_str("    var s1 = (((w2 >> 17u64) | (w2 << 15u64)) & m) ^ (((w2 >> 19u64) | (w2 << 13u64)) & m) ^ (w2 >> 10u64)\n");
    s.push_str("    append(w, (w[j - 16u64] + s0 + w[j - 7u64] + s1) & m)\n");
    s.push_str("    j += 1u64\n  }\n");
    // per-block compression; a..h carry across blocks
    s.push_str("  var i = 0u64\n  while i < 64u64 {\n");
    s.push_str("    var r1 = (((e >> 6u64) | (e << 26u64)) & m)\n");
    s.push_str("    var r2 = (((e >> 11u64) | (e << 21u64)) & m)\n");
    s.push_str("    var r3 = (((e >> 25u64) | (e << 7u64)) & m)\n");
    s.push_str("    var S1 = (r1 ^ r2 ^ r3)\n");
    s.push_str("    var ch = ((e & f) ^ ((m ^ e) & g))\n");
    s.push_str("    var t1 = (h + S1 + ch + k[i] + w[i])\n");
    s.push_str("    var r4 = (((a >> 2u64) | (a << 30u64)) & m)\n");
    s.push_str("    var r5 = (((a >> 13u64) | (a << 19u64)) & m)\n");
    s.push_str("    var r6 = (((a >> 22u64) | (a << 10u64)) & m)\n");
    s.push_str("    var S0 = (r4 ^ r5 ^ r6)\n");
    s.push_str("    var mj = ((a & b) ^ (a & c) ^ (b & c))\n");
    s.push_str("    var t2 = (S0 + mj)\n");
    s.push_str("    h = g\n    g = f\n    f = e\n");
    s.push_str("    e = (d + t1) & m\n");
    s.push_str("    d = c\n    c = b\n    b = a\n");
    s.push_str("    a = (t1 + t2) & m\n");
    s.push_str("    i += 1u64\n  }\n");
    // feedforward: hash_out = hash_in + compression output (per block)
    s.push_str("  a = (a + ea) & m\n  b = (b + eb) & m\n  c = (c + ec) & m\n  d = (d + ed) & m\n");
    s.push_str("  e = (e + ee) & m\n  f = (f + ef) & m\n  g = (g + eg) & m\n  h = (h + eh) & m\n");
    s.push_str("  blk += 1u64\n");
    s.push_str("}\n");
    s.push_str("var o1 = (a * 4294967296u64) + b\nvar o2 = (c * 4294967296u64) + d\n");
    s.push_str("var o3 = (e * 4294967296u64) + f\nvar o4 = (g * 4294967296u64) + h\n");
    s.push_str("return ((o1 as bytes) ++ (o2 as bytes)) ++ ((o3 as bytes) ++ (o4 as bytes))\n");
    s
}

/* ================================================================== */
/* M2-c tests.                                                         */
/* ================================================================== */

/// Known-answer across block boundaries: every length where the padded
/// shape changes (55→56→57, 63→64→65, 119→120, 128→129, 200) plus the
/// NIST two-block vector and a random-length sweep.
#[test]
fn h8_sha256_multi_block_known_answer() {
    let src = sha256_multi_block_src();
    let it = Interp::try_build(&src).unwrap_or_else(|e| panic!("interp build: {e}"));
    let mut msgs: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"abc".to_vec(),
        b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq".to_vec(),
    ];
    let mut rng = Rng::new(0x5A25_0201);
    for &n in &[
        1usize, 54, 55, 56, 57, 63, 64, 65, 100, 119, 120, 127, 128, 129, 191, 192, 200,
    ] {
        msgs.push(rng.byte_vec(n));
    }
    for msg in &msgs {
        let got = it
            .run(msg)
            .unwrap_or_else(|f| panic!("kernel run, msg len {}: {}", msg.len(), f.msg));
        let want = ref_sha256(msg);
        assert_eq!(
            hex(&got),
            hex(&want),
            "sha256 multi-block kernel diverged for msg len {}",
            msg.len()
        );
    }
    println!("h8: {} known-answer messages OK (cross-block boundary sweep)", msgs.len());
}

/// Four-way agreement on raw messages (the interpreter IS the spec; every
/// mode must produce identical bytes on every input, including all the
/// padding boundary lengths).
#[test]
fn h9_sha256_multi_block_agree4() {
    let src = sha256_multi_block_src();
    let mut inputs: Vec<Vec<u8>> = vec![b"".to_vec(), b"abc".to_vec()];
    let mut rng = Rng::new(0x5A25_0202);
    for &n in &[1usize, 55, 56, 57, 63, 64, 65, 120, 128, 129, 200] {
        inputs.push(rng.byte_vec(n));
    }
    agree4_hash(&src, &inputs);
    println!("h9: {} raw messages × 3 rv64 modes all agree with the interpreter", inputs.len());
}

/// Budget + cycles + determinism for the multi-block kernel.
#[test]
fn h10_sha256_multi_block_budget() {
    let src = sha256_multi_block_src();
    for (name, img) in measure_modes(&src) {
        println!(
            "sha256-multi {name}: code={} pool={} insts={} static-cycles={} locals={} container={} (vs 16383, headroom x{:.1})",
            img.code,
            img.pool,
            img.insts,
            img.cycles,
            img.locals,
            img.container,
            16383.0 / img.container as f64
        );
        assert!(img.container < 16383, "{name} container exceeds calc_image_max");
    }
    // dynamic cycles scale with the block count: 1 block vs 4 blocks
    for (len, label) in [(55usize, "1-block"), (200, "4-block")] {
        let mut rng = Rng::new(0x5A25_0203);
        let msg = rng.byte_vec(len);
        for (name, cyc) in dynamic_cycles(&src, &msg) {
            println!("sha256-multi {name} [{label}, {len}B]: dynamic cycles = {cyc}");
            assert!(cyc < 10_000_000);
        }
    }
    assert_deterministic(&src);
}



/* ================================================================== */
/* M2-d: per-region size attribution (prefix-diff method). Each prefix */
/* is a standalone body that compiles and runs; region size = prefix   */
/* delta. The 16383B admission INCLUDES rodata (cap.rs calc_image_max  */
/* docs "header + code + rodata + init"; reject_image_len measures    */
/* the fully encoded container), so code and pool are attributed       */
/* separately and summed at the container level.                      */
/* ================================================================== */

/// Prefixes of the sha256 single-block kernel (the M2-a form; the
/// multi-block kernel's attribution is its padding/chaining overhead,
/// measured by h10 minus h3).
fn sha256_prefix(prefix: usize) -> String {
    let full = sha256_single_block_src();
    // split points: 1 = after unpack loop, 2 = after K decl, 3 = after
    // schedule loop, 4 = after compress loop, 5 = full (IV tail + output)
    let p1 = "  o += 4u64\n}\n";
    let p2_mark = "var h = 1541459225u64\n";
    let p3 = "  j += 1u64\n}\n";
    let p4 = "  i += 1u64\n}\n";
    let i1 = full.find(p1).unwrap() + p1.len();
    let i2 = full.find(p2_mark).unwrap() + p2_mark.len();
    let i3 = full.find(p3).unwrap() + p3.len();
    let i4 = full.find(p4).unwrap() + p4.len();
    let cut = [0usize, i1, i2, i3, i4, full.len()][prefix.min(5)];
    let mut body = full[..cut].to_string();
    if prefix < 5 {
        body.push_str("return 0u64 as bytes\n");
    }
    body
}

#[test]
fn h11_sha256_region_attribution() {
    let block = pad_one_block(b"abc");
    let mut prev_code = 0usize;
    let mut prev_pool = 0usize;
    let mut prev_cyc = 0u64;
    for (name, p) in [
        ("prologue/epilogue baseline", 0usize),
        ("+ input unpack loop (16 words)", 1),
        ("+ K table decl (rodata)", 2),
        ("+ message schedule loop", 3),
        ("+ compression loop", 4),
        ("+ IV tail + output packing (full)", 5),
    ] {
        let body = sha256_prefix(p);
        // every prefix must RUN (attribution bodies are real programs)
        let it = Interp::try_build(&body).unwrap_or_else(|e| panic!("{name}: {e}"));
        let _ = it.run(&block).unwrap_or_else(|e| panic!("{name} run: {}", e.msg));
        let imgs = measure_modes(&body);
        let (_, im) = imgs.iter().find(|(n, _)| *n == "ImcRemap").unwrap();
        println!(
            "sha256-regions {name:38} code={:4} (+{:4}) pool={:3} (+{:3}) static-cyc={:4} (+{:4})",
            im.code,
            im.code.saturating_sub(prev_code),
            im.pool,
            im.pool.saturating_sub(prev_pool),
            im.cycles,
            im.cycles.saturating_sub(prev_cyc)
        );
        prev_code = im.code;
        prev_pool = im.pool;
        prev_cyc = im.cycles;
    }
}

/// Prefixes of the keccak scalar kernel (the shipped rodata-table form;
/// the LFSR round-loop delta is measured by h7).
fn keccak_prefix(prefix: usize) -> String {
    let full = keccak_scalar_src(true);
    // split points: 1 = after the 17-lane absorb, 2 = after the round loop
    let a_mark = "var rc_t = [";
    let r_mark = "  r += 1u64\n}\n";
    let i0 = full.find(a_mark).unwrap();
    let i2 = full.find(r_mark).unwrap() + r_mark.len();
    let cut = [0usize, i0, i2, full.len()][prefix.min(3)];
    let mut body = full[..cut].to_string();
    if prefix == 1 {
        // after the absorb prefix s0 exists; return it
        body.push_str("var r = s0\nreturn (r as u64) as bytes\n");
    } else if prefix == 0 {
        body.push_str("return 0u64 as bytes\n");
    }
    body
}

#[test]
fn h12_keccak_region_attribution() {
    let block = pad_keccak_block(b"abc");
    let mut prev_code = 0usize;
    let mut prev_pool = 0usize;
    let mut prev_cyc = 0u64;
    for (name, p) in [
        ("prologue/epilogue baseline", 0usize),
        ("+ absorb (136 byte() sites, 17 lanes)", 1),
        ("+ 24-round permutation loop (rc rodata)", 2),
        ("+ squeeze + output (full)", 3),
    ] {
        let body = keccak_prefix(p);
        let it = Interp::try_build(&body).unwrap_or_else(|e| panic!("{name}: {e}"));
        let _ = it.run(&block).unwrap_or_else(|e| panic!("{name} run: {}", e.msg));
        let imgs = measure_modes(&body);
        let (_, im) = imgs.iter().find(|(n, _)| *n == "ImcRemap").unwrap();
        println!(
            "keccak-regions {name:40} code={:4} (+{:4}) pool={:3} (+{:3}) static-cyc={:4} (+{:4})",
            im.code,
            im.code.saturating_sub(prev_code),
            im.pool,
            im.pool.saturating_sub(prev_pool),
            im.cycles,
            im.cycles.saturating_sub(prev_cyc)
        );
        prev_code = im.code;
        prev_pool = im.pool;
        prev_cyc = im.cycles;
    }
}

/// M2-d fact pin: the 16383B budget is the ENCODED CONTAINER (header +
/// segment table + code + rodata + arena headers) — rodata IS inside the
/// budget, so rodata optimizations are real admission wins.
#[test]
fn h13_container_budget_includes_rodata() {
    let src = sha256_single_block_src();
    let rv = Rv64::try_build_out(&src, 64, RUN_ARENA).unwrap();
    for (mode, bytes) in &rv.containers {
        // the container exceeds the code mirror by header + segment table
        // + the FULL rodata segment (+ arena segment headers)
        let want_name = format!("{mode:?}");
        let (_, im) = measure_modes(&src)
            .into_iter()
            .find(|(n, _)| *n == want_name)
            .unwrap();
        assert!(
            bytes.len() > im.code + im.pool,
            "{mode:?}: container {} must exceed code+pool {}+{}",
            bytes.len(),
            im.code,
            im.pool
        );
        assert!(bytes.len() < 16383);
        println!(
            "{mode:?}: container {} = code {} + rodata {} + envelope {} (budget 16383 = cap.calc_image_max, cap.rs:122; reject_image_len measures the encoded container, calc/mod.rs:600)",
            bytes.len(),
            im.code,
            im.pool,
            bytes.len() - im.code - im.pool
        );
    }
}

/* ================================================================== */
/* P3-GUARDTAIL GT-c: zero-byte-impact probe.                          */
/* ================================================================== */

/// Container sha256 for the three hash kernels in every mode. GT-c reworks
/// error-message text in rv64.rs; this probe pins the obligation that not a
/// single container byte moves: run it against the pre-GT-c and post-GT-c
/// trees and diff the printed table (doc 96 §5). `ref_sha256` is the
/// in-file hand-written reference (no external crate, the M2-a precedent).
#[test]
fn gt_c_container_sha256_probe() {
    let cap = vm::calc::SpaceCap::new(0);
    for (name, src) in [
        ("sha256_single_block", sha256_single_block_src()),
        ("sha256_multi_block", sha256_multi_block_src()),
        ("keccak_scalar_rodata", keccak_scalar_src(true)),
    ] {
        let (ir, _) = frontend(&src).unwrap_or_else(|e| panic!("frontend {name}: {e}"));
        for &mode in c3lib::MODES.iter() {
            let lo = lower_body_mode_arena(&ir, true, mode, RUN_ARENA)
                .unwrap_or_else(|e| panic!("lower {name} {mode:?}: {e}"));
            let profile = match mode {
                Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
                _ => vm::calc::container::CALC_PROFILE_RV64IMC,
            };
            let container = lo
                .pack_segments_profile(&cap, 64, 4800, 64, profile)
                .unwrap_or_else(|e| panic!("pack {name} {mode:?}: {e}"))
                .encode()
                .unwrap_or_else(|e| panic!("encode {name} {mode:?}: {e}"));
            println!(
                "GT-C-PROBE {name} {:?}: {} B sha256 {}",
                mode,
                container.len(),
                hex(&ref_sha256(&container))
            );
        }
    }
}
