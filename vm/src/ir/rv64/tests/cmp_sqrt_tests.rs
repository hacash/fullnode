//! D8/A8 shape pins (docs.loc/fitsh-rv64/survey: fin-control.md D8,
//! arith.md A8):
//! * D8 — the u128 compare branch fusion: an `if (a OP b)` condition on a
//!   u128 pair branches directly (no 0/1 byte vreg). The per-op branch
//!   shapes differ by one slot (the EQ "branch when different" form is
//!   two bne's); these pins hold the DELTAS over a no-compare baseline so
//!   a regression to the materializing shape (+4..+5 slots per site)
//!   trips them. Behavior is covered by the testkit corpus
//!   `cmp_sqrt_differential.rs`.
//! * A8 — the sqrt routine gate: a single-site body must lower
//!   BYTE-IDENTICAL to the pre-gate inline form (the gate is OFF), a
//!   ≥-threshold body emits the routine body exactly once per variant
//!   (pinned by the differential corpus; the gate economics are recorded
//!   in the A8 commit message).

use super::super::*;
use super::helpers::lower_mode;

const MODES: [Rv64Mode; 3] = [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap];

#[test]
fn d8_u128_cond_compare_branch_shapes() {
    let base = "var a = 0u128\nvar b = 1u128\nvar r = 0u64\nreturn r as bytes";
    // (op, expected if-region delta over `base`)
    // ordered (LT/GE/GT/LE): bne + decision branch + bltu tail = 3
    // EQ (when_false = branch when different): two bne's = 2
    // NEQ (when_false = branch when equal): bne + bne + jal = 3
    let ops: [(&str, i64); 6] = [("<", 14), (">", 14), ("<=", 14), (">=", 14), ("==", 13), ("!=", 14)];
    for mode in MODES {
        let b0 = lower_mode(base, mode).stats.insts as i64;
        for (op, want) in ops {
            let body = format!(
                "var a = 0u128\nvar b = 1u128\nvar r = 0u64\nif (a {op} b) {{ r = 7u64 }} else {{ r = 9u64 }}\nreturn r as bytes"
            );
            let d = lower_mode(&body, mode).stats.insts as i64 - b0;
            assert_eq!(
                d, want,
                "D8 fused {op} if-region delta {d} != {want} in {mode:?} — the 0/1 materialization is back"
            );
        }
    }
}
