#![allow(unused_imports)]

use super::super::*;
use crate::calc::decode::{BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp};
use crate::calc::{CalcFnObj, CalcRunLimits};
use crate::fitshc::compile_body;
use crate::lang::Tokenizer;


    pub(crate) fn compile_with_input_pub(body: &str) -> (IRNodeArray, Vec<u8>) {
        compile(body)
    }


    pub(crate) fn compile(body: &str) -> (IRNodeArray, Vec<u8>) {
        let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
        let (ir, compiled, _) = compile_body(tokens, vec![], &[], &[], false).unwrap();
        match compiled {
            crate::fitshc::CompiledCode::Bytecode(b) => (ir, b),
            _ => unreachable!(),
        }
    }


    pub(crate) fn lower(body: &str) -> Rv64Lowered {
        let (ir, _) = compile(body);
        lower_body(&ir).unwrap()
    }


    pub(crate) fn lower_mode(body: &str, mode: Rv64Mode) -> Rv64Lowered {
        let (ir, _) = compile(body);
        lower_body_mode(&ir, false, mode).unwrap()
    }


    pub(crate) fn decode_stream(code: &[u8]) -> Vec<Inst> {
        let mut out = Vec::new();
        let mut off = 0;
        while off < code.len() {
            let (inst, len) = decode_at(code, off, 0).unwrap();
            out.push(inst);
            off += len as usize;
        }
        out
    }


    pub(crate) fn decode_stream_p(code: &[u8], profile: u8) -> Vec<Inst> {
        let mut out = Vec::new();
        let mut off = 0;
        while off < code.len() {
            let (inst, len) = decode_at(code, off, profile).unwrap();
            out.push(inst);
            off += len as usize;
        }
        out
    }


    pub(crate) fn run_e2e(body: &str, input: &[u8]) -> Vec<u8> {
        run_e2e_mode(body, input, Rv64Mode::Im)
    }


    pub(crate) fn run_e2e_mode(body: &str, input: &[u8], mode: Rv64Mode) -> Vec<u8> {
        let lo = lower_mode(body, mode);
        let cap = SpaceCap::new(0);
        let container = match mode {
            Rv64Mode::Im => lo.pack_segments(&cap, 64, 5120, 1280),
            _ => lo.pack_segments_profile(&cap, 64, 5120, 1280, crate::calc::container::CALC_PROFILE_RV64IMC),
        }
        .unwrap_or_else(|e| panic!("pack failed: {e}"));
        let fno = CalcFnObj::build_v1(container).unwrap();
        let exec =
            fno.run(input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
        match exec.result {
            Ok(out) => out,
            Err(e) => panic!("kernel aborted: {e:?} exit={} cycles={}", exec.exit_code, exec.cycles_used),
        }
    }


    /// HIP5 Diamond Analyzer v0.1.1 scoring, the fitsh version of
    /// `doc/hacdsdefi4000.loc/score_sum.rs`, as one inlined kernel body (the
    /// contract keeps the multi-function form as comments; `self.` calls are
    /// outside the kernel subset). Wire = names[6n] || numbers[8n] ||
    /// visual_genes[10n]; n = size/24. Output = 8-byte BE u64 sum of
    /// score(record). Deviation vs score_sum.rs: malformed sizes are not
    /// trapped (the loop just iterates floor(size/24)).
    pub(crate) const HIP5_FITSH: &str = r#"// five-dimension score, Diamond Analyzer v0.1.1
var n = size(data) / 24
var ne = n * 6
var nue = ne + n * 8
var sum = 0u64
var i = 0u64
while i < n {
    var nb = i * 6
    var xb = ne + i * 8
    var gb = nue + i * 10

    // ---- shape: nibbles 0/1 of the visual gene ----
    var b0 = byte(data, gb)
    var sa = b0 / 16
    var sb = b0 % 16
    var shape = 0u64
    if sa == 0 {
        if sb == 1 { shape = 30 }
        if sb == 2 { shape = 30 }
        if sb == 3 { shape = 50 }
        if sb == 4 { shape = 20 }
        if sb == 5 { shape = 30 }
        if sb == 6 { shape = 30 }
        if sb == 7 { shape = 20 }
        if sb == 8 { shape = 20 }
    }

    var colorish = 0u64
    if shape == 0 {
        // ---- style (Common shape): nibbles 14..17 = bytes 7/8 ----
        var h7 = byte(data, gb + 7)
        var h8 = byte(data, gb + 8)
        var c13 = h7 / 16
        var c14 = h7 % 16
        var c15 = h8 / 16
        var c16 = h8 % 16
        // positional rules, DA order, first hit wins (mutually exclusive)
        var pos = 0u64
        if c13 == c14 && c14 == c15 && c15 == c16 { pos = 250 } else {
        if c13 == c14 && c14 == c15 && c14 != c16 { pos = 20 } else {
        if c13 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c13 == c14 && c14 == c16 && c15 != c13 { pos = 20 } else {
        if c14 == c15 && c15 == c16 && c14 != c13 { pos = 20 } else {
        if c16 == c15 && c13 == c14 { pos = 20 } else {
        if c16 == c14 && c13 == c15 { pos = 20 } else {
        if c15 * 16 + c13 == c14 * 16 + c16 { pos = 20 } else {
        if c13 == c14 && c15 != c16 { pos = 3 } else {
        if c15 == c16 && c14 != c13 { pos = 3 }
        }}}}}}}}}
        // bitmap + dark/light flags over nibbles 2..18 in one pass
        var bitmap = 0u64
        var alldark = 1u64
        var alllight = 1u64
        var k = 2u64
        while k < 18 {
            var hv = byte(data, gb + k / 2)
            if k % 2 == 0 { hv = hv / 16 } else { hv = hv % 16 }
            bitmap = bitmap | (1 << hv)
            if hv > 4 { alldark = 0 }
            if hv <= 4 { alllight = 0 }
            k += 1
        }
        // popcount16(bitmap)
        var px = bitmap & 65535
        px = px - ((px >> 1) & 21845)
        px = (px & 13107) + ((px >> 2) & 13107)
        px = (px + (px >> 4)) & 3855
        px = px + (px >> 8)
        var distinct = px & 31
        var cnt = 0u64
        if distinct == 4 { if 200 > cnt { cnt = 200 } }
        if distinct == 5 { if 100 > cnt { cnt = 100 } }
        if distinct == 6 { if 50 > cnt { cnt = 50 } }
        if distinct == 7 { if 20 > cnt { cnt = 20 } }
        if distinct == 14 { if 30 > cnt { cnt = 30 } }
        if distinct == 15 { if 100 > cnt { cnt = 100 } }
        if distinct == 16 { if 200 > cnt { cnt = 200 } }
        var dl = 0u64
        if alldark == 1 { if 20 > dl { dl = 20 } }
        if alllight == 1 { if 20 > dl { dl = 20 } }
        colorish = pos
        if cnt > colorish { colorish = cnt }
        if dl > colorish { colorish = dl }
    } else {
        // ---- color (non-Common shape): name[0] table + 17-nibble light/dark ----
        var nc = byte(data, nb)
        var cs = 0u64
        if nc == 65 { cs = 7 }
        if nc == 66 { cs = 6 }
        if nc == 69 { cs = 1 }
        if nc == 72 { cs = 8 }
        if nc == 73 { cs = 8 }
        if nc == 75 { cs = 8 }
        if nc == 77 { cs = 2 }
        if nc == 78 { cs = 6 }
        if nc == 83 { cs = 6 }
        if nc == 84 { cs = 8 }
        if nc == 85 { cs = 3 }
        if nc == 86 { cs = 8 }
        if nc == 87 { cs = 3 }
        if nc == 88 { cs = 5 }
        if nc == 89 { cs = 3 }
        if nc == 90 { cs = 10 }
        var alldark2 = 1u64
        var alllight2 = 1u64
        var k2 = 2u64
        while k2 < 19 {
            var hv2 = byte(data, gb + k2 / 2)
            if k2 % 2 == 0 { hv2 = hv2 / 16 } else { hv2 = hv2 % 16 }
            if hv2 > 4 { alldark2 = 0 }
            if hv2 <= 4 { alllight2 = 0 }
            k2 += 1
        }
        if alldark2 == 1 { if 20 > cs { cs = 20 } }
        if alllight2 == 1 { if 20 > cs { cs = 20 } }
        colorish = cs
    }

    // ---- nameliteral (6 name bytes) ----
    var maxrun = 1u64
    var run = 1u64
    var q = 1u64
    while q < 6 {
        var c1 = byte(data, nb + q)
        var c0 = byte(data, nb + q - 1)
        if c1 == c0 {
            run += 1
            if run > maxrun { maxrun = run }
        } else {
            run = 1
        }
        q += 1
    }
    var bm2 = 0u64
    q = 0
    while q < 6 {
        var lc = byte(data, nb + q)
        var li = 16u64
        if lc == 87 { li = 0 }
        if lc == 84 { li = 1 }
        if lc == 89 { li = 2 }
        if lc == 85 { li = 3 }
        if lc == 73 { li = 4 }
        if lc == 65 { li = 5 }
        if lc == 72 { li = 6 }
        if lc == 88 { li = 7 }
        if lc == 86 { li = 8 }
        if lc == 77 { li = 9 }
        if lc == 69 { li = 10 }
        if lc == 75 { li = 11 }
        if lc == 66 { li = 12 }
        if lc == 83 { li = 13 }
        if lc == 90 { li = 14 }
        if lc == 78 { li = 15 }
        bm2 = bm2 | (1 << li)
        q += 1
    }
    var px2 = bm2 & 65535
    px2 = px2 - ((px2 >> 1) & 21845)
    px2 = (px2 & 13107) + ((px2 >> 2) & 13107)
    px2 = (px2 + (px2 >> 4)) & 3855
    px2 = px2 + (px2 >> 8)
    var distinct2 = px2 & 31
    var nl = 0u64
    if maxrun >= 3 {
        if maxrun == 6 { nl = 200 }
        if maxrun == 5 { if 50 > nl { nl = 50 } }
        if maxrun == 4 { if 10 > nl { nl = 10 } }
        if maxrun < 4 { if 2 > nl { nl = 2 } }
    }
    if distinct2 == 2 { if 10 > nl { nl = 10 } }
    if distinct2 == 3 { if 3 > nl { nl = 3 } }
    // ABCABC: name[0..3] == name[3..6]
    if byte(data, nb) == byte(data, nb + 3) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 5) {
        if 20 > nl { nl = 20 }
    }
    // Symmetric letter: palindrome
    if byte(data, nb) == byte(data, nb + 5) && byte(data, nb + 1) == byte(data, nb + 4) && byte(data, nb + 2) == byte(data, nb + 3) {
        if 10 > nl { nl = 10 }
    }
    // AABCCD: 1=2 && 4=5
    if byte(data, nb) == byte(data, nb + 1) && byte(data, nb + 3) == byte(data, nb + 4) {
        if 10 > nl { nl = 10 }
    }
    // ABBCDD: 2=3 && 5=6
    if byte(data, nb + 1) == byte(data, nb + 2) && byte(data, nb + 4) == byte(data, nb + 5) {
        if 10 > nl { nl = 10 }
    }

    // ---- numberliteral: BE u64 fold, then the decimal digit string ----
    var num = 0u64
    var j = 0u64
    while j < 8 {
        num = num * 256 + byte(data, xb + j)
        j += 1
    }
    var nu = 0u64
    if num < 10 { nu = 30 }
    // decimal digits, ASCII, built by prepending
    var digs = ""
    var x = num
    if x == 0 { digs = "0" }
    while x > 0 {
        var d10 = x % 10
        var ch = "9"
        if d10 == 0 { ch = "0" }
        if d10 == 1 { ch = "1" }
        if d10 == 2 { ch = "2" }
        if d10 == 3 { ch = "3" }
        if d10 == 4 { ch = "4" }
        if d10 == 5 { ch = "5" }
        if d10 == 6 { ch = "6" }
        if d10 == 7 { ch = "7" }
        if d10 == 8 { ch = "8" }
        digs = ch ++ digs
        x = x / 10
    }
    var dn = size(digs)
    var bm3 = 0u64
    j = 0
    while j < dn {
        bm3 = bm3 | (1 << (byte(digs, j) - 48))
        j += 1
    }
    var px3 = bm3 & 65535
    px3 = px3 - ((px3 >> 1) & 21845)
    px3 = (px3 & 13107) + ((px3 >> 2) & 13107)
    px3 = (px3 + (px3 >> 4)) & 3855
    px3 = px3 + (px3 >> 8)
    var distinct3 = px3 & 31
    if distinct3 == 1 { if 50 > nu { nu = 50 } }
    // Serial num: strictly +1 or strictly -1 across the whole string
    var up = 1u64
    var down = 1u64
    j = 1
    while j < dn {
        var cur = byte(digs, j)
        var pre = byte(digs, j - 1)
        if cur <= pre { up = 0 } else { if cur - pre != 1 { up = 0 } }
        if cur >= pre { down = 0 } else { if pre - cur != 1 { down = 0 } }
        j += 1
    }
    if up == 1 || down == 1 { if 25 > nu { nu = 25 } }
    // Number Symmetry: palindrome
    var sym = 1u64
    j = 0
    while j < dn / 2 {
        if byte(digs, j) != byte(digs, dn - 1 - j) { sym = 0 }
        j += 1
    }
    if sym == 1 { if 10 > nu { nu = 10 } }
    // Tail runs (>= 4)
    var mrun = 1u64
    run = 1
    j = 1
    while j < dn {
        if byte(digs, j) == byte(digs, j - 1) {
            run += 1
            if run > mrun { mrun = run }
        } else {
            run = 1
        }
        j += 1
    }
    if mrun >= 4 {
        // DA: 7 -> 200, 6 -> 100, 5 -> 50, anything else (4 or >= 8) -> 20.
        if mrun == 7 { nu = 200 }
        if mrun == 6 { if 100 > nu { nu = 100 } }
        if mrun == 5 { if 50 > nu { nu = 50 } }
        if mrun < 5 { if 20 > nu { nu = 20 } }
        if mrun > 7 { if 20 > nu { nu = 20 } }
    }

    // ---- total = max(1, shape + colorish + nl + nu) ----
    var total = shape + colorish + nl + nu
    if total < 1 { total = 1 }
    sum += total
    i += 1
}
return sum as bytes"#;


    /* ---- HIP5 scorer: fitsh vs rust, Im vs Imc (RVC) ---- */

    /// Deterministic pseudo-random HIP5 wire: names[6n] || numbers[8n] ||
    /// visual_genes[10n], letters drawn from the 16-letter name alphabet.
    #[allow(dead_code)] // the random wire: kept for reference — it trips the
    // interpreter's narrow-width fault faces (see hip5_wire_narrow_safe)
    pub(crate) fn hip5_wire(n: usize) -> Vec<u8> {
        const LETTERS: &[u8] = b"WTYUIAHXVMEKBSZN";
        let mut st = 0x1234_5678_9abc_def0u64;
        let mut input = Vec::with_capacity(n * 24);
        for _ in 0..n {
            for _ in 0..6 {
                input.push(LETTERS[((st >> 8) % 16) as usize]);
                st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            }
            let num = st;
            input.extend_from_slice(&num.to_be_bytes());
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            for k in 0..10 {
                input.push(((st >> (k * 5)) & 0xFF) as u8);
            }
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        }
        input
    }


    /// Task-1 A3: the interpreter computes score_sum's narrow-width faces
    /// at u8/u16 — `1 << hv` faults for nibbles ≥ 8 and
    /// `1 << (digit - 48)` for decimal digits 8/9. The historical rv64 run
    /// masked/ignored those (the pinned width deviation); the closure
    /// makes both engines fault together. This wire keeps every value
    /// inside the narrow domains (vg nibbles ≤ 7, decimal digits ≤ 7) in
    /// the kernel's FIELD-MAJOR layout (names[6n] || numbers[8n] ||
    /// vg[10n] — `nb/xb/gb` index that layout) so the size/cycles report
    /// still measures a successful full run.
    pub(crate) fn hip5_wire_narrow_safe(n: usize) -> Vec<u8> {
        const LETTERS: &[u8] = b"WTYUIAHXVMEKBSZN";
        let mut st = 0x1234_5678_9abc_def0u64;
        let mut next = || {
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            st
        };
        let mut input = Vec::with_capacity(n * 24);
        for _ in 0..n * 6 {
            input.push(LETTERS[((next() >> 8) % 16) as usize]);
        }
        for _ in 0..n {
            // a number whose DECIMAL digits stay in 0..=7 (the digit-string
            // bitmap shift `1 << (digit - 48)` is u8-checked)
            let num: u64 = (0..19).fold(0u64, |a, k| a * 10 + ((next() >> (k * 3)) & 7) as u64);
            input.extend_from_slice(&num.to_be_bytes());
        }
        for _ in 0..n {
            for k in 0..10 {
                // both nibbles ≤ 7 (the visual-gene shift `1 << hv` is
                // u8-checked in the Common branch)
                input.push(((next() >> (k * 5)) & 0x77) as u8);
            }
        }
        input
    }


    /* ---- per-instruction census: fitsh stream vs rust profile-2 stream ---- */

    /// Machine category of one decoded instruction (stream-agnostic).
    pub(crate) fn census_cat(i: &Inst) -> &'static str {
        use crate::calc::decode::{ImmAluOp::*, LoadOp::*, RegAluOp::*, StoreOp::*};
        match i {
            Inst::Lui { .. } => "const/lui",
            Inst::Auipc { .. } => "auipc",
            Inst::ImmAlu { op: Addi, rs1: 0, .. } => "const/li",
            Inst::ImmAlu { op: Addi, imm: 0, rs1, rd } if rd == rs1 && *rs1 != 0 => "mv",
            Inst::RegAlu { op: Add, rs1: 0, .. } => "mv", // c.mv
            Inst::ImmAlu { .. } => "alu-imm",
            Inst::RegAlu { op, .. }
                if matches!(op, Mul | Mulh | Mulhsu | Mulhu | Div | Divu | Rem | Remu) =>
            {
                "mul-div"
            }
            Inst::RegAlu { op, .. } if matches!(op, Slt | Sltu) => "setbb",
            Inst::RegAlu { .. } => "alu-reg",
            Inst::ShiftImm { .. } => "shift",
            Inst::Load { op: Ld, .. } => "ld",
            Inst::Load { op: Lbu, .. } => "lbu",
            Inst::Load { .. } => "load",
            Inst::Store { op: Sd, .. } => "sd",
            Inst::Store { op: Sb, .. } => "sb",
            Inst::Store { .. } => "store",
            Inst::Branch { rs2: 0, .. } => "br-zero",
            Inst::Branch { .. } => "br",
            Inst::Jal { rd: 0, .. } => "j",
            Inst::Jal { .. } => "jal-link",
            Inst::Jalr { .. } => "jr",
            Inst::Ecall => "ecall",
            _ => "other",
        }
    }


    /// Decode a stream and aggregate (count, bytes, compressed-2B count)
    /// per category. Returns rows sorted by fitsh-bycategory order of first
    /// appearance; caller prints.
    pub(crate) fn census_stream(
        code: &[u8],
        profile: u8,
    ) -> Vec<(&'static str, usize, usize, usize)> {
        let mut map: std::collections::BTreeMap<&'static str, (usize, usize, usize)> =
            Default::default();
        let mut off = 0usize;
        while off < code.len() {
            let (inst, len) = decode_at(code, off, profile).unwrap();
            let e = map.entry(census_cat(&inst)).or_default();
            e.0 += 1;
            e.1 += len as usize;
            e.2 += (len == 2) as usize;
            off += len as usize;
        }
        map.into_iter().map(|(k, (n, b, c))| (k, n, b, c)).collect()
    }


    /// The rust kernel's RX code bytes (profile-2 fixture), for the census.
    pub(crate) fn rust_rvc_code() -> Option<Vec<u8>> {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let rel = std::path::Path::new("doc/hacdsdefi4000.loc/out_rvc/score_sum.hcf1");
        let p = [base.join(rel), base.join("../..").join(rel), base.join("..").join(rel)]
            .into_iter()
            .find(|p| p.exists())?;
        let bytes = std::fs::read(p).ok()?;
        let (c, _) = crate::calc::container::CalcContainer::decode(&bytes).ok()?;
        let seg = c
            .segs
            .iter()
            .find(|s| s.vaddr == crate::calc::container::CODE_BASE as u32)?;
        Some(seg.data.clone())
    }


    pub(crate) fn deep_add_body(depth: usize) -> (String, u64) {
        let mut body = String::new();
        let mut want = 0u64;
        for i in 0..depth {
            body.push_str(&format!("var v{i} = {}u64\n", i + 1));
            want += (i + 1) as u64;
        }
        let mut expr = String::new();
        for i in (0..depth).rev() {
            expr = if i == depth - 1 {
                format!("v{i}")
            } else {
                format!("v{i} + ({expr})")
            };
        }
        body.push_str(&format!("return ({expr}) as bytes"));
        (body, want)
    }


    /* ---- input-parameter kernel (the `function rv64im k(data: bytes)` ABI) ---- */

    pub(crate) fn compile_with_input(body: &str) -> (IRNodeArray, Vec<u8>) {
        let tokens = Tokenizer::new(body.as_bytes()).parse().unwrap();
        let (ir, compiled, _) = compile_body(
            tokens,
            vec![("data".to_string(), crate::value::ValueTy::Bytes)],
            &[],
            &[],
            false,
        )
        .unwrap();
        match compiled {
            crate::fitshc::CompiledCode::Bytecode(b) => (ir, b),
            _ => unreachable!(),
        }
    }


    pub(crate) fn run_input(body: &str, input: &[u8]) -> Vec<u8> {
        run_input_mode(body, input, Rv64Mode::Im)
    }


    pub(crate) fn run_input_mode(body: &str, input: &[u8], mode: Rv64Mode) -> Vec<u8> {
        let (ir, _) = compile_with_input(body);
        let lo = lower_body_mode(&ir, true, mode).unwrap();
        let cap = SpaceCap::new(0);
        let container = match mode {
            Rv64Mode::Im => lo.pack_segments(&cap, 64, 5120, 1280),
            _ => lo.pack_segments_profile(&cap, 64, 5120, 1280, crate::calc::container::CALC_PROFILE_RV64IMC),
        };
        let fno = CalcFnObj::build_v1(container.unwrap_or_else(|e| panic!("pack: {e}"))).unwrap();
        let exec =
            fno.run(input, CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
        match exec.result {
            Ok(out) => out,
            Err(e) => panic!("kernel aborted: {e:?} exit={}", exec.exit_code),
        }
    }


    /* ---- comparison report (same IR, both channels) ---- */

    /// cycles→compute uses the frozen CYCLES_PER_COMPUTE = 56 (SPEC §8).
    pub(crate) const N: f64 = 56.0;


    /// Find the first while-loop body in the IR and price its HVM bytecode.
    pub(crate) fn hvm_per_iter_gas(ir: &IRNodeArray) -> Option<i64> {
        fn find_while(node: &dyn IRNode) -> Option<i64> {
            if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
                return find_while(&*w.node);
            }
            if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
                for c in &arr.subs {
                    if let Some(r) = find_while(&**c) {
                        return Some(r);
                    }
                }
            }
            if let Some(d) = node.as_any().downcast_ref::<IRNodeDouble>() {
                if d.inst == Bytecode::IRWHILE {
                    if let Ok(bts) = d.suby.codegen() {
                        return hvm_bytecode_stats(&bts).ok().map(|(_, g)| g);
                    }
                }
                for c in [&d.subx, &d.suby] {
                    if let Some(r) = find_while(&**c) {
                        return Some(r);
                    }
                }
            }
            None
        }
        find_while(ir)
    }


    /// Executed rv64 cycles for a body run through the consensus engine.
    pub(crate) fn rv_exec(body: &str) -> (u64, Vec<u8>) {
        let lo = lower(body);
        let cap = SpaceCap::new(0);
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec =
            fno.run(&[], CalcRunLimits { cycles: 50_000_000, resource: -1, deadline: None });
        match exec.result {
            Ok(o) => (exec.cycles_used, o),
            Err(e) => panic!("abort {e:?}"),
        }
    }


    /* ---- A-19 B2-FIXUP: mul_check_base_const copy->check rescan pins ---- */

    pub(crate) fn li_i(rd: u8, imm: i64) -> Inst {
        Inst::ImmAlu { op: ImmAluOp::Addi, rd, rs1: 0, imm }
    }

    pub(crate) fn mv_r(rd: u8, rs2: u8) -> Inst {
        Inst::RegAlu { op: RegAluOp::Add, rd, rs1: 0, rs2 }
    }
