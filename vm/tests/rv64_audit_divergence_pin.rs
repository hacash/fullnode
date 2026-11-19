//! REGRESSION GUARDS for the round-1 adversarial audit divergences
//! (`rv64_adversarial_audit.rs`, seeds 0x32d23d5fc28837c7 and
//! 0x5a40a958a5949437, first reproduced on impl/audit-fuzz at 629e850).
//!
//! These tests were `#[ignore]`d pins that asserted the divergences were
//! STILL REPRODUCIBLE while the defect was open. The defect is now FIXED
//! (x17 removed from the ImcRemap bank candidate list; see mod.rs
//! P2-A27); the pins stay `#[ignore]`d under their historical names and
//! now assert the FIXED behavior as regression guards: interpreter and
//! all three rv64 modes must agree (identical Ok bytes or identical
//! fault code) on the exact program shapes that used to diverge. If one
//! of these goes red, the x17/routine-clobber conflict has regressed —
//! re-run the full adversarial campaign before touching the backend.
//!
//! Original defect — attribution: NOT candidate-introduced. Both faces
//! reproduced byte-identically at the backend's founding commit 25806ba
//! (before the whole rv64-size-min optimization campaign: verified at
//! 25806ba, 744416f and the size baseline 34d2935). Root cause (single,
//! shared by both faces):
//!
//!   In ImcRemap the P2-A27 bank expansion (vm/src/ir/rv64/mod.rs,
//!   "d <= 6" branch) offered x17 (a7) as a slot home with the comment
//!   "a7/x17 is safe because the whole product has exactly ONE ecall ...
//!   so its only write happens after no bank read can ever occur". But
//!   the shared u128 divmod routine (`emit_divmod_routine`,
//!   lib_u128.rs: `const TMP: u8 = 17`), the shared u128 mulmod routine
//!   (`emit_mulmod_routine`, emit/arith.rs: `const TMP: u8 = 17`) and
//!   the shared u256 divmod routines (lib_u256.rs, `const TMP: u8 = 17`)
//!   use x17 as an UNSAVED working register: their save/restore sets
//!   ([T1, T2, 14, (VLO, VHI, CARRY,) T3, T4] — mode-specialized per D4)
//!   never include 17, so any ImcRemap bank slot homed in x17 and live
//!   across a routine call read routine scratch afterwards. The two code
//!   comments contradicted each other; both shipped in 25806ba, and the
//!   campaign's generator merely grew the first program shape that
//!   satisfies all trigger conditions: ImcRemap bank expansion active
//!   (probe depth <= 6), a web segment homed in x17 across the call, and
//!   a divmod/mulmod site (D2/D3 gating: the routine only fires at >= 3
//!   sites; here w0 % u128::MAX supplies it).
//!
//! Faces (both were ImcRemap-only; Im and Imc always agreed with the
//! interpreter):
//!   * F3 — silent value divergence (the worst kind: both engines Ok,
//!     different bytes). Mode ImcRemap took the wrong arm of a value-if
//!     because the condition's operand (n16, homed in x17) read 0
//!     instead of 128.
//!   * F4 — missing fault: interp/Im/Imc raise CastFail(71) for
//!     `n32 as u8` with n32 = 4294967295; ImcRemap's guard compared
//!     garbage (x17 = 0) and passed, returning Ok.
//!
//! Disassembly proof sketch (F3, pre-fix ImcRemap): n16's PUT was
//! `c.mv a7, a0` (0x5c); after `jal t0` into the divmod routine, the
//! condition loaded `c.mv a1, a7` (0xea) and `andi a1, a1, 0xff`; the
//! routine's loop last wrote a7 via `sltu a7, s8, t6` = 0 — so
//! `0 == 0` compared true and the then-arm (c3 + 1 = 1) was selected
//! where the interpreter took the else-arm (0). acc folded 1 -> 31 ->
//! 961 = 0x3c1 = output [0,0,0,0,0,0,3,193]. Full chain in
//! docs.loc/fitsh-rv64/audit-round1.md (§F3/§F4).

#![allow(dead_code)]

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

const Z128: &[u8] = &[0u8; 128];
const FF128: &[u8] = &[0xFFu8; 128];

/// F3 — seed 0x32d23d5fc28837c7 (audit_main_campaign, fault=false,
/// input = 128 zero bytes). Minimized from the 47-statement generated
/// body by statement delta-debugging + expression mutation.
const F3_BODY: &str = "\
var c0 = 0u64
var c1 = 0u64
var c2 = (byte(data, 16) as u64)
var c3 = (byte(data, 17) as u64) % 4u64
var n8 = byte(data, 18)
var n16 = size(data)
var w0 = ((c0 as u128) * 18446744073709551616u128) + (c1 as u128)
var w2 = (w0 % 340282366920938463463374607431768211455u128)
var acc = 0u64
var ch1 = if (n8 == n16 % 256u64) { (c3 + 1u64) } else { (c1 % 1000003u64) }
acc = (acc * 31u64 + (ch1 % 97u64)) % 1000000007u64
acc = (acc * 31u64 + ((c0 % 97u64) % 97u64)) % 1000000007u64
acc = (acc * 31u64 + ((c2 % 7u64) % 97u64)) % 1000000007u64
return (acc) as bytes";

/// F4 — seed 0x5a40a958a5949437 (audit_fault_campaign, fault=true,
/// input = 128 x 0xFF). The generated body minimized to 22 statements;
/// interp/Im/Imc fault CastFail(71) on `n32 as u8`, ImcRemap returns Ok.
const F4_BODY: &str = "\
var c0 = 0u64
var k0 = 0u64
while k0 < 8 { c0 = c0 * 256 + (byte(data, k0) as u64)
k0 += 1 }
var c1 = 0u64
var c3 = (byte(data, 17) as u64) % 4u64
var n8 = byte(data, 18)
var n16 = size(data)
var n32 = ((c0 % 4294967296u64) as u32)
var w0 = ((c0 as u128) * 18446744073709551616u128) + (c1 as u128)
var w2 = (w0 % 340282366920938463463374607431768211455u128)
var acc = 0u64
var l0 = [252u8, byte(data, 3)]
var t1 = ((n16 as u8) as u64)
var t2 = ((n32 as u8) as u64)
acc = (acc * 2u64 + (has_key(l0, (n8 % 3u64)) as u64)) % 1000000007u64
acc = (acc * 31u64 + ((c0 % 97u64) % 97u64)) % 1000000007u64
acc = (acc * 31u64 + ((c1 % 1000003u64) % 97u64)) % 1000000007u64
var i4 = 0u64
while i4 < 5u64 { i4 += 1u64
if i4 == c3 { break }
}
return (acc) as bytes";

/// REGRESSION GUARD (F3): the defect is fixed — interpreter and all
/// three rv64 modes must agree on the exact shape that used to diverge
/// silently (interp Ok([0,...,0]); pre-fix ImcRemap returned
/// Ok([0,0,0,0,0,0,3,193]) = 961 = the then-arm's 1 folded 1 -> 31 ->
/// 961).
#[test]
#[ignore]
fn audit_pinned_f3_silent_value_divergence() {
    let interp = Interp::try_build(F3_BODY).expect("F3: interp build");
    let rv = Rv64::try_build(F3_BODY).expect("F3: rv64 build");
    let want = interp.run(Z128).map_err(|f| f.code as u64).expect("F3: interp Ok");
    assert_eq!(want, vec![0u8; 8], "F3 precondition: interp returns zeros");
    for (mode, got) in rv.run(Z128) {
        assert_eq!(
            got,
            Ok(want.clone()),
            "F3 REGRESSION: {mode:?} disagrees with the interpreter — \
             the divmod/mulmod TMP=x17 vs ImcRemap bank-home-x17 conflict \
             is back; re-run the full adversarial campaign"
        );
    }
}

/// REGRESSION GUARD (F4): the defect is fixed — interpreter and all
/// three rv64 modes must fault identically (CastFail(71),
/// 4294967295 -> u8) on the exact shape where pre-fix ImcRemap silently
/// returned Ok.
#[test]
#[ignore]
fn audit_pinned_f4_missing_cast_fault() {
    let interp = Interp::try_build(F4_BODY).expect("F4: interp build");
    let rv = Rv64::try_build(F4_BODY).expect("F4: rv64 build");
    let fault = interp.run(FF128).err().expect("F4 precondition: interp faults");
    assert_eq!(fault.code, 71, "F4 precondition: CastFail(71)");
    for (mode, got) in rv.run(FF128) {
        let err = got
            .err()
            .unwrap_or_else(|| panic!("F4 REGRESSION: {mode:?} returns Ok where interp faults 71 — the divmod/mulmod TMP=x17 vs ImcRemap bank-home-x17 conflict is back; re-run the full adversarial campaign"));
        assert_eq!(err, 71, "F4: {mode:?} must fault 71 like interp");
    }
}
