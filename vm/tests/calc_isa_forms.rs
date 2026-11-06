//! Execution-level form-coverage proofs for the admitted B (bitmanip) and
//! C (RVC) profiles.
//!
//! `calc_riscvtests.rs` proves the corpus passes; this suite proves it is
//! COMPLETE: every whitelisted instruction FORM must actually have been
//! *executed*, not merely linked into an image. The proof reads the
//! interpreter's per-instruction PC trace (`interp::run_with_state`,
//! test/debug entry) and classifies every executed instruction:
//!
//! - profile B: each traced PC is decoded with the production decoder and
//!   mapped to its bitmanip form name; the union over all 43 upstream
//!   B-profile images must cover the whole 43-form whitelist.
//! - profile C: each traced PC's halfword is classified into one of the 34
//!   admitted RV64C encodings; the union over all profile-2 images (the
//!   65 auto-compressed `c_*` images + the calc-native `rvc_forms` kernel
//!   + the `rvc_ebreak`/`rvc_unimp` trap probes) must cover them all.
//!   `rvc_forms` alone deterministically covers all 32 non-trap forms with
//!   one checked case apiece (see fixtures/riscvtests/rvc_forms.S), so the
//!   guarantee does not depend on what clang happens to compress.
//!
//! Any decode-side whitelist change that drops a form without a matching
//! test update trips these assertions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use vm::calc::container::{CalcContainer, CalcSegment, SEG_X};
use vm::calc::decode::{
    decode_at, BitImmOp, Inst, RegAluOp, ShiftKind, UnAluOp,
};
use vm::calc::interp::{self, CalcAbort, CalcRun};
use vm::calc::verify::verify_container_with_cap;
use vm::calc::{pack_elf_min_mem, SpaceCap};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/riscvtests/elf")
}

fn corpus_profiles() -> BTreeMap<String, u8> {
    let txt = std::fs::read_to_string(corpus_dir().join("profiles.txt"))
        .expect("profiles.txt written by build.sh");
    txt.lines()
        .filter_map(|l| {
            let (name, prof) = l.split_once('=')?;
            Some((name.to_string(), prof.parse().ok()?))
        })
        .collect()
}

fn corpus_files() -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(corpus_dir())
        .expect("riscvtests corpus dir exists")
        .filter_map(|e| {
            let p = e.ok()?.path();
            if p.extension()?.to_str()? == "elf" {
                let name = p.file_stem()?.to_str()?.to_string();
                Some((name, p))
            } else {
                None
            }
        })
        .collect();
    files.sort();
    files
}

/// Images whose designed behavior is to end in a guest trap (see
/// fixtures/riscvtests/rvc_ebreak.S and rvc_unimp.S).
const TRAP_IMAGES: [&str; 2] = ["rvc_ebreak", "rvc_unimp"];

/// Pack → verify → run one image on the interpreter with the per-instruction
/// PC trace enabled.
fn run_traced(
    name: &str,
    profile: u8,
) -> (Result<CalcRun, (CalcAbort, u64)>, Vec<u64>, CalcContainer) {
    let elf = std::fs::read(corpus_dir().join(format!("{name}.elf")))
        .unwrap_or_else(|e| panic!("{name}: read failed: {e}"));
    let container = pack_elf_min_mem(&SpaceCap::new(0), &elf, profile, 0, 16)
        .unwrap_or_else(|e| panic!("{name}: pack failed: {}", e.1));
    let module =
        verify_container_with_cap(&container, &SpaceCap::new(0)).expect("container verifies");
    let (res, _regs, _mem, trace) = interp::run_with_state(&module, &container, &[], 5_000_000);
    (res, trace, container)
}

fn exec_segment(c: &CalcContainer) -> &CalcSegment {
    c.segs
        .iter()
        .find(|s| s.flags & SEG_X != 0)
        .expect("container has an executable segment")
}

// ------------------------------------------------------------ profile B ---

/// The 43 admitted bitmanip forms (Zba/Zbb/Zbs/Zbc), one name per decoder
/// instruction form; `rori`/`roriw` and the unary W forms count as forms of
/// their own (same names as the upstream suite files that exercise them).
const BITMANIP_FORMS: [&str; 43] = [
    // Zba
    "add.uw", "sh1add", "sh1add.uw", "sh2add", "sh2add.uw", "sh3add",
    "sh3add.uw", "slli.uw",
    // Zbb (incl the unary W forms and the immediate shifts)
    "andn", "clz", "clzw", "cpop", "cpopw", "ctz", "ctzw", "max", "maxu",
    "min", "minu", "orc.b", "orn", "rev8", "rol", "rolw", "ror", "rori",
    "roriw", "rorw", "sext.b", "sext.h", "xnor", "zext.h",
    // Zbs
    "bclr", "bclri", "bext", "bexti", "binv", "binvi", "bset", "bseti",
    // Zbc
    "clmul", "clmulh", "clmulr",
];

/// Map a decoded instruction to its bitmanip form name (None for
/// non-bitmanip instructions).
fn bitmanip_form(inst: &Inst) -> Option<&'static str> {
    match inst {
        Inst::RegAlu { op, .. } => match op {
            RegAluOp::Adduw => Some("add.uw"),
            RegAluOp::Sh1add => Some("sh1add"),
            RegAluOp::Sh1adduw => Some("sh1add.uw"),
            RegAluOp::Sh2add => Some("sh2add"),
            RegAluOp::Sh2adduw => Some("sh2add.uw"),
            RegAluOp::Sh3add => Some("sh3add"),
            RegAluOp::Sh3adduw => Some("sh3add.uw"),
            RegAluOp::Andn => Some("andn"),
            RegAluOp::Orn => Some("orn"),
            RegAluOp::Xnor => Some("xnor"),
            RegAluOp::Rol => Some("rol"),
            RegAluOp::Ror => Some("ror"),
            RegAluOp::Rolw => Some("rolw"),
            RegAluOp::Rorw => Some("rorw"),
            RegAluOp::Min => Some("min"),
            RegAluOp::Minu => Some("minu"),
            RegAluOp::Max => Some("max"),
            RegAluOp::Maxu => Some("maxu"),
            RegAluOp::Bset => Some("bset"),
            RegAluOp::Bclr => Some("bclr"),
            RegAluOp::Binv => Some("binv"),
            RegAluOp::Bext => Some("bext"),
            RegAluOp::Clmul => Some("clmul"),
            RegAluOp::Clmulh => Some("clmulh"),
            RegAluOp::Clmulr => Some("clmulr"),
            _ => None,
        },
        Inst::UnAlu { op, .. } => match op {
            UnAluOp::Clz => Some("clz"),
            UnAluOp::Ctz => Some("ctz"),
            UnAluOp::Cpop => Some("cpop"),
            UnAluOp::Sextb => Some("sext.b"),
            UnAluOp::Sexth => Some("sext.h"),
            UnAluOp::Zexth => Some("zext.h"),
            UnAluOp::Orcb => Some("orc.b"),
            UnAluOp::Rev8 => Some("rev8"),
            UnAluOp::Clzw => Some("clzw"),
            UnAluOp::Ctzw => Some("ctzw"),
            UnAluOp::Cpopw => Some("cpopw"),
        },
        Inst::BitImm { op, .. } => match op {
            BitImmOp::Set => Some("bseti"),
            BitImmOp::Clr => Some("bclri"),
            BitImmOp::Inv => Some("binvi"),
            BitImmOp::Ext => Some("bexti"),
        },
        Inst::Slliuw { .. } => Some("slli.uw"),
        Inst::ShiftImm {
            kind: ShiftKind::Ror,
            word,
            ..
        } => Some(if *word { "roriw" } else { "rori" }),
        _ => None,
    }
}

/// Every one of the 43 bitmanip forms must be executed by at least one
/// upstream profile-B image (proven from the interpreter trace).
#[test]
fn b_profile_corpus_executes_every_bitmanip_form() {
    let profiles = corpus_profiles();
    let mut executed: BTreeSet<&str> = BTreeSet::new();
    let mut b_images = 0usize;
    for (name, profile) in &profiles {
        if *profile != 1 {
            continue;
        }
        b_images += 1;
        let (res, trace, container) = run_traced(name, *profile);
        assert!(
            res.is_ok(),
            "{name}: B-profile image must HALT 0: {res:?}"
        );
        let seg = exec_segment(&container);
        let vaddr = seg.vaddr as u64;
        for pc in trace {
            let off = usize::try_from(pc - vaddr).expect("traced pc inside the X segment");
            let (inst, _) = decode_at(&seg.data, off, *profile)
                .unwrap_or_else(|e| panic!("{name}: traced pc {pc:#x} fails to decode: {}", e.1));
            if let Some(form) = bitmanip_form(&inst) {
                executed.insert(form);
            }
        }
    }
    assert_eq!(b_images, 43, "profile-B image count drifted");
    let missing: Vec<&str> = BITMANIP_FORMS
        .iter()
        .copied()
        .filter(|f| !executed.contains(f))
        .collect();
    assert!(
        missing.is_empty(),
        "bitmanip forms never executed by the corpus: {missing:?}"
    );
}

// ------------------------------------------------------------ profile C ---

/// Every admitted RV64C encoding the profile-2 decoder accepts, one variant
/// per compressed form (mirrors `decode_compressed` in vm/src/calc/decode.rs;
/// reserved/rejected encodings are decode-level tested in isa_tests and are
/// deliberately absent here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RvcForm {
    // quadrant 0
    Addi4spn,
    Lw,
    Ld,
    Sw,
    Sd,
    // quadrant 1
    Nop,
    Addi,
    Addiw,
    Li,
    Addi16sp,
    Lui,
    Srli,
    Srai,
    Andi,
    Sub,
    Xor,
    Or,
    And,
    Subw,
    Addw,
    J,
    Beqz,
    Bnez,
    // quadrant 2
    Slli,
    Lwsp,
    Ldsp,
    Jr,
    Mv,
    Jalr,
    Add,
    Ebreak,
    Swsp,
    Sdsp,
    /// the canonical all-zero halfword (C.UNIMP): admitted as Trap
    Unimp,
}

const ALL_RVC_FORMS: [RvcForm; 34] = [
    RvcForm::Addi4spn,
    RvcForm::Lw,
    RvcForm::Ld,
    RvcForm::Sw,
    RvcForm::Sd,
    RvcForm::Nop,
    RvcForm::Addi,
    RvcForm::Addiw,
    RvcForm::Li,
    RvcForm::Addi16sp,
    RvcForm::Lui,
    RvcForm::Srli,
    RvcForm::Srai,
    RvcForm::Andi,
    RvcForm::Sub,
    RvcForm::Xor,
    RvcForm::Or,
    RvcForm::And,
    RvcForm::Subw,
    RvcForm::Addw,
    RvcForm::J,
    RvcForm::Beqz,
    RvcForm::Bnez,
    RvcForm::Slli,
    RvcForm::Lwsp,
    RvcForm::Ldsp,
    RvcForm::Jr,
    RvcForm::Mv,
    RvcForm::Jalr,
    RvcForm::Add,
    RvcForm::Ebreak,
    RvcForm::Swsp,
    RvcForm::Sdsp,
    RvcForm::Unimp,
];

/// The 32 non-trap forms — exactly what the rvc_forms kernel covers.
const NON_TRAP_FORMS: [RvcForm; 32] = [
    RvcForm::Addi4spn,
    RvcForm::Lw,
    RvcForm::Ld,
    RvcForm::Sw,
    RvcForm::Sd,
    RvcForm::Nop,
    RvcForm::Addi,
    RvcForm::Addiw,
    RvcForm::Li,
    RvcForm::Addi16sp,
    RvcForm::Lui,
    RvcForm::Srli,
    RvcForm::Srai,
    RvcForm::Andi,
    RvcForm::Sub,
    RvcForm::Xor,
    RvcForm::Or,
    RvcForm::And,
    RvcForm::Subw,
    RvcForm::Addw,
    RvcForm::J,
    RvcForm::Beqz,
    RvcForm::Bnez,
    RvcForm::Slli,
    RvcForm::Lwsp,
    RvcForm::Ldsp,
    RvcForm::Jr,
    RvcForm::Mv,
    RvcForm::Jalr,
    RvcForm::Add,
    RvcForm::Swsp,
    RvcForm::Sdsp,
];

/// Classify a compressed halfword into its RV64C form, mirroring
/// `decode_compressed` (reserved encodings → None).
fn classify_rvc(h: u16) -> Option<RvcForm> {
    if h & 0b11 == 0b11 {
        return None; // 4-byte encoding
    }
    let quad = h & 0b11;
    let funct3 = h >> 13;
    let bit12 = (h >> 12) & 1;
    let rd = ((h >> 7) & 0b11111) as u8;
    let rs2 = ((h >> 2) & 0b11111) as u8;
    let imm5 = ((bit12 << 5) | ((h >> 2) & 0b11111)) as u32;
    let _ = imm5;
    let q0_rd = ((h >> 2) & 0b111) as u8 + 8;
    let q0_rs1 = ((h >> 7) & 0b111) as u8 + 8;
    let _ = q0_rs1;
    match quad {
        0 => match funct3 {
            0 => {
                // C.ADDI4SPN; imm == 0 is the all-zero/reserved trap encoding
                let imm = ((h >> 11) & 0b11) << 4
                    | ((h >> 7) & 0b1111) << 6
                    | ((h >> 6) & 1) << 2
                    | ((h >> 5) & 1) << 3;
                if imm == 0 {
                    Some(RvcForm::Unimp)
                } else {
                    let _ = q0_rd;
                    Some(RvcForm::Addi4spn)
                }
            }
            2 => Some(RvcForm::Lw),
            3 => Some(RvcForm::Ld),
            6 => Some(RvcForm::Sw),
            7 => Some(RvcForm::Sd),
            _ => None, // FP quadruple / reserved funct3
        },
        1 => match funct3 {
            0 => Some(if rd == 0 { RvcForm::Nop } else { RvcForm::Addi }),
            1 => {
                if rd == 0 {
                    None
                } else {
                    Some(RvcForm::Addiw)
                }
            }
            2 => Some(RvcForm::Li),
            3 => {
                if rd == 2 {
                    let imm = ((h >> 12) & 1) << 9
                        | ((h >> 6) & 1) << 4
                        | ((h >> 5) & 1) << 6
                        | ((h >> 3) & 0b11) << 7
                        | ((h >> 2) & 1) << 5;
                    if imm == 0 {
                        None
                    } else {
                        Some(RvcForm::Addi16sp)
                    }
                } else if rd == 0 {
                    None
                } else {
                    let v = ((bit12 << 5) | ((h >> 2) & 0b11111)) as i64;
                    let v = if v >= 32 { v - 64 } else { v };
                    if v == 0 {
                        None
                    } else {
                        Some(RvcForm::Lui)
                    }
                }
            }
            4 => match (h >> 10) & 0b11 {
                0 => Some(RvcForm::Srli),
                1 => Some(RvcForm::Srai),
                2 => Some(RvcForm::Andi),
                _ => match (bit12, (h >> 5) & 0b11) {
                    (0, 0) => Some(RvcForm::Sub),
                    (0, 1) => Some(RvcForm::Xor),
                    (0, 2) => Some(RvcForm::Or),
                    (0, 3) => Some(RvcForm::And),
                    (1, 0) => Some(RvcForm::Subw),
                    (1, 1) => Some(RvcForm::Addw),
                    _ => None,
                },
            },
            5 => Some(RvcForm::J),
            6 => Some(RvcForm::Beqz),
            7 => Some(RvcForm::Bnez),
            _ => unreachable!(),
        },
        2 => match funct3 {
            0 => Some(RvcForm::Slli),
            2 => {
                if rd == 0 {
                    None
                } else {
                    Some(RvcForm::Lwsp)
                }
            }
            3 => {
                if rd == 0 {
                    None
                } else {
                    Some(RvcForm::Ldsp)
                }
            }
            4 => {
                if bit12 == 0 {
                    if rs2 == 0 {
                        if rd == 0 {
                            None // c.jr rs1=0 reserved
                        } else {
                            Some(RvcForm::Jr)
                        }
                    } else {
                        Some(RvcForm::Mv)
                    }
                } else if rs2 == 0 {
                    if rd == 0 {
                        Some(RvcForm::Ebreak)
                    } else {
                        Some(RvcForm::Jalr)
                    }
                } else {
                    Some(RvcForm::Add)
                }
            }
            6 => Some(RvcForm::Swsp),
            7 => Some(RvcForm::Sdsp),
            _ => None, // FP quadruple
        },
        _ => unreachable!("quadrant 3 is a 4-byte prefix"),
    }
}

/// Classify the instruction executed at an absolute PC (profile-2 images
/// only: non-compressed encodings → None).
fn rvc_form_at(c: &CalcContainer, pc: u64) -> Option<RvcForm> {
    let seg = exec_segment(c);
    let off = usize::try_from(pc - seg.vaddr as u64).ok()?;
    let b = seg.data.get(off..off + 2)?;
    let h = u16::from_le_bytes([b[0], b[1]]);
    classify_rvc(h)
}

fn assert_trap_image(name: &str, want: RvcForm) {
    let (res, trace, container) = run_traced(name, 2);
    assert!(
        matches!(res, Err((CalcAbort::Trap(_), _))),
        "{name}: must abort with a guest trap, got {res:?}"
    );
    assert!(
        trace.iter().any(|pc| rvc_form_at(&container, *pc) == Some(want)),
        "{name}: trace does not contain the {want:?} encoding"
    );
}

/// The calc-native rvc_forms kernel deterministically executes all 32
/// non-trap RV64C forms with one checked case apiece, and the two trap
/// probes cover the remaining encodings (see fixtures/riscvtests/*.S).
/// Complete profile-C coverage therefore does not depend on what clang
/// happens to compress in the auto-compressed c_* images.
#[test]
fn rvc_kernel_executes_every_compressed_form() {
    let (res, trace, container) = run_traced("rvc_forms", 2);
    assert!(
        res.is_ok(),
        "rvc_forms: kernel must HALT 0: {res:?}"
    );
    let executed: BTreeSet<RvcForm> =
        trace.iter().filter_map(|pc| rvc_form_at(&container, *pc)).collect();
    let want: BTreeSet<RvcForm> = NON_TRAP_FORMS.iter().copied().collect();
    let missing: Vec<_> = want.difference(&executed).collect();
    assert!(
        missing.is_empty(),
        "rvc_forms kernel does not execute: {missing:?}"
    );
    assert_trap_image("rvc_ebreak", RvcForm::Ebreak);
    assert_trap_image("rvc_unimp", RvcForm::Unimp);
}

/// Whole-corpus census: the union of executed forms over ALL profile-2
/// images (66 auto-compressed c_* + the 3 calc-native kernels) must cover
/// every admitted RV64C encoding. This guards against toolchain drift
/// silently dropping a form from the auto-compressed images.
#[test]
fn c_profile_corpus_executes_every_compressed_form() {
    let profiles = corpus_profiles();
    let mut executed: BTreeSet<RvcForm> = BTreeSet::new();
    let mut c_images = 0usize;
    for (name, profile) in &profiles {
        if *profile != 2 {
            continue;
        }
        c_images += 1;
        let (res, trace, container) = run_traced(name, *profile);
        if TRAP_IMAGES.contains(&name.as_str()) {
            assert!(
                matches!(res, Err((CalcAbort::Trap(_), _))),
                "{name}: trap probe must abort with a guest trap, got {res:?}"
            );
        } else {
            assert!(res.is_ok(), "{name}: C-profile image must HALT 0: {res:?}");
        }
        for pc in trace {
            if let Some(form) = rvc_form_at(&container, pc) {
                executed.insert(form);
            }
        }
    }
    assert_eq!(c_images, 68, "profile-C image count drifted");
    let want: BTreeSet<RvcForm> = ALL_RVC_FORMS.iter().copied().collect();
    let missing: Vec<_> = want.difference(&executed).collect();
    assert!(
        missing.is_empty(),
        "RV64C encodings never executed by the profile-C corpus: {missing:?}"
    );
}
