//! Task 7: the heap faces — one constant `heap_grow` reserves a zero-filled
//! frame block, the six read/write opcodes share it with the interpreter's
//! `checked_right` faces, four-way agreed against the consensus interpreter.
//!
//! Faces and their ORDER mirror the interpreter exactly (execute.rs:941-953
//! + space/heap.rs):
//! * the heap length is `datas.len()`: 0 until the grow executes, the grown
//!   byte count after — the kernel tracks it with a prologue-zeroed grown
//!   flag (the admitted HGROW runs at most once per path, so the flag is
//!   monotone); `start + len > length` → HeapError(44), `==` is IN bounds,
//!   and `read(0, 0)` on the empty heap returns the empty bytes.
//! * HREAD/HWRITE dynamic operands: `extract_u16`/`extract_u32` faces first
//!   (CastParamFail 72 on Nil / over-ceiling), then the write value's
//!   `extract_bytes` (Nil → mapped HeapError 44), then the bounds (44).
//! * HREADU/HREADUL decode their u8/u16 markers exactly like `read_u` /
//!   `read_ul` (uty | seg; the cell is `len(uty) × seg`, big-endian); u128
//!   reads materialize the (lo, hi) vstack pair.
//! * per-grow segment cap = `SpaceCap::heap_grow_segment` (task A), checked
//!   at COMPILE time kernel-side through the lowering parameter channel;
//!   the FRAME budget is the 2040B S0 window (the same budget that bounds
//!   LIST_BLOCK_MAX and the divmod workspace) — a reservation that does not
//!   fit rejects with the heap-named message.

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{frontend, Interp, MODES, Rv64};

use vm::ir::rv64::{lower_body_mode_arena_heap, ARENA_SIZE_MAX, Rv64Mode};

/// Interpreter result mapped to the rv64 observable shape.
fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

/// Four-way agreement on EVERY input: interpreter == all three rv64 modes.
fn agree4(body: &str, inputs: &[Vec<u8>]) {
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("rv64 must build: {body}\n{e}"));
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
    for input in inputs {
        let want = interp_obs(&it, input);
        for (mode, got) in rv.run(input) {
            assert_eq!(
                got, want,
                "mode {mode:?} diverged on input {input:?} of body:\n{body}"
            );
        }
    }
}

/// Four-way agreement over a body built with a CUSTOM per-grow segment cap
/// (the task-A field through the lowering parameter channel).
fn agree4_field(body: &str, heap_seg_max: usize, inputs: &[Vec<u8>]) {
    let (ir, hvm_size) = frontend(body).unwrap_or_else(|e| panic!("frontend: {body}\n{e}"));
    let cap = vm::calc::SpaceCap::new(0);
    let mut engines = Vec::new();
    for &mode in MODES.iter() {
        let lo = lower_body_mode_arena_heap(&ir, true, mode, ARENA_SIZE_MAX, heap_seg_max)
            .unwrap_or_else(|e| panic!("rv64(lower,{mode:?}): {e}"));
        let profile = match mode {
            Rv64Mode::Im => vm::calc::container::CALC_PROFILE_RV64IM,
            _ => vm::calc::container::CALC_PROFILE_RV64IMC,
        };
        let container = lo
            .pack_segments_profile(&cap, 64, 4800, 16, profile)
            .unwrap_or_else(|e| panic!("rv64(pack,{mode:?}): {e}"));
        let bytes = container.encode().unwrap_or_else(|e| panic!("rv64(encode): {e}"));
        let (rebuilt, _) = vm::calc::container::CalcContainer::decode(&bytes).unwrap();
        let fno = vm::calc::CalcFnObj::build(rebuilt, &cap, vm::calc::CalcHostConfig::new())
            .unwrap_or_else(|e| panic!("rv64(build,{mode:?}): {e}"));
        engines.push((mode, fno));
    }
    let _ = hvm_size;
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("interp must build: {body}\n{e}"));
    for input in inputs {
        let want = interp_obs(&it, input);
        for &(mode, ref fno) in &engines {
            let ex = fno.run(input, vm::calc::CalcRunLimits {
                cycles: c3lib::RV64_CYCLES,
                resource: -1,
                deadline: None,
            });
            let got = ex.result.map_err(|_| ex.exit_code);
            assert_eq!(
                got, want,
                "mode {mode:?} diverged on input {input:?} of body:\n{body}"
            );
        }
    }
}

/// rv64 must reject, naming `frag` and the canonical subset text.
fn rv64_must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must name {frag:?}: {err:?} (body: {body})"
    );
    assert!(
        err.contains("outside the translatable kernel subset"),
        "reject must use the canonical subset text: {err:?}"
    );
}

/// rv64 must reject the body built with the CUSTOM per-grow segment cap.
fn rv64_field_must_reject(body: &str, heap_seg_max: usize, frag: &str) {
    let (ir, _) = frontend(body).unwrap_or_else(|e| panic!("frontend: {e}"));
    let err = lower_body_mode_arena_heap(&ir, true, Rv64Mode::Im, ARENA_SIZE_MAX, heap_seg_max)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must name {frag:?}: {err:?} (body: {body})"
    );
}

/// Interpreter acceptance proof.
fn interp_accepts(body: &str, run: bool) {
    let it = Interp::try_build(body)
        .unwrap_or_else(|e| panic!("interp must accept: {body}\n{e}"));
    if run {
        let _ = it.run(&[1, 2, 3, 4]);
    }
}

fn inputs_sweep() -> Vec<Vec<u8>> {
    (0u16..=255).map(|b| vec![b as u8, 7, 11]).collect()
}

/* ================================================================== */
/* Section 1: grow + write/read roundtrips (four-way agreement).       */
/* ================================================================== */

#[test]
fn s1_grow_write_read_roundtrip() {
    // the parent probe shape: grow 4 segments, write 258 (a U16 → 2 BE
    // bytes) at 0, read 2 bytes back
    let body = "heap_grow(4)\n\
                heap_write(0, 258)\n\
                return heap_read(0, 2)";
    agree4(body, &[vec![0u8; 4]]);
    // the read is a real bytes value: byte()/size over the descriptor
    let body2 = "heap_grow(4)\n\
                 heap_write(0, 258)\n\
                 var b = heap_read(0, 2)\n\
                 return ((byte(b, 0) as u64) * 256u64 + (byte(b, 1) as u64)) as bytes";
    agree4(body2, &[vec![0u8; 4]]);
}

#[test]
fn s2_fresh_heap_reads_zeros() {
    // the grown block is zero-filled (interpreter resize(0u8))
    let body = "heap_grow(2)\n\
                return heap_read(0, 4)";
    agree4(body, &[vec![0u8; 4]]);
}

#[test]
fn s3_readu_widths() {
    // read_u's marker: uty = mark >> 6, seg = mark & 63 — each width's
    // big-endian cell at len × seg
    // u8: mark 0x00 → 1 byte at 0 (the written 200 is a U8 → 1 byte)
    let b8 = "heap_grow(1)\n\
              heap_write(0, 200)\n\
              return (heap_read_uint(0) as u64) as bytes";
    agree4(b8, &[vec![0u8; 4]]);
    // u16: mark 0x40 → 2 bytes at 0 (258 is a U16 → bytes 01 02)
    let b16 = "heap_grow(1)\n\
               heap_write(0, 258)\n\
               return (heap_read_uint(64) as u64) as bytes";
    agree4(b16, &[vec![0u8; 4]]);
    // u32: mark 0x80 → 4 bytes at 0
    let b32 = "heap_grow(1)\n\
               heap_write(0, 287454020)\n\
               return (heap_read_uint(128) as u64) as bytes";
    agree4(b32, &[vec![0u8; 4]]);
    // u64: mark 0xC0 → 8 bytes at 0 (the literal > 2³² is a U64 → 8 bytes)
    let b64 = "heap_grow(1)\n\
               heap_write(0, 4294967301)\n\
               return (heap_read_uint(192) as u64) as bytes";
    agree4(b64, &[vec![0u8; 4]]);
    // the read width carries into the arithmetic faces: u8 + 1 at the u8
    // width (255 overflows → 90 on both engines)
    let ovf = "heap_grow(1)\n\
               heap_write(0, 255)\n\
               var v = heap_read_uint(0)\n\
               return ((v + 1) as u64) as bytes";
    agree4(ovf, &[vec![0u8; 4]]);
}

#[test]
fn s4_readul_u128_cell() {
    // read_ul's marker: uty = mark >> 13 — uty 4 (source arg 128) is the
    // u128 read; the written literal > u64::MAX is a CU128 → 16 BE bytes;
    // the read returns the u128 pair and `as bytes` re-emits 16 BE bytes
    let body = "heap_grow(1)\n\
                heap_write(0, 18446744073709551621)\n\
                return (heap_read_uint_long(128, 0) as u128) as bytes";
    agree4(body, &[vec![0u8; 4]]);
    // a u128 read at a nonzero segment: mark = (4 << 13) | seg, idx = 16×seg
    let body2 = "heap_grow(2)\n\
                 heap_write_x(32, 171)\n\
                 heap_write(16, 18446744073709551621)\n\
                 return (heap_read_uint_long(128, 1) as u128) as bytes";
    agree4(body2, &[vec![0u8; 4]]);
}

#[test]
fn s5_writex_writexl_immediates() {
    // HWRITEX: u8 immediate offset — the LAST byte of a 1-segment heap is
    // exactly at the checked_right boundary (== length is IN bounds)
    let body = "heap_grow(1)\n\
                heap_write_x(255, 200)\n\
                return heap_read(255, 1)";
    agree4(body, &[vec![0u8; 4]]);
    // HWRITEXL: u16 immediate offset (source args are the two BE bytes —
    // (1, 44) = 300), over a 2-segment heap
    let body2 = "heap_grow(2)\n\
                 heap_write_xl(1, 44, 171)\n\
                 return heap_read(300, 1)";
    agree4(body2, &[vec![0u8; 4]]);
}

#[test]
fn s6_write_value_kinds() {
    // bool → the canonical 0/1 byte; a bytes value → its own bytes; a u128
    // → the 16-byte big-endian form
    let body = "heap_grow(1)\n\
                heap_write(0, true)\n\
                heap_write_x(1, false)\n\
                var b = \"abc\" as bytes\n\
                heap_write(2, b)\n\
                return heap_read(0, 5)";
    agree4(body, &[vec![0u8; 4]]);
    let body2 = "heap_grow(1)\n\
                 heap_write(0, 18446744073709551621)\n\
                 return heap_read(0, 16)";
    agree4(body2, &[vec![0u8; 4]]);
}

#[test]
fn s7_dynamic_offset_and_len_sweep() {
    // HREAD/HWRITE with data-driven start/length — the 72 extract faces
    // (Nil is structurally absent on a definite uint; the u16/u32 ceiling
    // faces are exercised by the sweep), the 44 bounds, the descriptor
    // roundtrip
    let body = "heap_grow(1)\n\
                var i = (byte(data, 0) as u64) % 250u64\n\
                heap_write(i, (byte(data, 1) as u64) + 1u64)\n\
                var b = heap_read(i, 1)\n\
                return ((byte(b, 0) as u64)) as bytes";
    agree4(body, &inputs_sweep());
}

/* ================================================================== */
/* Section 2: length semantics — before the grow / past the length.    */
/* ================================================================== */

#[test]
fn s8_before_grow_read_write_trap_44() {
    // the grow sits under a conditional: on the false path the heap length
    // stays 0 — every non-empty access faults HeapError(44) on BOTH engines
    let body = "var q = byte(data, 0)\n\
                if q == 255u64 { heap_grow(1) }\n\
                heap_write(0, 1)\n\
                return 0u64 as bytes";
    agree4(body, &inputs_sweep());
    let body2 = "var q = byte(data, 0)\n\
                 if q == 255u64 { heap_grow(1) }\n\
                 var b = heap_read(0, 1)\n\
                 return (size(b) as u64) as bytes";
    agree4(body2, &inputs_sweep());
}

#[test]
fn s9_empty_read_on_the_empty_heap() {
    // checked_right(0, 0) passes on a length-0 heap: read(0, 0) returns the
    // empty bytes BEFORE any grow — the pinned edge of the length model
    // (length = grown_flag × reserved, no separate flag gate)
    let body = "var b = heap_read(0, 0)\n\
                return b";
    agree4(body, &[vec![0u8; 4]]);
    // read(1, 0) does not: 1 > 0 → 44
    let body2 = "var b = heap_read(1, 0)\n\
                 return (size(b) as u64) as bytes";
    agree4(body2, &[vec![0u8; 4]]);
    // after the grow the empty read at the very end of the block passes
    let body3 = "heap_grow(1)\n\
                 var b = heap_read(256, 0)\n\
                 return (size(b) as u64) as bytes";
    agree4(body3, &[vec![0u8; 4]]);
}

#[test]
fn s10_seg_zero_runtime_heap_error() {
    // seg == 0 keeps the interpreter's runtime face: the trap fires where
    // execution reaches the grow (decision: NOT a compile reject — the
    // spec pins HeapError, and the execution-position-dependent face is
    // reproduced exactly)
    let body = "heap_grow(0)\n\
                return 0u64 as bytes";
    agree4(body, &[vec![0u8; 4]]);
    // reads BEFORE the grow still see the length-0 heap on both engines
    let body2 = "var b = heap_read(0, 1)\n\
                 heap_grow(0)\n\
                 return (size(b) as u64) as bytes";
    agree4(body2, &[vec![0u8; 4]]);
}

#[test]
fn s11_oob_after_grow_44() {
    // full-segment read (== length) is IN bounds; one past is 44
    let body = "heap_grow(1)\n\
                var n = (byte(data, 0) as u64) % 4u64 + 254u64\n\
                var b = heap_read(0, n)\n\
                return (size(b) as u64) as bytes";
    // n ∈ 254..257: 254/255/256 read, 257 faults 44 — both engines
    agree4(body, &inputs_sweep());
    // dynamic write past the end: offset 255 + a 2-byte value → 44
    let body2 = "heap_grow(1)\n\
                 var i = (byte(data, 0) as u64) % 3u64 + 254u64\n\
                 heap_write(i, 1000)\n\
                 return 0u64 as bytes";
    agree4(body2, &inputs_sweep());
}

/* ================================================================== */
/* Section 3: compile rejects (kernel subset boundaries).              */
/* ================================================================== */

#[test]
fn r1_grow_in_loop_rejects() {
    let body = "var i = 0u64\n\
                while i < 2u64 { heap_grow(1)\ni += 1 }\n\
                return 0u64 as bytes";
    rv64_must_reject(body, "heap_grow inside a loop");
}

#[test]
fn r2_second_grow_rejects_even_on_exclusive_paths() {
    // decision: exactly ONE heap_grow node per body — the frame reserves
    // that single constant's bytes and the length semantics reference it;
    // exclusive-path pairs would need two reservations
    let body = "var q = byte(data, 0)\n\
                if q == 1u64 { heap_grow(1) } else { heap_grow(2) }\n\
                return 0u64 as bytes";
    rv64_must_reject(body, "exactly one heap_grow");
}

#[test]
fn r3_seg_over_default_field_rejects() {
    let body = "heap_grow(17)\n\
                return 0u64 as bytes";
    rv64_must_reject(body, "exceeds SpaceCap::heap_grow_segment 16");
}

/// rv64 must reject naming `frag` — for the BUDGET faces (which are not
/// subset rejections and carry no canonical subset text).
fn rv64_budget_must_reject(body: &str, frag: &str) {
    let err = Rv64::try_build(body)
        .err()
        .unwrap_or_else(|| panic!("rv64 must reject: {body}"))
        .to_string();
    assert!(
        err.contains(frag),
        "reject text must name {frag:?}: {err:?} (body: {body})"
    );
}

#[test]
fn r4_frame_budget_rejects() {
    // seg 8 → 2048B + flag > the 2040B window: the same budget that bounds
    // the list blocks and the divmod workspace (seg ≤ 7 fits a heap-only
    // frame; the default field 16 admits up to the budget, no further)
    let body = "heap_grow(8)\n\
                return 0u64 as bytes";
    rv64_budget_must_reject(body, "kernel memory budget");
    // the largest usable reservation builds and agrees
    let ok = "heap_grow(7)\n\
              heap_write_xl(6, 255, 9)\n\
              return heap_read(1791, 1)";
    agree4(ok, &[vec![0u8; 4]]);
}

#[test]
fn r5_non_constant_grow_rejects_at_the_frontend() {
    // the HGROW immediate is structural (pms=1): a variable argument is a
    // frontend reject on BOTH engines — no rv64-side face exists
    let body = "var n = 4u64\n\
                heap_grow(n)\n\
                return 0u64 as bytes";
    let ie = Interp::try_build(body).err().unwrap_or_else(|| panic!("interp must reject"));
    assert!(ie.contains("immediate u8 literal"), "interp: {ie}");
    let re = Rv64::try_build(body).err().unwrap_or_else(|| panic!("rv64 must reject"));
    assert!(re.contains("immediate u8 literal"), "rv64: {re}");
}

#[test]
fn r6_field_lowered_kernel_rejects() {
    // the same body: seg 3 over a lowered field 2 rejects; seg 2 passes the
    // field and agrees — the cap number flows through the parameter
    // channel (no kernel-side 16)
    let over = "heap_grow(3)\n\
                heap_write_x(0, 1)\n\
                return heap_read(0, 1)";
    rv64_field_must_reject(over, 2, "exceeds SpaceCap::heap_grow_segment 2");
    let ok = "heap_grow(2)\n\
              heap_write_x(0, 1)\n\
              return heap_read(0, 1)";
    agree4_field(ok, 2, &[vec![0u8; 4]]);
    // the field raised to the u8 ceiling: seg 7 still builds (the frame
    // budget, not the field, is the binding constraint kernel-side) and
    // the interpreter's own raised-field faces (grow 255 etc.) are the
    // task-A heap.rs unit tests
    agree4_field(ok, 255, &[vec![0u8; 4]]);
}

#[test]
fn r7_heap_ops_are_plan_admitted_without_a_grow() {
    // a body with heap reads but no heap_grow compiles (length-0 heap) and
    // agrees with the interpreter — every non-empty access traps 44
    let body = "var b = heap_read(0, 1)\n\
                return (size(b) as u64) as bytes";
    agree4(body, &[vec![0u8; 4]]);
    let body2 = "heap_write_x(0, 5)\n\
                 return 0u64 as bytes";
    agree4(body2, &[vec![0u8; 4]]);
}

/* ================================================================== */
/* Section 4: interpreter-side field faces (default cap, end to end).  */
/* ================================================================== */

#[test]
fn i1_default_cap_grow_faces_end_to_end() {
    // grow 17 under the default cap: the interpreter faults HeapError(44)
    // at runtime; the rv64 kernel compile-rejects (single-side pins)
    let body17 = "heap_grow(17)\n\
                  return 0u64 as bytes";
    interp_accepts(body17, true);
    let it = Interp::try_build(body17).unwrap();
    let f = it.run(&[0u8; 4]).unwrap_err();
    assert_eq!(f.code, 44, "interp grow(17) is a runtime HeapError");
    rv64_must_reject(body17, "exceeds SpaceCap::heap_grow_segment 16");

    // grow 16 under the default cap runs on the interpreter (the kernel
    // rejects on the frame budget — the documented budget-side difference)
    let body16 = "heap_grow(16)\n\
                  heap_write(0, 258)\n\
                  return heap_read(0, 2)";
    let it = Interp::try_build(body16).unwrap();
    let out = it.run(&[0u8; 4]).unwrap();
    assert_eq!(out, vec![1, 2]);
    rv64_budget_must_reject(body16, "kernel memory budget");
}
