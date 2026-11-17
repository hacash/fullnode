//! kehuobu 10 (developable part) · INC / DEC lowering.
//!
//! `increase(n, x)` / `decrease(n, x)` keep the operand's own width: the
//! interpreter's `Value::inc`/`dec` are checked ops on the value's type
//! (U8 + n stays U8, overflow → Arithmetic 90). The kernel emits the same
//! width-exact add/sub with the task-1 A3 range check.

#[path = "c3lib/mod.rs"]
mod c3lib;
use c3lib::{Interp, Rv64};

fn agree4(body: &str) {
    let it = Interp::try_build(body).unwrap_or_else(|e| panic!("{body}: interp build: {e}"));
    let want = it.run(&[]);
    let rv = Rv64::try_build(body).unwrap_or_else(|e| panic!("{body}: rv64 build: {e}"));
    for (mode, got) in rv.run(&[]) {
        match (&want, &got) {
            (Ok(w), Ok(g)) => assert_eq!(w, g, "{body} [{mode:?}]"),
            (Err(w), Err(code)) => assert_eq!(w.code as u64, *code, "{body} [{mode:?}]"),
            _ => panic!("{body} [{mode:?}]: interp={want:?} rv64={got:?}"),
        }
    }
}

#[test]
fn inc_dec_agree_at_every_width() {
    agree4("var a = 5u64\nvar b = increase(3, a)\nreturn b as bytes");
    agree4("var a = 5u64\nvar b = decrease(3, a)\nreturn b as bytes");
    agree4("var a = 5u32\nvar b = increase(1, a)\nreturn b as bytes");
    agree4("var a = 0u16\nvar b = decrease(0, a)\nreturn b as bytes");
    agree4("var a = 5u64\nvar b = increase(255, a)\nreturn b as bytes");
}

#[test]
fn inc_dec_keep_the_operand_width_and_fault_on_overflow() {
    // U8 + 10 wraps the u8 domain: the interpreter faults 90, so does the
    // kernel (the result width follows the operand, never widens).
    agree4("var a = 250u8\nvar b = increase(10, a)\nreturn b as bytes");
    agree4("var a = 2u8\nvar b = decrease(5, a)\nreturn b as bytes");
    agree4("var a = 255u8\nvar b = increase(0, a)\nreturn b as bytes");
}
