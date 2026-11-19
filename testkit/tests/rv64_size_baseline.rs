//! fitsh→RV64 compiler SIZE BASELINE tool — the measurement basis for every
//! later code-size optimization on vm/src/ir/rv64.
//!
//! What it does, per corpus case × lowering mode (`Im` / `Imc` / `ImcRemap`):
//!   * fitsh source → tokens (`vm::lang::Tokenizer`) → `IRNodeArray`
//!     (`vm::fitshc::compile_body`, with the `(data: bytes)` kernel
//!     parameter — the exact front-end path of
//!     `vm::fitshc::kernel::compile_rv64im_body_kernel`);
//!   * IR → machine code through `vm::ir::rv64::lower_body_mode_arena_heap`
//!     with the production arena budget (`Rv64imBodyMeta::default` ceilings:
//!     mem 64 KiB, L1 input/output, `SpaceCap::heap_grow_segment`);
//!   * DETERMINISM SELF-CHECK: every case×mode is lowered TWICE and the two
//!     code/pool byte streams must be identical (asserted);
//!   * the packed container must pass the consensus admission
//!     (`pack_segments_profile` + `verify_container_with_cap`);
//!   * stats recorded from `Rv64Lowered.stats` (insts / code_bytes /
//!     cycles / locals) plus the literal-pool size.
//!
//! Output: a long-format TSV (`name  group  mode  insts  code_bytes  cycles
//! locals  pool_bytes`, one row per case×mode, plus per-mode TOTAL rows) and
//! per-group subtotals. When a previous baseline.tsv exists next to this
//! tool's fixtures, a delta report (old → new per case and per mode) is
//! printed — re-run the same command after changing the compiler and read
//! the drift straight off the output.
//!
//! Run:
//!   cargo test -p testkit --offline --test rv64_size_baseline -- --ignored --nocapture
//!
//! Regenerate the stored baseline (writes baseline.tsv + baseline.md into
//! testkit/tests/fixtures/rv64_size_baseline/ — the git-tracked location of
//! record; docs.loc/ is ignored by `*.loc/` in .gitignore):
//!   RV64_BASELINE_WRITE=1 cargo test -p testkit --offline --test rv64_size_baseline -- --ignored --nocapture
//!
//! Corpus provenance (every body is a verbatim copy or a faithful generator
//! port; groups tag the source):
//!   * `micro`    — hand-built snippets for the shapes no other group
//!                  reaches (u16 checked faces, dominated constant-mul
//!                  checks, FIN SqrtMul, u128 pair arithmetic, item_concat,
//!                  compound item assignment, heap ops, else-if chains);
//!   * `vmunit`   — the canonical `lowers_core_constructs` bodies from
//!                  vm/src/ir/rv64/tests/subset_tests.rs;
//!   * `net.g1`..`net.g9` — the hacdtestnet three_way campaign bodies, as
//!                  already embedded (body set, deduplicated) in
//!                  testkit/tests/calc_aot_differential.rs;
//!   * `net.g10`  — algorithm bodies from fullnodetest
//!                  g10_algorithms.rs (median network, settle, sha256-r4,
//!                  keccak-r1, uni_v2, black-scholes, rpow);
//!   * `net.g17`  — tier2 production crops from g17_tier2.rs (dict-walk
//!                  parser, price-feed TWAP, CPMM swap quote);
//!   * `prod`     — the tracked production kernels
//!                  vm/tests/fitsh_kernels_{score_sum,hacd200}.inc and the
//!                  sha256 single-block kernel generator from
//!                  vm/tests/rv64_hash_kernels.rs.
//!
//! Discipline: this file only measures; it must stay free of corpus changes
//! between runs so numbers stay comparable (the delta report also flags
//! added/removed cases).

use std::collections::HashMap;
use std::path::PathBuf;

use vm::calc::container::{CalcContainer, CODE_BASE, CALC_PROFILE_RV64IM, CALC_PROFILE_RV64IMC};
use vm::calc::verify::verify_container_with_cap;
use vm::calc::{SpaceCap, CALC_STACK_GUARD, CALC_STACK_SIZE};
use vm::fitshc::compile_body;
use vm::ir::rv64::{lower_body_mode_arena_heap, ARENA_BASE, ARENA_SIZE_MAX, Rv64Mode};
use vm::lang::Tokenizer;
use vm::{IRNodeArray, ValueTy};

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

fn mode_name(m: Rv64Mode) -> &'static str {
    match m {
        Rv64Mode::Im => "Im",
        Rv64Mode::Imc => "Imc",
        Rv64Mode::ImcRemap => "ImcRemap",
    }
}

fn profile_byte(m: Rv64Mode) -> u8 {
    match m {
        Rv64Mode::Im => CALC_PROFILE_RV64IM,
        _ => CALC_PROFILE_RV64IMC,
    }
}

/// fitsh body source → IR, mirroring `compile_rv64im_body_kernel`'s front end
/// (same tokenizer, same `(data: bytes)` argument, same `is_ircode=false`).
fn body_to_ir(body: &str) -> IRNodeArray {
    let tokens = Tokenizer::new(body.as_bytes())
        .parse()
        .unwrap_or_else(|e| panic!("tokenize failed: {e}\nbody:\n{body}"));
    let (ir, _, _) = compile_body(tokens, vec![("data".to_string(), ValueTy::Bytes)], &[], &[], false)
        .unwrap_or_else(|e| panic!("front end rejected body: {e}\nbody:\n{body}"));
    ir
}

/// The production kernel arena budget — a byte-for-byte mirror of the
/// computation in `vm/src/fitshc/kernel.rs` under `Rv64imBodyMeta::default`
/// (mem_kib 64, L1 input/output ceilings). The arena end address is baked
/// into the emitted code (`li a0, ARENA_BASE + arena_size` bounds checks), so
/// matching it keeps the baseline identical to production container bytes.
fn kernel_arena_size(cap: &SpaceCap) -> u64 {
    let mem_kib = 64u64;
    let input_max = cap.call_data_size as u64;
    let output_max = cap.value_size as u64;
    let stack_gap = CALC_STACK_GUARD + CALC_STACK_SIZE;
    let budget = (mem_kib * 1024)
        .saturating_sub(ARENA_BASE - CODE_BASE)
        .saturating_sub(input_max)
        .saturating_sub(output_max)
        .saturating_sub(stack_gap)
        .saturating_sub(30);
    ARENA_SIZE_MAX.min(budget)
}

fn lower_once(ir: &IRNodeArray, mode: Rv64Mode, arena: u64, cap: &SpaceCap) -> vm::ir::rv64::Rv64Lowered {
    lower_body_mode_arena_heap(ir, true, mode, arena, cap.heap_grow_segment)
        .unwrap_or_else(|e| panic!("rv64 lowering failed (mode {}): {e}", mode_name(mode)))
}

/* ==================================================================== */
/* Corpus                                                               */
/* ==================================================================== */

struct Case {
    name: &'static str,
    group: &'static str,
    body: String,
}

fn case(name: &'static str, group: &'static str, body: impl Into<String>) -> Case {
    Case { name, group, body: body.into() }
}

/* ---------- shared body builders (ported from the differential suite) ---------- */

#[derive(Clone, Copy, PartialEq, Eq)]
enum W {
    U8,
    U64,
}

impl W {
    fn nbytes(self) -> usize {
        match self {
            W::U8 => 1,
            W::U64 => 8,
        }
    }
}

/// Assemble a width-W value from `data[off..off+n]` BE (g1 `read_lines`).
fn read_lines(v: &str, off: usize, w: W) -> String {
    match w {
        W::U8 => format!("var {v} = byte(data, {off})"),
        W::U64 => format!(
            "var {v} = byte(data, {off}) * 72057594037927936 + byte(data, {o1}) * 281474976710656 + byte(data, {o2}) * 1099511627776 + byte(data, {o3}) * 4294967296 + byte(data, {o4}) * 16777216 + byte(data, {o5}) * 65536 + byte(data, {o6}) * 256 + byte(data, {o7})",
            o1 = off + 1, o2 = off + 2, o3 = off + 3, o4 = off + 4, o5 = off + 5, o6 = off + 6, o7 = off + 7
        ),
    }
}

fn bin_body(w: W, op: &str) -> String {
    format!(
        "{}\n{}\nreturn ((a {op} b) as bytes)",
        read_lines("a", 0, w),
        read_lines("b", w.nbytes(), w)
    )
}

fn fin2_body(w: W, f: &str) -> String {
    format!(
        "{}\n{}\nreturn (({f}(a, b)) as bytes)",
        read_lines("a", 0, w),
        read_lines("b", w.nbytes(), w)
    )
}

fn clamp_body(w: W) -> String {
    let n = w.nbytes();
    format!(
        "{}\n{}\n{}\nreturn ((clamp(a, lo, hi)) as bytes)",
        read_lines("a", 0, w),
        read_lines("lo", n, w),
        read_lines("hi", 2 * n, w)
    )
}

fn pattern_byte(i: usize, salt: u32) -> u8 {
    (((i as u32).wrapping_mul(7) + salt) & 0xFF) as u8
}

fn pattern_hex(n: usize, salt: u32) -> String {
    (0..n).map(|i| format!("{:02x}", pattern_byte(i, salt))).collect()
}

/// Like g2 `pin_body`: one body carries multiple pins; byte 0 selects the face.
fn pin_body(pins: &[String]) -> String {
    let mut b = String::from("var s = (byte(data, 0) as u64)\nvar r = (0u64) as bytes\n");
    for (i, p) in pins.iter().enumerate() {
        b.push_str(&format!("if (s == {i}u64) {{ var t{i} = {p}\nr = (t{i}) as bytes }}\n"));
    }
    b.push_str("return r");
    b
}

const MAX_U64: &str = "18446744073709551615";
const MAX_U128: &str = "340282366920938463463374607431768211455";
const BIG1: &str = "123456789012345678901234567890123456789";
const BIG2: &str = "98765432109876543210987654321";
const BIGM: &str = "1000000000000000000000000000007";

/* ---------- g10 generator ports (fullnodetest g10_algorithms.rs) ---------- */

fn median_fitsh() -> String {
    let mut src = String::new();
    for i in 0..16u64 {
        let v = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let r = v.rotate_left(7);
        src.push_str(&format!("var m{i} = {r}u64\n"));
    }
    let net: [(usize, usize); 60] = [
        (0, 1), (2, 3), (4, 5), (6, 7), (8, 9), (10, 11), (12, 13), (14, 15),
        (0, 2), (1, 3), (4, 6), (5, 7), (8, 10), (9, 11), (12, 14), (13, 15),
        (0, 4), (1, 5), (2, 6), (3, 7), (8, 12), (9, 13), (10, 14), (11, 15),
        (0, 8), (1, 9), (2, 10), (3, 11), (4, 12), (5, 13), (6, 14), (7, 15),
        (5, 10), (6, 9), (3, 12), (13, 14), (1, 2), (4, 8), (7, 11), (11, 14),
        (2, 4), (5, 8), (9, 12), (13, 15), (3, 8), (7, 12), (1, 4), (9, 13),
        (3, 5), (7, 9), (11, 13), (2, 3), (6, 7), (10, 11), (12, 13), (5, 6),
        (9, 10), (3, 4), (7, 8), (11, 12),
    ];
    for (n, (a, b)) in net.iter().enumerate() {
        src.push_str(&format!(
            "if m{a} > m{b} {{ var t{n} = m{a}\nm{a} = m{b}\nm{b} = t{n} }}\n"
        ));
    }
    src.push_str("return m8 as bytes");
    src
}

fn settle_fitsh() -> String {
    r#"var pot = 1000000000000000000u64
var q = pot / 5050
var r = pot - q * 5050
var out = ""
var g = 0u64
while g < 10 {
    var grp = ""
    var k = 0u64
    while k < 10 {
        var idx = g * 10 + k + 1
        var pay = q * idx + (r * idx) / 5050
        var w8 = pay as bytes
        grp = grp ++ w8
        k += 1
    }
    out = out ++ grp
    g += 1
}
return out
"#
    .to_string()
}

fn sha256_reduced_fitsh(rounds: usize) -> String {
    const K4: [u32; 4] = [0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5];
    const IV: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut s = String::new();
    for (n, v) in ["a", "b", "c", "d", "e", "f", "g", "h"].iter().zip(IV) {
        s += &format!("var {n}={v}u64\n");
    }
    s += "var k1=256u64\nvar k2=65536u64\nvar k3=16777216u64\n";
    for i in 0..8u64 {
        let (b0, b1, b2, b3) = (4 * i, 4 * i + 1, 4 * i + 2, 4 * i + 3);
        s += &format!(
            "var w{i}=(byte(data,{b0})*k3)+((byte(data,{b1})*k2)+((byte(data,{b2})*k1)+byte(data,{b3})))\n"
        );
    }
    for t in 0..rounds {
        let k = K4[t];
        s += &format!("var s1{t}=((e>>6)|(e<<26))^((e>>11)|(e<<21))^((e>>25)|(e<<7))\n");
        s += &format!("var cq{t}=((e&f)^(((e^4294967295)&g)))\n");
        s += &format!("var u1{t}=(h+s1{t}+cq{t}+{k}+w{t})&4294967295\n");
        s += &format!("var s0{t}=((a>>2)|(a<<30))^((a>>13)|(a<<19))^((a>>22)|(a<<10))\n");
        s += &format!("var mj{t}=((a&b)^((a&c)^(b&c)))\n");
        s += &format!("var u2{t}=(s0{t}+mj{t})&4294967295\n");
        s += "h=g\ng=f\nf=e\n";
        s += &format!("e=(d+u1{t})&4294967295\nd=c\nc=b\nb=a\n");
        s += &format!("a=(u1{t}+u2{t})&4294967295\n");
    }
    s += "var out=\"\"\n";
    for n in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        s += &format!("var wb{n}={n} as bytes\nout=out++buf_cut(wb{n},4,4)\n");
    }
    s += "return out\n";
    s
}

fn keccak_r1_fitsh() -> String {
    const RHO: [u32; 25] = [
        0, 1, 62, 28, 27,
        36, 44, 6, 55, 20,
        3, 10, 43, 25, 39,
        41, 45, 15, 21, 8,
        18, 2, 61, 56, 14,
    ];
    let mut s = String::new();
    for i in 0..25u64 {
        let (b0, b1) = (2 * i, 2 * i + 1);
        s += &format!("var l{i}=byte(data,{b0})*256u64+byte(data,{b1})\n");
    }
    for x in 0..5u64 {
        s += &format!("var c{x}=l{x}^l{}^l{}^l{}^l{}\n", x + 5, x + 10, x + 15, x + 20);
    }
    for x in 0..5u64 {
        let (xm, xp) = ((x + 4) % 5, (x + 1) % 5);
        s += &format!("var d{x}=c{xm}^((c{xp}<<1)|(c{xp}>>63))\n");
    }
    for x in 0..5u64 {
        for y in 0..5u64 {
            s += &format!("l{}=l{}^d{x}\n", x + 5 * y, x + 5 * y);
        }
    }
    for x in 0..5u64 {
        for y in 0..5u64 {
            let idx = y + 5 * ((2 * x + 3 * y) % 5);
            let r = RHO[(x + 5 * y) as usize];
            if r == 0 {
                s += &format!("var b{idx}=l{}\n", x + 5 * y);
            } else {
                s += &format!("var b{idx}=(l{}<<{r})|(l{}>>{})\n", x + 5 * y, x + 5 * y, 64 - r);
            }
        }
    }
    s += "b0=b0^1\n";
    s += "var out2=\"\"\n";
    for i in 0..8u64 {
        s += &format!("var q{i}=b{i} as bytes\nout2=out2++q{i}\n");
    }
    s += "return out2\n";
    s
}

fn uni_v2_fitsh() -> String {
    r#"var ain = 0u64
var i = 0u64
while i < 8 {
    ain = ain * 256 + byte(data, i)
    i += 1
}
var rin = 0u64
var j = 0u64
while j < 8 {
    rin = rin * 256 + byte(data, 8 + j)
    j += 1
}
var rout = 0u64
var k = 0u64
while k < 8 {
    rout = rout * 256 + byte(data, 16 + k)
    k += 1
}
var feein = ain * 997
var den = rin * 1000 + feein
return mul_div_floor(feein, rout, den) as bytes
"#
    .to_string()
}

fn black_scholes_fitsh() -> String {
    r#"var one = 1000000000u64
var d1 = 0u64
var i = 0u64
while i < 8 {
    d1 = d1 * 256 + byte(data, i)
    i += 1
}
var s = 0u64
var j = 0u64
while j < 8 {
    s = s * 256 + byte(data, 8 + j)
    j += 1
}
var kk = 0u64
var k = 0u64
while k < 8 {
    kk = kk * 256 + byte(data, 16 + k)
    k += 1
}
var t = mul_div_floor(one, one, one + mul_div_floor(327591, d1, one))
var tt = mul_div_floor(t, t, one)
var p1 = mul_div_floor(254829592, t, one)
var p2 = mul_div_floor(284496736, tt, one)
var poly = p1 - p2
var nd = 500000000 + mul_div_floor(poly, 1, one)
var pay = sat_sub(s, kk)
var out = ""
var w8 = t as bytes
out = out ++ w8
w8 = poly as bytes
out = out ++ w8
w8 = nd as bytes
out = out ++ w8
w8 = pay as bytes
out = out ++ w8
return out
"#
    .to_string()
}

fn rpow_fitsh() -> String {
    r#"var x = 0u64
var i = 0u64
while i < 8 {
    x = x * 256 + byte(data, i)
    i += 1
}
var n = 0u64
var j = 0u64
while j < 8 {
    n = n * 256 + byte(data, 8 + j)
    j += 1
}
var z = 0u64
var k = 0u64
while k < 8 {
    z = z * 256 + byte(data, 16 + k)
    k += 1
}
return rpow_half_up(x, n, z) as bytes
"#
    .to_string()
}

/* ---------- g17 tier2 crops (fullnodetest g17_tier2.rs, verbatim) ---------- */

const G17_DICT_WALK_BODY: &str = r#"var n = size(data)
var bad = 0u64
var i = 0u64
var closed = 0u64
if n < 5 {
 bad = 1u64
} else {
 if byte(data,0)!=123 {
  bad = 1u64
 } else {
  i = 1u64
  while bad==0&&closed==0 {
   var k0 = i
   while i < n {
    var c = byte(data,i)
    if c>=97&&c<=122 { i+=1 } else { break }
   }
   if i==k0 { bad = 1u64; break }
   if i>=n { bad = 1u64; break }
   if byte(data,i)!=61 { bad = 1u64; break }
   i+=1
   var v0 = i
   while i < n {
    var cv = byte(data,i)
    if (cv>=48&&cv<=57)||(cv>=65&&cv<=90)||(cv>=97&&cv<=122) { i+=1 } else { break }
   }
   if i==v0 { bad = 1u64; break }
   if i>=n { bad = 1u64; break }
   var ke = v0-1
   var p = 1u64
   var dup = 0u64
   while dup==0&&p < k0 {
    var pc = byte(data,p-1)
    if pc==123||pc==44 {
     var a = p
     while a < n {
      var ca = byte(data,a)
      if ca>=97&&ca<=122 { a+=1 } else { break }
     }
     if a-p==ke-k0 {
      var eq = 1u64
      var q = 0u64
      while q < ke-k0 {
       if byte(data,p+q)!=byte(data,k0+q) { eq = 0u64 }
       q+=1
      }
      if eq==1 { dup = 1u64 }
     }
    }
    p+=1
   }
   if dup==1 { bad = 1u64; break }
   var term = byte(data,i)
   if term==44 { i+=1 } else {
    if term==125 { closed = 1u64; i+=1 } else { bad = 1u64; break }
   }
  }
  if bad==0 {
   if i!=n { bad = 1u64 }
  }
 }
}
var acc = ""
if bad==0 {
 var ksrc = 0u64
 var klen = 5u64
 var vpos = 0u64
 var vlen = 0u64
 var steps = 0u64
 var done = 0u64
 var kstart = "start"
 while done==0&&bad==0 {
  if steps>=64 {
   bad = 1u64
  } else {
   var found = 0u64
   var wp = 1u64
   while found==0&&wp < n {
    var wpc = byte(data,wp-1)
    if wpc==123||wpc==44 {
     var wa = wp
     while wa < n {
      var wca = byte(data,wa)
      if wca>=97&&wca<=122 { wa+=1 } else { break }
     }
     if wa-wp==klen {
      var weq = 1u64
      var wq = 0u64
      while wq < klen {
       var wkb = byte(data,vpos+wq)
       if ksrc==0 { wkb = byte(kstart,wq) }
       else { wkb = byte(data,vpos+wq) }
       if wkb!=byte(data,wp+wq) { weq = 0u64 }
       wq+=1
      }
      if weq==1 { found = 1u64; vpos = wp; }
     }
    }
    wp+=1
   }
   if found==0 { bad = 1u64 } else {
    var ve = vpos+klen+1
    var vend = ve
    while vend < n {
     var vc = byte(data,vend)
     if vc==44||vc==125 { break }
     vend+=1
    }
    var vstr = buf_cut(data,ve,vend-ve)
    if vlen>0 {
     acc = acc ++ buf_cut(data,vpos,klen)
    }
    acc = acc ++ vstr
    if vend-ve==3 {
     if byte(data,ve)==101&&byte(data,ve+1)==110&&byte(data,ve+2)==100 { done = 1u64 }
    }
    ksrc = 1u64
    klen = vend-ve
    vpos = ve
    vlen = klen
    steps+=1
   }
  }
 }
}
return acc
"#;

const G17_TWAP_BODY: &str = r#"// g17 tier2: price_feed TWAP cumulative window pure crop
// wire = [p_init 8B]++[(p 8B)(dt 8B)]* ; output = cum 8B++last 8B
var total = size(data)
var last = 0u64
var i = 0u64
while i < 8 {
 last = last * 256u64 + byte(data,i)
 i+=1
}
var k = (total - 8) / 16
var cum = 0u64
var j = 0u64
while j < k {
 var p = 0u64
 var b = 8 + j * 16
 var t = 0u64
 while t < 8 {
  p = p * 256u64 + byte(data,b + t)
  t+=1
 }
 var dt = 0u64
 var u = 0u64
 while u < 8 {
  dt = dt * 256u64 + byte(data,b + 8 + u)
  u+=1
 }
 if dt > 0 {
  cum = cum + last * dt
 }
 last = p
 j+=1
}
var oc = cum as bytes
var ol = last as bytes
return oc++ol
"#;

const G17_CPMM_QUOTE_BODY: &str = r#"// g17 tier2: AssetPairCpmmBase._swap_quote pure crop (u64 scaling)
// wire = [fee_num 8B][rin 8B][rout 8B][ain 8B]; out = errno 8B++quote 8B
var fee_num = 0u64
var i = 0u64
while i < 8 {
 fee_num = fee_num * 256u64 + byte(data,i)
 i+=1
}
var rin = 0u64
var j = 0u64
while j < 8 {
 rin = rin * 256u64 + byte(data,8 + j)
 j+=1
}
var rout = 0u64
var k = 0u64
while k < 8 {
 rout = rout * 256u64 + byte(data,16 + k)
 k+=1
}
var ain = 0u64
var q = 0u64
while q < 8 {
 ain = ain * 256u64 + byte(data,24 + q)
 q+=1
}
var errno = 0u64
var out = 0u64
if rin==0||rout==0||ain==0 {
 errno = 1u64
} else {
 var eff = ain * fee_num
 var den = rin * 10000u64 + eff
 out = rout * eff / den
 if out==0||out>=rout {
  errno = 2u64
  out = 0u64
 }
}
var oe = errno as bytes
var oo = out as bytes
return oe++oo
"#;

/* ---------- sha256 single-block kernel (vm/tests/rv64_hash_kernels.rs port) ---------- */

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x7a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c8dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_single_block_src() -> String {
    let mut s = String::new();
    s.push_str("var w = []\nvar o = 0u64\nwhile o < 64u64 {\n");
    s.push_str("  append(w, byte(data, o) * 16777216u64 + byte(data, o + 1u64) * 65536u64 + byte(data, o + 2u64) * 256u64 + byte(data, o + 3u64))\n");
    s.push_str("  o += 4u64\n}\n");
    let items: Vec<String> = SHA256_K.iter().map(|&v| format!("{v}u64")).collect();
    s.push_str(&format!("var k = [{}]\n", items.join(", ")));
    s.push_str("var m = 4294967295u64\n");
    s.push_str("var a = 1779033703u64\nvar b = 3144134277u64\nvar c = 1013904242u64\nvar d = 2773480762u64\n");
    s.push_str("var e = 1359893119u64\nvar f = 2600822924u64\nvar g = 528734635u64\nvar h = 1541459225u64\n");
    s.push_str("var j = 16u64\nwhile j < 64u64 {\n");
    s.push_str("  var w15 = w[j - 15u64]\n  var w2 = w[j - 2u64]\n");
    s.push_str("  var s0 = (((w15 >> 7u64) | (w15 << 25u64)) & m) ^ (((w15 >> 18u64) | (w15 << 14u64)) & m) ^ (w15 >> 3u64)\n");
    s.push_str("  var s1 = (((w2 >> 17u64) | (w2 << 15u64)) & m) ^ (((w2 >> 19u64) | (w2 << 13u64)) & m) ^ (w2 >> 10u64)\n");
    s.push_str("  append(w, (w[j - 16u64] + s0 + w[j - 7u64] + s1) & m)\n");
    s.push_str("  j += 1u64\n}\n");
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
    s.push_str("a = (a + 1779033703u64) & m\nb = (b + 3144134277u64) & m\nc = (c + 1013904242u64) & m\nd = (d + 2773480762u64) & m\n");
    s.push_str("e = (e + 1359893119u64) & m\nf = (f + 2600822924u64) & m\ng = (g + 528734635u64) & m\nh = (h + 1541459225u64) & m\n");
    s.push_str("var o1 = (a * 4294967296u64) + b\nvar o2 = (c * 4294967296u64) + d\n");
    s.push_str("var o3 = (e * 4294967296u64) + f\nvar o4 = (g * 4294967296u64) + h\n");
    s.push_str("return ((o1 as bytes) ++ (o2 as bytes)) ++ ((o3 as bytes) ++ (o4 as bytes))\n");
    s
}

/* ---------- the corpus ---------- */

fn corpus() -> Vec<Case> {
    let mut v: Vec<Case> = Vec::new();

    /* ---- micro: shapes no other group reaches ---- */
    v.push(case("micro.arith.u16.checked", "micro",
        "var a = byte(data, 0) * 256 + byte(data, 1)\nvar b = byte(data, 2) * 256 + byte(data, 3)\nvar s = (a + b) as bytes\nvar d = (a * b) as bytes\nreturn s ++ d"));
    v.push(case("micro.mulcheck.dominated", "micro",
        &format!("{}\nvar x = a * 2u64\nvar y = a * 3u64\nreturn ((x + y) as bytes)", read_lines("a", 0, W::U64))));
    v.push(case("micro.fin2.sqrt_mul", "micro",
        &format!("{}\n{}\nvar lo = sqrt_mul_floor(a, b)\nvar hi = sqrt_mul_ceil(a, b)\nreturn ((lo + hi) as bytes)", read_lines("a", 0, W::U64), read_lines("b", 8, W::U64))));
    v.push(case("micro.u128.arith", "micro",
        "var x = (byte(data, 0) as u128) * 18446744073709551616u128\nvar y = x + 12345u128\nvar z = y / 3u128\nvar w = y % 7u128\nvar v2 = y - z\nreturn ((z + w + v2) as bytes)"));
    v.push(case("micro.list.itemcat", "micro",
        "var l = [1u64, 2u64]\nreturn item_concat(l)"));
    v.push(case("micro.list.itemxop", "micro",
        "var l = [7u64, 8u64]\nl[0] += (byte(data, 0) as u64) + 1u64\nl[1] *= 5u64\nreturn ((l[0] + l[1]) as u64) as bytes"));
    v.push(case("micro.heap.rw", "micro",
        "heap_grow(4)\nheap_write(0, 258)\nvar b = heap_read(0, 2)\nvar u = (heap_read_uint(192) as u64)\nreturn ((b as bytes) ++ (u as bytes))"));
    v.push(case("micro.ctrl.elsechain", "micro",
        "var a = (byte(data, 0) as u64)\nvar r = 0u64\nif a == 0u64 { r = 10u64 } else { if a == 1u64 { r = 20u64 } else { if a == 2u64 { r = 30u64 } else { if a == 3u64 { r = 40u64 } else { r = 50u64 } } } }\nreturn (r as u64) as bytes"));

    /* ---- vmunit: vm/src/ir/rv64/tests/subset_tests.rs `lowers_core_constructs` ---- */
    v.push(case("vmunit.while-sum", "vmunit",
        "var s = 0u64\nvar i = 0u64\nwhile i < 100 { s += i\ni += 1 }\nreturn s as bytes"));
    v.push(case("vmunit.bytes-idiom", "vmunit",
        "var b = 0x00112233445566778899aabbccddeeff\nreturn b ++ buf_cut(b, 1, 4) ++ buf_right(4, b)"));
    v.push(case("vmunit.bytes-fold", "vmunit",
        "var b = 0x01020304\nvar i = 0u64\nvar s = 0u64\nwhile i < size(b) { s = s * 256 + byte(b, i)\ni += 1 }\nreturn s as bytes"));
    v.push(case("vmunit.if-else-return", "vmunit",
        "if 3 > 2 { return 0x01 } else { return 0x02 }"));
    v.push(case("vmunit.require", "vmunit",
        "require 1001, 1 < 2\nreturn 0xff"));
    v.push(case("vmunit.throw", "vmunit", "throw 42"));
    v.push(case("vmunit.fin-minmax-clamp", "vmunit",
        "var x = min(3, 5) + max(2, 9) + clamp(11, 1, 10)\nreturn x as bytes"));
    v.push(case("vmunit.pow-mod", "vmunit",
        "var x = (2 ** 10) % 1000\nreturn x as bytes"));

    /* ---- net.g1 (arith; body set of the differential g1) ---- */
    v.push(case("g1.add.u8", "net.g1", bin_body(W::U8, "+")));
    v.push(case("g1.add.u64", "net.g1", bin_body(W::U64, "+")));
    v.push(case("g1.sub.u64", "net.g1", bin_body(W::U64, "-")));
    v.push(case("g1.mul.u64", "net.g1", bin_body(W::U64, "*")));
    v.push(case("g1.div.u64", "net.g1", bin_body(W::U64, "/")));
    v.push(case("g1.mod.u64", "net.g1", bin_body(W::U64, "%")));
    v.push(case("g1.bshl.u64", "net.g1", bin_body(W::U64, "<<")));
    v.push(case("g1.bshr.u64", "net.g1", bin_body(W::U64, ">>")));
    v.push(case("g1.pow.u64", "net.g1", bin_body(W::U64, "**")));
    v.push(case("g1.clamp.u64", "net.g1", clamp_body(W::U64)));
    v.push(case("g1.max.u64", "net.g1", fin2_body(W::U64, "max")));
    v.push(case("g1.min.u64", "net.g1", fin2_body(W::U64, "min")));
    v.push(case("g1.absdiff.u64", "net.g1", fin2_body(W::U64, "abs_diff")));
    v.push(case("g1.divup.u64", "net.g1", fin2_body(W::U64, "div_up")));
    v.push(case("g1.divexact.u64", "net.g1", fin2_body(W::U64, "div_exact_op")));
    v.push(case("g1.mixed.add.u8u64", "net.g1",
        "var a = byte(data, 0)\nvar b = (byte(data, 1) as u64)\nreturn ((a + b) as bytes)"));
    v.push(case("g1.mixed.div.u8u128", "net.g1",
        "var a = byte(data, 0)\nvar b = (byte(data, 1) as u128)\nreturn ((a / b) as bytes)"));
    v.push(case("g1.mixed.bshl.u8u16", "net.g1",
        "var a = 1u8\nvar b = (byte(data, 0) as u16)\nreturn ((a << b) as bytes)"));

    /* ---- net.g2 (row-C pin bodies) ---- */
    {
        let pins: Vec<String> = vec![
            "mul_add(100u8, 2u8, 50u8)".to_string(),
            "mul_sub(100u8, 2u8, 50u8)".to_string(),
            "mul_add(200u8, 2u8, 100u8)".to_string(),
            "mul_add(200u8, 2u8, 100u16)".to_string(),
            "mul_sub(200u8, 2u8, 255u8)".to_string(),
            "mul_add(65535u16, 65535u16, 1u32)".to_string(),
            "mul_sub(4000000000u32, 2u32, 1u32)".to_string(),
            "mul_add(3000000000u64, 3u64, 5000000000u64)".to_string(),
            "mul_sub(3000000000u64, 3u64, 1000000000u64)".to_string(),
            format!("mul_add({MAX_U64}u64, {MAX_U64}u64, 0u64)"),
            format!("mul_sub({MAX_U64}u64, {MAX_U64}u64, 1u64)"),
            format!("mul_add(4294967295u64, 4294967295u64, {MAX_U64}u64)"),
        ];
        v.push(case("g2.muladdsub", "net.g2", pin_body(&pins)));
    }
    {
        let pins: Vec<String> = vec![
            "mul_div(7u32, 5u32, 8u32)".to_string(),
            "mul_div_up(7u32, 5u32, 8u32)".to_string(),
            "mul_div(6u32, 4u32, 8u32)".to_string(),
            "mul_div(255u8, 255u8, 2u8)".to_string(),
            "mul_div(255u8, 255u8, 2u16)".to_string(),
            "mul_div_up(255u8, 255u8, 2u16)".to_string(),
            "mul_div(200u8, 200u8, 1u8)".to_string(),
            "mul_div_up(200u8, 200u8, 1u8)".to_string(),
            "mul_div(5u32, 3u32, 0u32)".to_string(),
            "mul_div_up(5u32, 3u32, 0u32)".to_string(),
            "mul_div(4294967295u64, 4294967295u64, 2u64)".to_string(),
            "mul_div_up(4294967295u64, 4294967295u64, 2u64)".to_string(),
            format!("mul_div({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"),
            format!("mul_div_up({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"),
            "mul_div(0u64, 18446744073709551615u64, 7u64)".to_string(),
            format!("mul_div({MAX_U64}u64, {MAX_U64}u64, 1u64)"),
            format!("mul_div_up({MAX_U64}u64, {MAX_U64}u64, 1u64)"),
            format!("mul_div({MAX_U64}u64, 5u64, 0u64)"),
            "mul_div(3u64, 2u64, 100u64)".to_string(),
            "mul_div_up(3u64, 2u64, 100u64)".to_string(),
            format!("mul_div({MAX_U64}u64, 5u64, {MAX_U64}u64)"),
        ];
        v.push(case("g2.muldiv", "net.g2", pin_body(&pins)));
    }
    {
        let pins: Vec<String> = vec![
            "add_mod(5u64, 7u64, 10u64)".to_string(),
            "mul_mod(11u64, 13u64, 17u64)".to_string(),
            format!("add_mod({MAX_U64}u64, 1u64, {MAX_U64}u64)"),
            format!("add_mod({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"),
            format!("mul_mod({MAX_U64}u64, {MAX_U64}u64, {MAX_U64}u64)"),
            "mul_mod(4294967296u64, 4294967296u64, 4294967295u64)".to_string(),
            "add_mod(5u64, 3u64, 0u64)".to_string(),
            "mul_mod(5u64, 3u64, 0u64)".to_string(),
            "add_mod(5u64, 3u64, 1u64)".to_string(),
            "mul_mod(5u64, 3u64, 1u64)".to_string(),
            "add_mod(200u8, 200u8, 256u16)".to_string(),
            "mul_mod(250u8, 250u8, 251u8)".to_string(),
            "mul_mod(65535u16, 65535u16, 65537u32)".to_string(),
            "add_mod(255u8, 4294967296u64, 65537u64)".to_string(),
            "mul_mod(65535u16, 4294967295u32, 65537u64)".to_string(),
        ];
        v.push(case("g2.modarith64", "net.g2", pin_body(&pins)));
    }
    {
        let pins: Vec<String> = vec![
            format!("add_mod({BIG1}u128, {BIG2}u128, {BIGM}u128)"),
            format!("mul_mod({BIG1}u128, {BIG2}u128, {BIGM}u128)"),
            format!("add_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128)"),
            format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U128}u128)"),
            format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, {MAX_U64}u128)"),
            format!("add_mod({BIG1}u128, {BIG2}u128, 0u128)"),
            format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, 1u128)"),
            format!("mul_mod({BIG1}u128, 12345u64, {BIGM}u128)"),
            format!("mul_mod({MAX_U128}u128, {MAX_U128}u128, {BIGM}u128)"),
        ];
        v.push(case("g2.modarith128", "net.g2", pin_body(&pins)));
    }

    /* ---- net.g3 (fin catalog) ---- */
    {
        let g = |v: &mut Vec<Case>, name: &'static str, body: &str| {
            v.push(case(name, "net.g3", body));
        };
        g(&mut v, "g3.fin2.div_exact.exact", "return div_exact(8u64, 2u64) as bytes");
        g(&mut v, "g3.fin2.div_exact.inexact", "return div_exact(7u64, 2u64) as bytes");
        g(&mut v, "g3.fin2.div_floor", "return div_floor(7u64, 2u64) as bytes");
        g(&mut v, "g3.fin2.div_half_up", "return div_half_up(5u64, 2u64) as bytes");
        g(&mut v, "g3.fin2.sat_add", "return sat_add(200u8, 100u8) as bytes");
        g(&mut v, "g3.fin2.sat_sub", "return sat_sub(100u8, 200u8) as bytes");
        g(&mut v, "g3.fin2.div.zero", "return div_floor(7u64, 0u64) as bytes");
        g(&mut v, "g3.fin3.mul_div_exact.exact", "return mul_div_exact(10u64, 4u64, 5u64) as bytes");
        g(&mut v, "g3.fin3.mul_div_exact.inexact", "return mul_div_exact(10u64, 4u64, 3u64) as bytes");
        g(&mut v, "g3.fin3.mul_div_floor", "return mul_div_floor(10u64, 4u64, 3u64) as bytes");
        g(&mut v, "g3.fin3.mul_div_ceil", "return mul_div_ceil(10u64, 4u64, 3u64) as bytes");
        g(&mut v, "g3.fin3.scaled_add_floor", "return scaled_add_floor(100u64, 3u64, 2u64) as bytes");
        g(&mut v, "g3.fin3.scaled_sub_floor", "return scaled_sub_floor(100u64, 1u64, 2u64) as bytes");
        g(&mut v, "g3.fin3.u64_mad", "return u64_mad(100u64, 3u64, 10u64) as bytes");
        g(&mut v, "g3.fin3.u64_mad.u8-ovf", "return u64_mad(250u8, 3u64, 10u64) as bytes");
        g(&mut v, "g3.fin3.u64_mad.u16", "return u64_mad(5u16, 3u64, 10u64) as bytes");
        g(&mut v, "g3.fin4.mul_sub_div.neg", "return mul_sub_div_floor(5u64, 4u64, 100u64, 9u64) as bytes");
        g(&mut v, "g3.finp3.abs_diff_lte.true", "return (abs_diff_lte(5u64, 9u64, 4u64) as u64) as bytes");
        g(&mut v, "g3.finp3.abs_diff_lte.false", "return (abs_diff_lte(5u64, 9u64, 3u64) as u64) as bytes");
        g(&mut v, "g3.finpow3.scaled-square", "return rpow_half_up(3u64, 2u64, 10u64) as bytes");
        g(&mut v, "g3.finpow3.half-up-per-mul", "return rpow_half_up(105u64, 2u64, 100u64) as bytes");
        g(&mut v, "g3.finpow3.scale-zero", "return rpow_half_up(5u64, 3u64, 0u64) as bytes");
        g(&mut v, "g3.finpow3.zero-base", "return rpow_half_up(0u64, 5u64, 10u64) as bytes");
    }

    /* ---- net.g4 (bytes) ---- */
    {
        let omni = format!(
            "var h = buf_left(3, data)\nvar t = buf_right(2, data)\nvar ld = buf_left_drop(1, data)\nvar rd = buf_right_drop(1, data)\nvar c = buf_cut(data, 1, 3)\nvar n = size(c)\nvar lit = \"K7\"\nvar big = 0x{}\nvar emp = \"\"\nvar b0 = byte(data, 0)\nreturn h ++ t ++ ld ++ rd ++ c ++ lit ++ big ++ emp ++ (n as bytes) ++ (b0 as bytes)",
            pattern_hex(300, 0xA3)
        );
        v.push(case("g4.omni", "net.g4", omni));
        v.push(case("g4.slb", "net.g4",
            "var f = buf_cut(data, 0, 1)\nvar l = buf_cut(data, byte(data, 0), 1)\nvar z = buf_left(0, data)\nvar all = buf_cut(data, 0, size(data))\nvar k = buf_right(0, data)\nvar d0 = buf_left_drop(0, data)\nvar d9 = buf_right_drop(0, data)\nreturn f ++ l ++ z ++ all ++ k ++ d0 ++ d9"));
        v.push(case("g4.byte-oob", "net.g4",
            "var v = byte(data, 42)\nvar w = byte(data, 90)\nreturn (v as bytes) ++ (w as bytes)"));
        v.push(case("g4.cut7", "net.g4", "var c = buf_cut(data, 7, 4)\nreturn c"));
        v.push(case("g4.drop11", "net.g4",
            "var a = buf_left_drop(11, data)\nvar b = buf_right_drop(11, data)\nreturn a ++ b"));
        let lit = format!(
            "var e = \"\"\nvar s = \"K7\"\nvar l = 0x{}\nreturn s ++ l ++ e ++ s ++ (l ++ s)",
            pattern_hex(300, 0x5A)
        );
        v.push(case("g4.lit", "net.g4", lit));
        v.push(case("g4.wide", "net.g4",
            "var a = buf_cut(data, 0, 300)\nreturn a ++ a ++ a"));
        v.push(case("g4.cmp", "net.g4",
            "var eq = (buf_left(2, data) == buf_left(2, data))\nvar ne = (buf_left(1, data) != buf_right(1, data))\nvar eq2 = (data == buf_cut(data, 0, size(data)))\nreturn ((eq as u64) as bytes) ++ ((ne as u64) as bytes) ++ ((eq2 as u64) as bytes)"));
        v.push(case("g4.read", "net.g4",
            "var i = byte(data, byte(data, 0))\nvar c = buf_cut(data, 1, byte(data, byte(data, 1)))\nvar s = size(data)\nvar l = size(buf_cut(data, 0, byte(data, 2)))\nreturn (i as bytes) ++ c ++ (s as bytes) ++ (l as bytes)"));
        v.push(case("g4.nil.size", "net.g4",
            "if byte(data, 0) == 1u64 { var u = buf_left(2, data) }\nvar r = size(local_x(1))\nreturn (r as bytes)"));
        v.push(case("g4.nil.byte", "net.g4",
            "if byte(data, 0) == 1u64 { var u = buf_left(2, data) }\nvar r = byte(local_x(1), 0)\nreturn (r as bytes)"));
        v.push(case("g4.nil.cbytes", "net.g4",
            "if byte(data, 0) == 1u64 { var u = buf_left(2, data) }\nvar r = (local_x(1) as bytes)\nreturn r"));
        v.push(case("g4.choose", "net.g4",
            "var r = if byte(data, 0) == 1u64 { buf_left(1, data) } else { buf_right(1, data) }\nreturn r"));
    }

    /* ---- net.g5 (control) ---- */
    v.push(case("g5.if.stmt", "net.g5",
        "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 10u64) {\ns = a * 2u64\n}\nif (a > 20u64) {\ns = s + 1u64\n} else {\ns = s + 2u64\n}\nreturn (s as u64) as bytes"));
    v.push(case("g5.if.value", "net.g5",
        "var a = (byte(data, 0) as u64)\nvar r = if (a > 10u64) { a * 2u64 } else { a + 1u64 }\nreturn (r as u64) as bytes"));
    v.push(case("g5.if.nest", "net.g5",
        "var a = (byte(data, 0) as u64)\nvar s = 0u64\nif (a > 1u64) {\nif (a > 2u64) {\ns += 1u64\n}\n}\nif (a > 3u64) {\nif (a > 4u64) {\nif (a > 5u64) {\nif (a > 6u64) {\ns += 2u64\n}\n}\n}\n}\nif (a > 7u64) {\nif (a > 8u64) {\nif (a > 9u64) {\nif (a > 10u64) {\nif (a > 11u64) {\nif (a > 12u64) {\nif (a > 13u64) {\nif (a > 14u64) {\ns += 4u64\n}\n}\n}\n}\n}\n}\n}\n}\nreturn (s as u64) as bytes"));
    v.push(case("g5.choose.ternary", "net.g5",
        "var a = (byte(data, 0) as u64)\nvar r = (a > 10u64) ? (a * 2u64) : (a + 7u64)\nreturn (r as u64) as bytes"));
    v.push(case("g5.choose.builtin", "net.g5",
        "var a = (byte(data, 0) as u64)\nvar r = choose((a > 10u64), (a * 3u64), (a + 5u64))\nreturn (r as u64) as bytes"));
    v.push(case("g5.choose.eager", "net.g5",
        "var a = (byte(data, 0) as u64)\nvar z = (a - a)\nvar r = choose((a > 0u64), (7u64 / z), 42u64)\nreturn (r as u64) as bytes"));
    v.push(case("g5.while.break", "net.g5",
        "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\nif (i == 4u64) {\nbreak\n}\ns += i\ni += 1u64\n}\nreturn (s as u64) as bytes"));
    v.push(case("g5.while.cont", "net.g5",
        "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\ni += 1u64\nif ((i % 2u64) == 0u64) {\ncontinue\n}\ns += i\n}\nreturn (s as u64) as bytes"));
    v.push(case("g5.while.trap", "net.g5",
        "var n = (byte(data, 0) as u64)\nvar i = 0u64\nvar s = 0u64\nwhile (i < n) {\ni += 1u64\nif (i == 3u64) {\ns += (9u64 / (i - i))\n}\ns += 1u64\n}\nreturn (s as u64) as bytes"));
    v.push(case("g5.while.breakonly", "net.g5",
        "var n = (byte(data, 0) as u64)\nvar i = 0u64\nwhile (n == n) {\nif (i >= n) {\nbreak\n}\ni += 1u64\n}\nreturn (i as u64) as bytes"));
    v.push(case("g5.while.count", "net.g5",
        "var n = (byte(data, 0) as u64)\nvar i = 0u64\nwhile (i != n) {\ni += 1u64\n}\nreturn (i as u64) as bytes"));

    /* ---- net.g6 (types) ---- */
    const A_V1: &str = "RtqvBaudCALQXgZRiRUT8kuYCsjxPFk8P";
    v.push(case("g6.is_pred", "net.g6",
        "var a = (byte(data, 0) as u64)\nvar r = 0u64\nif (a is nil) { r += 1u64 }\nif (a is u64) { r += 2u64 }\nif (a is u8) { r += 4u64 }\nif (a is bool) { r += 8u64 }\nif (a is bytes) { r += 16u64 }\nif (a is address) { r += 32u64 }\nif (a is list) { r += 64u64 }\nif (a is map) { r += 128u64 }\nvar nb = ((data is not bytes) as u64)\nvar lb = ((data is list) as u64)\nvar mb = ((data is map) as u64)\nreturn (r + nb * 256u64 + lb * 512u64 + mb * 1024u64) as bytes"));
    v.push(case("g6.tid", "net.g6",
        "var w = (byte(data, 0) % 5u64)\nvar b = byte(data, 1)\nvar t = 0u8\nif (w == 0u64) { t = type_id(b as u8) }\nif (w == 1u64) { t = type_id(b as u16) }\nif (w == 2u64) { t = type_id(b as u32) }\nif (w == 3u64) { t = type_id(b as u64) }\nif (w == 4u64) { t = type_id(b as u128) }\nreturn ((t as u64) + (type_id(data) as u64) * 16u64 + (type_id(b < b) as u64) * 256u64) as bytes"));
    v.push(case("g6.logic", "net.g6",
        "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar r = 0u64\nif (a == b) { r += 1u64 }\nif (a != b) { r += 2u64 }\nif (a < b) { r += 4u64 }\nif (a > b) { r += 8u64 }\nif (a <= b) { r += 16u64 }\nif (a >= b) { r += 32u64 }\nif (a > 1u64) && (b > 1u64) { r += 64u64 }\nif (a == 0u64) || (b == 0u64) { r += 128u64 }\nvar q = (!(a > b))\nif q { r += 256u64 }\nreturn (r) as bytes"));
    v.push(case("g6.logic8", "net.g6",
        "var a = byte(data, 0)\nvar b = byte(data, 1)\nvar r = 0u64\nif (a == b) { r += 1u64 }\nif (a < b) { r += 2u64 }\nif (a > b) { r += 4u64 }\nreturn (r) as bytes"));
    v.push(case("g6.nilcmp", "net.g6",
        "var q = (byte(data, 0) as u64)\nif (q == 1u64) { var n = 5u64 }\nvar r = (n == 5u64)\nreturn (r as u64) as bytes"));
    v.push(case("g6.asbool.u64", "net.g6",
        "var a = (byte(data, 0) as u64)\nreturn ((a as bool) as u64) as bytes"));
    v.push(case("g6.asbool.bytes", "net.g6",
        "return ((data as bool) as u64) as bytes"));
    v.push(case("g6.asu8", "net.g6", "return (data as u8) as bytes"));
    v.push(case("g6.asu64", "net.g6", "return (data as u64) as bytes"));
    v.push(case("g6.multiwidth", "net.g6",
        "var x = byte(data, 0)\nvar r = ((x as u8) as bytes) ++ ((x as u16) as bytes) ++ ((x as u32) as bytes) ++ ((x as u64) as bytes) ++ ((x as u128) as bytes) ++ ((x < x) as bytes)\nreturn data ++ r"));
    v.push(case("g6.asaddr", "net.g6",
        "var a = (data as address)\nreturn (a as bytes)"));
    v.push(case("g6.cast8", "net.g6",
        "var a = (byte(data, 0) as u64)\nvar r = ((a * 100u64) as u8)\nreturn (r as u64) as bytes"));
    v.push(case("g6.nilcbytes", "net.g6",
        "var q = (byte(data, 0) as u64)\nif (q == 1u64) { var n = 5u64 }\nreturn (n) as bytes"));
    v.push(case("g6.addr.literal", "net.g6",
        format!("var a = {A_V1}\nreturn (a as bytes)")));
    v.push(case("g6.addr.eq.same", "net.g6",
        format!("var r = ({A_V1} == {A_V1})\nreturn ((r as u64)) as bytes")));
    v.push(case("g6.addr.eq.len20", "net.g6",
        format!("var r = (data == {A_V1})\nreturn ((r as u64)) as bytes")));

    /* ---- net.g7 (lists) ---- */
    {
        let g = |v: &mut Vec<Case>, name: &'static str, body: &str| {
            v.push(case(name, "net.g7", body));
        };
        g(&mut v, "g7.create", "var l = [1u64, 2u64, 3u64]\nvar i = (byte(data, 0) as u64) % 3u64\nvar r = l[i] + (length(l) as u64)\nreturn (r as u64) as bytes");
        g(&mut v, "g7.append.frame", "var l = []\nvar i = 0u64\nwhile i < 5u64 {\nappend(l, i * 11u64)\ni += 1u64\n}\nvar r = (length(l) as u64) * 100u64 + l[0] + l[4]\nreturn (r as u64) as bytes");
        g(&mut v, "g7.clear", "var l = [1u64, 2u64, 3u64]\nclear(l)\nvar l0 = (length(l) as u64)\nappend(l, 9u64)\nappend(l, 8u64)\nvar r = l0 * 10u64 + (length(l) as u64)\nreturn (r as u64) as bytes");
        g(&mut v, "g7.itemset", "var l = [1u64, 2u64, 3u64]\nl[1] = 9u64\nitem_set(l, 2u64, 7u64)\nvar r = (l[0] * 10u64 + l[1]) * 10u64 + l[2]\nreturn (r as u64) as bytes");
        g(&mut v, "g7.haskey", "var w = [4u64, 5u64]\nappend(w, 6u64)\nvar r = (has_key(w, 1u64) as u64) * 10u64 + (has_key(w, 9u64) as u64)\nreturn (r as u64) as bytes");
        g(&mut v, "g7.oob.read", "var l = [1u64, 2u64]\nvar i = (byte(data, 0) as u64) + 2u64\nvar r = l[i]\nreturn (r as u64) as bytes");
        g(&mut v, "g7.idx72.itemget", "var l = [1u64, 2u64]\nvar i = (byte(data, 0) as u64) * 4294967296u64\nvar r = l[i]\nreturn (r as u64) as bytes");
        g(&mut v, "g7.cap.append129", "var l = []\nvar i = 0u64\nwhile i < 128u64 {\nappend(l, i)\ni += 1u64\n}\nappend(l, 5u64)\nreturn 0u64 as bytes");
        g(&mut v, "g7.cap.exact128", "var l = []\nvar i = 0u64\nwhile i < 128u64 {\nappend(l, i)\ni += 1u64\n}\nreturn (length(l) as u64) as bytes");
        g(&mut v, "g7.nested.literal", "var l = [1u64, 2u64]\nvar m = [l]\nreturn 0u64 as bytes");
        g(&mut v, "g7.nested.append", "var a = [1u64]\nvar b = [2u64]\nappend(a, b)\nreturn 0u64 as bytes");
        g(&mut v, "g7.take.first-last", "var l = [10u64, 20u64, 30u64]\nvar a = (take_first(l) as u64)\nvar b = (take_last(l) as u64)\nvar r = a * 100u64 + b\nreturn (r as u64) as bytes");
        g(&mut v, "g7.take.empty", "var l = []\nvar a = (take_first(l) as u64)\nreturn (a as u64) as bytes");
        g(&mut v, "g7.take.clone-iso", "var l = [1u64, 2u64]\nvar m = clone(l)\nvar a = (take_first(m) as u64)\nvar r = (length(l) as u64) * 10u64 + (length(m) as u64)\nreturn (r as u64) as bytes");
        g(&mut v, "g7.insert.middle", "var l = [1u64, 2u64, 3u64]\ninsert(l, 1u64, 99u64)\nvar r = ((l[0] * 1000u64 + l[1]) * 1000u64 + l[2]) * 10u64 + (length(l) as u64)\nreturn (r as u64) as bytes");
        g(&mut v, "g7.insert.pastlen", "var l = [1u64, 2u64]\ninsert(l, 3u64, 9u64)\nreturn 0u64 as bytes");
        g(&mut v, "g7.remove.middle", "var l = [1u64, 2u64, 3u64, 4u64]\nremove(l, 1u64)\nvar r = ((l[0] * 100u64 + l[1]) * 100u64 + l[2]) * 10u64 + (length(l) as u64)\nreturn (r as u64) as bytes");
        g(&mut v, "g7.merge.basic", "var a = [1u64, 2u64]\nvar b = [3u64, 4u64]\nmerge(a, b)\nvar r = ((length(a) as u64) * 100u64 + a[2]) * 100u64 + a[3]\nreturn (r as u64) as bytes");
        g(&mut v, "g7.merge.samerc", "var a = [1u64, 2u64]\nmerge(a, a)\nreturn 0u64 as bytes");
        g(&mut v, "g7.keys.on-list", "var l = [1u64, 2u64]\nvar m = keys(l)\nreturn 0u64 as bytes");
        g(&mut v, "g7.remove.to-empty", "var l = [4u64]\nremove(l, 0u64)\nvar a = (take_first(l) as u64)\nreturn (a as u64) as bytes");
    }

    /* ---- net.g8 (maps) ---- */
    {
        let g = |v: &mut Vec<Case>, name: &'static str, body: &str| {
            v.push(case(name, "net.g8", body));
        };
        g(&mut v, "g8.newmap.len", "var m = new_map()\nreturn length(m) as bytes");
        g(&mut v, "g8.map1.len", "var m = map{1u64 : 2u64}\nreturn length(m) as bytes");
        g(&mut v, "g8.insert.itemget", "var m = new_map()\ninsert(m, 7u64, 40u64)\nreturn (m[7u64] as u64) as bytes");
        g(&mut v, "g8.overwrite", "var m = map{1u64 : 2u64}\ninsert(m, 1u64, 9u64)\nreturn ((m[1u64] as u64) * 10u64 + length(m)) as bytes");
        g(&mut v, "g8.zero.key", "var m = new_map()\ninsert(m, 0u64, 5u64)\nreturn (m[0u64] as u64) as bytes");
        g(&mut v, "g8.itemset.overwrite", "var m = map{1u64 : 2u64, 2u64 : 3u64}\nitem_set(m, 2u64, 9u64)\nreturn ((m[2u64] as u64) * 10u64 + length(m)) as bytes");
        g(&mut v, "g8.index.assign", "var m = map{1u64 : 2u64}\nm[1u64] = 9u64\nreturn (m[1u64] as u64) as bytes");
        g(&mut v, "g8.keys.sorted", "var m = new_map()\ninsert(m, 3u64, 30u64)\ninsert(m, 1u64, 10u64)\ninsert(m, 2u64, 20u64)\nvar ks = keys(m)\nreturn ((ks[0] as u64) * 100u64 + (ks[2] as u64)) as bytes");
        g(&mut v, "g8.values.order", "var m = new_map()\ninsert(m, 3u64, 30u64)\ninsert(m, 1u64, 10u64)\ninsert(m, 2u64, 20u64)\nvar vs = values(m)\nreturn ((vs[0] as u64) * 100u64 + (vs[1] as u64)) as bytes");
        g(&mut v, "g8.remove.reinsert", "var m = map{1u64 : 10u64, 2u64 : 20u64}\nremove(m, 1u64)\ninsert(m, 3u64, 30u64)\nreturn ((m[2u64] as u64) * 100u64 + (m[3u64] as u64)) as bytes");
        g(&mut v, "g8.merge.values", "var a = map{1u64 : 2u64}\nvar b = map{3u64 : 4u64}\nmerge(a, b)\nreturn ((a[1u64] as u64) * 10u64 + (a[3u64] as u64)) as bytes");
        g(&mut v, "g8.alias.b", "var a = new_map()\nvar b = a\ninsert(b, 1u64, 2u64)\nreturn (a[1u64] as u64) as bytes");
        g(&mut v, "g8.missing.get", "var m = map{1u64 : 2u64}\nvar r = m[9u64]\nreturn r as bytes");
        g(&mut v, "g8.missing.remove", "var m = map{1u64 : 2u64}\nremove(m, 9u64)\nreturn 0u64 as bytes");
        g(&mut v, "g8.missing.set", "var m = map{1u64 : 2u64}\nm[9u64] = 3u64\nreturn 0u64 as bytes");
        g(&mut v, "g8.boolkey", "var m = new_map()\ninsert(m, (1u64 == 1u64), 5u64)\nreturn 0u64 as bytes");
        g(&mut v, "g8.emptybyteskey", "var m = new_map()\nvar e = buf_left(0, data)\ninsert(m, e, 5u64)\nreturn 0u64 as bytes");
        g(&mut v, "g8.dup.pack", "var m = map{1u64 : 2u64, 1u64 : 3u64}\nreturn length(m) as bytes");
        g(&mut v, "g8.dup.merge", "var a = map{1u64 : 2u64}\nvar b = map{1u64 : 3u64}\nmerge(a, b)\nreturn length(a) as bytes");
        g(&mut v, "g8.selfmerge", "var a = map{1u64 : 2u64}\nmerge(a, a)\nreturn length(a) as bytes");
        g(&mut v, "g8.capacity", "var m = new_map()\nvar i = 0u64\nwhile i < 129u64 { insert(m, i, i)\ni += 1 }\nreturn length(m) as bytes");
        g(&mut v, "g8.canon.collide", "var m = new_map()\ninsert(m, 1u64, 5u64)\ninsert(m, buf_left(1, data), 6u64)\nreturn ((m[1u64] as u64) * 10u64 + length(m)) as bytes");
        g(&mut v, "g8.canon.zeroext", "var m = new_map()\ninsert(m, 65536u32, 9u64)\nreturn (m[65536u64] as u64) as bytes");
        g(&mut v, "g8.byteskey.sort", "var m = new_map()\ninsert(m, buf_left(2, data), 1u64)\ninsert(m, buf_right(1, data), 2u64)\nvar ks = keys(m)\nreturn ks[0]");
        g(&mut v, "g8.bytesvalue.itemget", "var m = new_map()\ninsert(m, 1u64, buf_left(3, data))\nreturn m[1u64]");
        g(&mut v, "g8.loop.sum", "var m = new_map()\nvar i = 0u64\nwhile i < 10u64 { insert(m, i, i)\ni += 1u64 }\nvar j = 0u64\nvar s = 0u64\nwhile j < 10u64 { s += m[j]\nj += 1u64 }\nreturn s as bytes");
    }

    /* ---- net.g9 (assignment paths) ---- */
    {
        let g = |v: &mut Vec<Case>, name: &'static str, body: &str| {
            v.push(case(name, "net.g9", body));
        };
        g(&mut v, "g9.byte_reads", "var a = (byte(data, 0) as u64)\nvar b = (byte(data, 1) as u64)\nvar c = (byte(data, 2) as u64)\nvar d = (byte(data, 3) as u64)\nreturn (((a * 256u64 + b) * 256u64 + c) * 256u64 + d) as bytes");
        g(&mut v, "g9.slice_family", "var a = (byte(data, 0) as u64)\nvar c1 = buf_cut(data, 1u64, 2u64)\nvar c2 = buf_cut(data, 0u64, 1u64)\nvar l = buf_left(2, data)\nvar r = buf_right(2, data)\nvar ld = buf_left_drop(1, data)\nvar rd = buf_right_drop(1, data)\nvar r1 = (a * 1000u64 + (size(c1) as u64)) * 1000u64 + (size(c2) as u64)\nvar r2 = ((size(l) as u64) * 1000u64 + (size(r) as u64)) * 1000u64 + (size(ld) as u64) * 1000u64 + (size(rd) as u64)\nreturn (r1 * 1000000u64 + r2) as bytes");
        g(&mut v, "g9.data_rebind", "data = buf_right(1, data)\nreturn (byte(data, 0) as u64) as bytes");
        g(&mut v, "g9.rebind.both", "var a = (byte(data, 0) as u64)\ndata = 0x0506\nvar b = (byte(data, 0) as u64)\nreturn (((a * 100u64) + b) as u64) as bytes");
        g(&mut v, "g9.rebind.twice", "data = buf_left(2, data)\ndata = data ++ 0xff\nreturn data");
        let big = "ab".repeat(300);
        let buf_pbuf = format!(
            "var e = \"\"\nvar s = \"abcdefgh\"\nvar big = 0x{big}\nvar b = \"aaaaaaaa\"\nvar o = (byte(data, 0) as u64) % 8u64\nvar x = buf_set(b, o, 17u8)\nvar r1 = ((size(e) as u64) * 1000000u64 + (size(s) as u64)) * 1000u64 + (size(big) as u64)\nvar r2 = (byte(x, o) as u64) * 10u64 + (byte(b, o) as u64)\nreturn (r1 * 1000u64 + r2) as bytes"
        );
        g(&mut v, "g9.buf_pbuf", &buf_pbuf);
        g(&mut v, "g9.bufset.res", "var b = \"abcdefgh\"\nvar x = buf_set(b, 2u64, 85u8)\nvar r = (byte(x, 2) as u64) * 10u64 + (byte(b, 2) as u64)\nreturn (r as u64) as bytes");
        g(&mut v, "g9.compound.rhsfirst", "var x $70 = (byte(data, 0) as u64)\nx += (x * 10u64)\nreturn x as bytes");
        g(&mut v, "g9.compound.u128.div", "var x $64 = ((byte(data, 0) as u128) * 18446744073709551616u128)\nx /= 3u128\nx -= 1u128\nreturn x as bytes");
        g(&mut v, "g9.compound.u128.add", "var x $64 = ((byte(data, 0) as u128) * 18446744073709551616u128)\nx += 5u128\nreturn x as bytes");
        g(&mut v, "g9.compound.div0", "var x $64 = (byte(data, 0) as u64)\nx /= 0u64\nreturn x as bytes");
        g(&mut v, "g9.compound.addovf", "var x $64 = 18446744073709551615u64\nx += 1u64\nreturn x as bytes");
        g(&mut v, "g9.compound.subunder", "var x $64 = ((byte(data, 0) as u64) % 3u64)\nx -= 10u64\nreturn x as bytes");
        g(&mut v, "g9.compound.mulovf", "var x $64 = 18446744073709551615u64\nx *= 2u64\nreturn x as bytes");
        g(&mut v, "g9.itemset.oob", "var l = [7u64, 8u64]\nvar i = (byte(data, 0) as u64) % 4u64\nl[i] = 5u64\nvar r = l[0] + l[1]\nreturn (r as u64) as bytes");
        g(&mut v, "g9.itemset.u32", "var l = [7u64, 8u64]\nvar i = 4294967296u64 + ((byte(data, 0) as u64) % 2u64)\nl[i] = 5u64\nreturn 0u64 as bytes");
        g(&mut v, "g9.itemset.rhstrap", "var l = [7u64, 8u64]\nl[0] += (1u64 / (byte(data, 1) - byte(data, 1)))\nreturn 0u64 as bytes");
        g(&mut v, "g9.itemset.addovf", "var l = [18446744073709551615u64, 2u64]\nl[0] += (byte(data, 0) as u64) + 1u64\nvar r = l[0] + l[1]\nreturn (r as u64) as bytes");
        g(&mut v, "g9.bufset.nil", "var q = 0u64\nif q == 1u64 { var t = \"ab\" }\nbuf_set(t, 0u64, 5u8)\nreturn 0u64 as bytes");
        g(&mut v, "g9.map.missing.set", "var m = map{1u64 : 2u64}\nm[9u64] = 3u64\nreturn 0u64 as bytes");
        g(&mut v, "g9.map.missing.get", "var m = map{1u64 : 2u64}\nvar r = m[9u64]\nreturn r as bytes");
    }

    /* ---- net.g10 (algorithm bodies) ---- */
    v.push(case("g10.median", "net.g10", median_fitsh()));
    v.push(case("g10.settle", "net.g10", settle_fitsh()));
    v.push(case("g10.sha256r4", "net.g10", sha256_reduced_fitsh(4)));
    v.push(case("g10.keccakr1", "net.g10", keccak_r1_fitsh()));
    v.push(case("g10.uni_v2", "net.g10", uni_v2_fitsh()));
    v.push(case("g10.black_scholes", "net.g10", black_scholes_fitsh()));
    v.push(case("g10.rpow", "net.g10", rpow_fitsh()));

    /* ---- net.g17 (tier2 production crops) ---- */
    v.push(case("g17.dict_walk", "net.g17", G17_DICT_WALK_BODY));
    v.push(case("g17.twap", "net.g17", G17_TWAP_BODY));
    v.push(case("g17.cpmm_quote", "net.g17", G17_CPMM_QUOTE_BODY));

    /* ---- prod (tracked production kernels) ---- */
    v.push(case("prod.score_sum", "prod",
        include_str!("../../vm/tests/fitsh_kernels_score_sum.inc")));
    v.push(case("prod.hacd200", "prod",
        include_str!("../../vm/tests/fitsh_kernels_hacd200.inc")));
    v.push(case("prod.sha256_single_block", "prod", sha256_single_block_src()));

    v
}

/* ==================================================================== */
/* Measurement                                                          */
/* ==================================================================== */

#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    group: &'static str,
    mode: &'static str,
    insts: usize,
    code_bytes: usize,
    cycles: u64,
    locals: usize,
    pool_bytes: usize,
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rv64_size_baseline")
}

fn tsv_line(r: Row) -> String {
    format!("{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", r.name, r.group, r.mode, r.insts, r.code_bytes, r.cycles, r.locals, r.pool_bytes)
}

/// Parse a stored baseline.tsv into (name, mode) → Row (TOTAL rows skipped).
fn parse_baseline(text: &str) -> HashMap<(String, String), Row> {
    let mut map = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.starts_with("TOTAL") || line.starts_with("name\t") {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 8 {
            continue;
        }
        let row = Row {
            name: Box::leak(f[0].to_string().into_boxed_str()),
            group: Box::leak(f[1].to_string().into_boxed_str()),
            mode: Box::leak(f[2].to_string().into_boxed_str()),
            insts: f[3].parse().unwrap_or(0),
            code_bytes: f[4].parse().unwrap_or(0),
            cycles: f[5].parse().unwrap_or(0),
            locals: f[6].parse().unwrap_or(0),
            pool_bytes: f[7].parse().unwrap_or(0),
        };
        map.insert((f[0].to_string(), f[2].to_string()), row);
    }
    map
}

#[test]
#[ignore = "size-baseline measurement tool; run with -- --ignored --nocapture (see file header)"]
fn rv64_size_baseline() {
    let cap = SpaceCap::new(0);
    let arena = kernel_arena_size(&cap);
    let input_max = cap.call_data_size as u16;
    let output_max = cap.value_size as u16;
    let cases = corpus();

    // corpus hygiene: names must be unique (they are the delta keys)
    {
        let mut seen = std::collections::HashSet::new();
        for c in &cases {
            assert!(seen.insert(c.name), "duplicate corpus case name: {}", c.name);
        }
    }

    let mut rows: Vec<Row> = Vec::new();
    for c in &cases {
        let ir = { eprintln!("[case] {} ({})", c.name, c.group); body_to_ir(&c.body) };
        for m in MODES {
            // -- determinism self-check: two independent lowerings must be
            //    byte-identical (same IR in, same bytes out)
            let a = lower_once(&ir, m, arena, &cap);
            let b = lower_once(&ir, m, arena, &cap);
            assert_eq!(
                a.code, b.code,
                "[{}::{} / {}] lowering is NOT deterministic (code bytes differ between two runs)",
                c.group, c.name, mode_name(m)
            );
            assert_eq!(
                a.pool, b.pool,
                "[{}::{} / {}] lowering is NOT deterministic (pool bytes differ)",
                c.group, c.name, mode_name(m)
            );
            assert_eq!(a.stats.insts, b.stats.insts, "[{}/{}]", c.name, mode_name(m));
            assert_eq!(a.stats.cycles, b.stats.cycles, "[{}/{}]", c.name, mode_name(m));
            // stats self-consistency
            assert_eq!(a.stats.code_bytes, a.code.len(), "[{}/{}] code_bytes stat", c.name, mode_name(m));

            // -- the packed container must pass consensus admission
            let container: CalcContainer = a
                .pack_segments_profile(&cap, 64, input_max, output_max, profile_byte(m))
                .unwrap_or_else(|e| panic!("[{}/{}] container packs: {e}", c.name, mode_name(m)));
            verify_container_with_cap(&container, &cap)
                .unwrap_or_else(|e| panic!("[{}/{}] container verifies: {e}", c.name, mode_name(m)));

            rows.push(Row {
                name: c.name,
                group: c.group,
                mode: mode_name(m),
                insts: a.stats.insts,
                code_bytes: a.stats.code_bytes,
                cycles: a.stats.cycles,
                locals: a.stats.locals,
                pool_bytes: a.pool.len(),
            });
        }
    }

    /* ---- totals ---- */
    let mut totals: HashMap<&str, (usize, usize, u64, usize, usize)> = HashMap::new(); // mode -> (insts, code, cycles, locals, pool)
    for r in &rows {
        let t = totals.entry(r.mode).or_insert((0, 0, 0, 0, 0));
        t.0 += r.insts;
        t.1 += r.code_bytes;
        t.2 += r.cycles;
        t.3 += r.locals;
        t.4 += r.pool_bytes;
    }
    let mode_of = |i: usize| mode_name(MODES[i]);

    println!("\n== RV64 size baseline: {} cases x 3 modes (arena {} B, admission-verified, determinism-checked) ==",
        cases.len(), arena);
    for i in 0..3 {
        let m = mode_of(i);
        let t = totals[m];
        println!("{m:>9}: insts {:>8}  code_bytes {:>8}  cycles {:>12}  locals(sum) {:>7}  pool_bytes {:>8}",
            t.0, t.1, t.2, t.3, t.4);
    }

    /* ---- per-group subtotals (code_bytes) ---- */
    let mut groups: Vec<(&str, [usize; 3])> = {
        let mut g: Vec<(&str, [usize; 3])> = Vec::new();
        for c in &cases {
            if !g.iter().any(|(n, _)| *n == c.group) {
                g.push((c.group, [0; 3]));
            }
        }
        g
    };
    for r in &rows {
        let mi = MODES.iter().position(|&m| mode_name(m) == r.mode).unwrap();
        if let Some(e) = groups.iter_mut().find(|(n, _)| *n == r.group) {
            e.1[mi] += r.code_bytes;
        }
    }
    println!("\n-- per-group code_bytes subtotals --");
    println!("{:<10} {:>9} {:>9} {:>9}", "group", "Im", "Imc", "ImcRemap");
    for (g, cbs) in &groups {
        println!("{:<10} {:>9} {:>9} {:>9}", g, cbs[0], cbs[1], cbs[2]);
    }

    /* ---- full TSV (the canonical machine-readable output) ---- */
    let mut tsv = String::from("# rv64 size baseline (long format): name\tgroup\tmode\tinsts\tcode_bytes\tcycles\tlocals\tpool_bytes\n");
    for r in &rows {
        tsv.push_str(&tsv_line(r.clone()));
        tsv.push('\n');
    }
    for i in 0..3 {
        let m = mode_of(i);
        let t = totals[m];
        tsv.push_str(&format!("TOTAL\tall\t{m}\t{}\t{}\t{}\t{}\t{}\n", t.0, t.1, t.2, t.3, t.4));
    }
    println!("\n-- TSV --\n{}", tsv);

    /* ---- delta report against the stored baseline (if any) ---- */
    let baseline_path = fixtures_dir().join("baseline.tsv");
    if let Ok(old_text) = std::fs::read_to_string(&baseline_path) {
        let old = parse_baseline(&old_text);
        let mut drift: Vec<(&&str, &str, Row, Row)> = Vec::new(); // (name, mode, old, new)
        let mut added = 0usize;
        let mut removed = 0usize;
        let mut cur_keys = std::collections::HashSet::new();
        for r in &rows {
            let key = (r.name.to_string(), r.mode.to_string());
            cur_keys.insert(key.clone());
            match old.get(&key) {
                None => added += 1,
                Some(o) => {
                    if o.code_bytes != r.code_bytes || o.insts != r.insts || o.cycles != r.cycles {
                        drift.push((&r.name, r.mode, *o, *r));
                    }
                }
            }
        }
        for k in old.keys() {
            if !cur_keys.contains(k) {
                removed += 1;
            }
        }
        println!("-- delta vs stored baseline ({}) --", baseline_path.display());
        println!("   cases changed: {}   added: {}   removed: {}", drift.len(), added, removed);
        if !drift.is_empty() {
            println!("{:<34} {:>9} {:>10} {:>8} {:>9} {:>10} {:>8}",
                "case", "old B", "new B", "d B", "old inst", "new inst", "d inst");
            for (name, mode, o, n) in &drift {
                println!("{:<34} {:>9} {:>10} {:>+8} {:>9} {:>10} {:>+8}  [{}]",
                    format!("{}/{}", name, mode), o.code_bytes, n.code_bytes,
                    n.code_bytes as i64 - o.code_bytes as i64,
                    o.insts, n.insts, n.insts as i64 - o.insts as i64, n.group);
            }
            let mut ot: HashMap<&str, (i64, i64, i64)> = HashMap::new();
            for (_, mode, o, n) in &drift {
                let t = ot.entry(*mode).or_insert((0, 0, 0));
                t.0 += n.code_bytes as i64 - o.code_bytes as i64;
                t.1 += n.insts as i64 - o.insts as i64;
                t.2 += n.cycles as i64 - o.cycles as i64;
            }
            for i in 0..3 {
                let m = mode_of(i);
                if let Some(t) = ot.get(m) {
                    println!("   TOTAL delta {m}: code_bytes {:+}  insts {:+}  cycles {:+}", t.0, t.1, t.2);
                }
            }
        } else if added == 0 && removed == 0 {
            println!("   NO DRIFT: current compiler output matches the stored baseline byte-for-byte on every stat.");
        }
    } else {
        println!("(no stored baseline at {} — set RV64_BASELINE_WRITE=1 to write one)", baseline_path.display());
    }

    /* ---- optional baseline write ---- */
    if std::env::var("RV64_BASELINE_WRITE").map(|v| v == "1").unwrap_or(false) {
        std::fs::create_dir_all(fixtures_dir()).expect("create fixtures dir");
        std::fs::write(fixtures_dir().join("baseline.tsv"), &tsv).expect("write baseline.tsv");
        let md = render_baseline_md(&cases.len(), &rows, &groups, &totals);
        std::fs::write(fixtures_dir().join("baseline.md"), md).expect("write baseline.md");
        println!("wrote {} and baseline.md", baseline_path.display());
    }
}

/* ==================================================================== */
/* baseline.md rendering                                                */
/* ==================================================================== */

fn render_baseline_md(
    cases: &usize,
    rows: &[Row],
    groups: &[(&str, [usize; 3])],
    totals: &HashMap<&str, (usize, usize, u64, usize, usize)>,
) -> String {
    let mut md = String::new();
    md.push_str("# fitsh→RV64 代码体积基线（size baseline）\n\n");
    md.push_str("生成工具：`testkit/tests/rv64_size_baseline.rs`（`#[ignore]` 测量工具，非断言型基线检查）。\n\n");
    md.push_str("重新生成（改编译器后更新基线）：\n\n");
    md.push_str("```\nRV64_BASELINE_WRITE=1 cargo test -p testkit --offline --test rv64_size_baseline -- --ignored --nocapture\n```\n\n");
    md.push_str("对比运行（不写文件，输出与基线的逐用例 delta）：\n\n```\ncargo test -p testkit --offline --test rv64_size_baseline -- --ignored --nocapture\n```\n\n");
    md.push_str(&format!("语料：{} 个 fitsh 用例 × 3 个编译模式（Im / Imc / ImcRemap），共 {} 行统计。\n\n",
        cases, rows.len()));
    md.push_str("测量口径：\n\n");
    md.push_str("- 管线 = 生产内核路径：fitsh 源 → `Tokenizer` → `vm::fitshc::compile_body`（`(data: bytes)` 参数）→ `vm::ir::rv64::lower_body_mode_arena_heap`（arena 按 `Rv64imBodyMeta::default` 预算，即 mem 64 KiB / L1 IO 上限，镜像 `vm/src/fitshc/kernel.rs` 的计算）；\n");
    md.push_str("- 每用例 × 模式编译两次断言字节一致（确定性自检），容器 `pack_segments_profile` + `verify_container_with_cap` 共识准入校验通过；\n");
    md.push_str("- `code_bytes` 只计 RX 代码段（`Rv64Stats.code_bytes`），`pool_bytes` 为只读字面量池大小（单独列，不计入 code_bytes）；`cycles` 为静态指令权重和（`inst_cycles` 重解码求和）。\n\n");

    md.push_str("## 三模式合计\n\n");
    md.push_str("| mode | insts | code_bytes | cycles | locals(sum) | pool_bytes |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|\n");
    let mut grand = [0usize; 3];
    for m in ["Im", "Imc", "ImcRemap"] {
        let t = totals[m];
        md.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n", m, t.0, t.1, t.2, t.3, t.4));
        if m == "Im" { grand[0] = t.1; }
        if m == "Imc" { grand[1] = t.1; }
        if m == "ImcRemap" { grand[2] = t.2 as usize; }
    }
    let im = totals["Im"].1 as f64;
    let imc = totals["Imc"].1 as f64;
    let remap = totals["ImcRemap"].1 as f64;
    md.push_str(&format!(
        "\nImc 相对 Im 压缩 {:.1}%；ImcRemap 相对 Im 压缩 {:.1}%；ImcRemap 相对 Imc 再压 {:.1}%。\n\n",
        (im - imc) / im * 100.0,
        (im - remap) / im * 100.0,
        (imc - remap) / imc * 100.0,
    ));

    md.push_str("## 分组 code_bytes 小计\n\n");
    md.push_str("| group | Im | Imc | ImcRemap |\n|---|---:|---:|---:|\n");
    for (g, cbs) in groups {
        md.push_str(&format!("| {} | {} | {} | {} |\n", g, cbs[0], cbs[1], cbs[2]));
    }
    md.push('\n');

    md.push_str("## 语料构成\n\n");
    md.push_str("- `micro` — 自建覆盖片段：其余组未覆盖的形态（u16 checked 面、被支配常量乘溢出检查合并面、FIN2 SqrtMul（软件 isqrt）、u128 对域四则、item_concat、复合下标赋值 *=、heap 读写、else-if 链）；\n");
    md.push_str("- `vmunit` — `vm/src/ir/rv64/tests/subset_tests.rs` 的 `lowers_core_constructs` 8 个规范化用例；\n");
    md.push_str("- `net.g1..net.g9` — hacdtestnet three_way 战役（g1 算术含 checked 溢出面 / g2 row-C 大数 mul_add、mul_div、add_mod、mul_mod（u64+u128，u256/u128 软除法与 mulmod 例程）/ g3 fin 目录 / g4 bytes / g5 控制流 / g6 类型转换 / g7 列表 / g8 映射 / g9 赋值路径），取自 testkit/tests/calc_aot_differential.rs 的去重 body 集；\n");
    md.push_str("- `net.g10` — fullnodetest g10 算法集（median 排序网络、settle 分账、sha256-r4、keccak-r1、uni_v2、black_scholes、rpow）；\n");
    md.push_str("- `net.g17` — fullnodetest g17 生产裁剪（dict-walk 解析游走、价格流 TWAP、CPMM 报价）；\n");
    md.push_str("- `prod` — 跟踪的生产内核：`vm/tests/fitsh_kernels_score_sum.inc`、`vm/tests/fitsh_kernels_hacd200.inc`、sha256 单块内核（`vm/tests/rv64_hash_kernels.rs` 生成器移植）。\n\n");

    md.push_str("## 每用例明细（长表）\n\n");
    md.push_str("| name | group | mode | insts | code_bytes | cycles | locals | pool_bytes |\n");
    md.push_str("|---|---|---|---:|---:|---:|---:|---:|\n");
    for r in rows {
        md.push_str(&format!("| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            r.name, r.group, r.mode, r.insts, r.code_bytes, r.cycles, r.locals, r.pool_bytes));
    }
    md.push('\n');
    md
}
