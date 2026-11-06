//! riscv-tests conformance corpus (rv64ui + rv64um + rv64uzb* + rv64uc)
//! executed on the calc VM.
//!
//! Upstream: https://github.com/riscv-software-src/riscv-tests — the ISA
//! test suites for RV64I (`rv64ui`), RV64M (`rv64um`), the bitmanip
//! extensions (`rv64uzbb/zba/zbs/zbc` → profile B) and compressed
//! instructions (`rv64uc` → profile C). Sources are NOT vendored (see
//! `elf/README.md`); the built ELF images under `elf/` were produced by
//! `build.sh` with a calc-specific environment header (`riscv_test.h`):
//! no CSRs, no tohost — the pass/fail gate is the calc ecall ABI itself:
//!
//! - pass: `HALT(a0=0, a1=0)` → `CalcFnObj::run` returns `Ok`
//! - fail: `HALT(a0=test_number, a1=0)` → `GuestExit(n)` → abort `Trap`
//!   with `exit_code = n`, so a failing test names its own test number.
//!
//! Every image goes through the production pack path (`pack_elf_min_mem`
//! with the suite's profile byte), chain-cap verification and — under
//! `calc-aot` — the interpreter/AOT lockstep, which since the B/C-admission
//! change compares the FULL stop state (output, gas, all 32 registers and
//! the whole memory image). Any disagreement fails as `EngineMismatch`.
//! Profile-B images exercise the AOT bitmanip translation; profile-C images
//! exercise its compressed-instruction translation.

use std::collections::HashMap;
use std::path::PathBuf;

use vm::calc::{pack_elf_min_mem, CalcFnObj, CalcRunLimits, SpaceCap};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/riscvtests/elf")
}

/// name -> suite profile byte (written by build.sh as `profiles.txt`).
fn corpus_profiles() -> HashMap<String, u8> {
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

/// Run one corpus image end to end: pack with its suite profile → verify →
/// build (with AOT mount when the feature is on) → run. Returns
/// (cycles_used, exit_code) on a successful HALT.
fn run_one(name: &str, path: &PathBuf, profile: u8) -> Result<(u64, u64), String> {
    let elf = std::fs::read(path).unwrap_or_else(|e| panic!("{name}: read failed: {e}"));
    let container = pack_elf_min_mem(&SpaceCap::new(0), &elf, profile, 0, 16)
        .map_err(|e| format!("{name}: pack failed: {}", e.1))?;
    let obj = CalcFnObj::build_with_aot(container)
        .map_err(|e| format!("{name}: build/verify failed: {}", e.1))?;
    let exec = obj.run(
        &[],
        CalcRunLimits {
            cycles: 5_000_000,
            resource: -1,
            deadline: None,
        },
    );
    match exec.result {
        Ok(_) => Ok((exec.cycles_used, 0)),
        Err(abort) => Err(format!(
            "{name}: FAILED at test number {} ({} cycles): {abort:?}",
            exec.exit_code, exec.cycles_used
        )),
    }
}

/// Images that end in a guest trap by design (the C.EBREAK and all-zero
/// c.unimp encodings, see fixtures/riscvtests/rvc_ebreak.S / rvc_unimp.S):
/// the expected outcome is the `Trap` abort, not a HALT.
const TRAP_IMAGES: [&str; 2] = ["rvc_ebreak", "rvc_unimp"];

/// The whole corpus must pass: interpreter result and (mounted) AOT agree —
/// including on the full register/memory stop state — and every HALT exits
/// with code 0. The two trap-probe images must abort with `Trap` instead.
#[test]
fn riscvtests_conformance_corpus_passes() {
    let profiles = corpus_profiles();
    let files = corpus_files();
    assert!(files.len() >= 176, "corpus truncated: {}", files.len());
    let mut failures = Vec::new();
    for (name, path) in &files {
        let profile = profiles
            .get(name)
            .copied()
            .unwrap_or_else(|| panic!("{name}: missing from profiles.txt"));
        if TRAP_IMAGES.contains(&name.as_str()) {
            let container = match pack_elf_min_mem(&SpaceCap::new(0), &std::fs::read(path).expect("read"), profile, 0, 16) {
                Ok(c) => c,
                Err(e) => {
                    failures.push(format!("{name}: pack failed: {}", e.1));
                    continue;
                }
            };
            let obj = match CalcFnObj::build_with_aot(container) {
                Ok(o) => o,
                Err(e) => {
                    failures.push(format!("{name}: build/verify failed: {}", e.1));
                    continue;
                }
            };
            let exec = obj.run(
                &[],
                CalcRunLimits {
                    cycles: 5_000_000,
                    resource: -1,
                    deadline: None,
                },
            );
            if !matches!(exec.result, Err(vm::calc::CalcAbort2::Trap)) {
                failures.push(format!(
                    "{name}: expected a guest-trap abort, got {:?} (exit {})",
                    exec.result, exec.exit_code
                ));
            }
            continue;
        }
        if let Err(msg) = run_one(name, path, profile) {
            failures.push(msg);
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} riscv-tests images failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

/// Per-family smoke: one image per suite must charge cycles (execution
/// actually happened, not an instant-trap).
#[test]
fn corpus_execution_charges_cycles() {
    let profiles = corpus_profiles();
    for name in ["add", "mul", "clz", "sh1add_uw", "bseti", "clmul", "c_add", "c_div"] {
        let path = corpus_dir().join(format!("{name}.elf"));
        let profile = profiles.get(name).copied().expect("suite in profiles.txt");
        let (cycles, _) = run_one(name, &path, profile).unwrap();
        assert!(cycles > 0, "{name} charged no cycles");
    }
}

/// Opcode coverage manifest: every opcode-decided upstream test of the
/// admitted whitelist must be present in the built corpus, per profile.
///
/// Forms covered inside another image (still executed, just not
/// file-named): `rori`/`roriw` run inside `ror`/`rorw`, `clzw`/`ctzw`/
/// `cpopw` run inside `clz`/`ctz`/`cpop` (xlen==64 branches). Not covered
/// by this corpus by upstream design (covered elsewhere, as noted):
/// - `fence`: no dedicated upstream rv64ui test — pinned by
///   `calc_plan_coverage::cycle_charges_match_weights` and the decode
///   vectors;
/// - `ecall`/`ebreak`/`unimp`: ecall ABI matrix + trap tests in
///   `calc_plan_coverage` and `isa_tests`;
/// - `fence_i`/`ma_data`: excluded from the build (see build.sh);
/// - `brev8`/`pack`/`packh`/`packw` (rv64uzbkb): not in the profile-B
///   whitelist.
#[test]
fn corpus_covers_the_whole_admitted_whitelist() {
    let rv64i = [
        "add", "addi", "addiw", "addw", //
        "and", "andi", "auipc", //
        "beq", "bge", "bgeu", "blt", "bltu", "bne", //
        "jal", "jalr", //
        "lb", "lbu", "ld", "lh", "lhu", "lui", "lw", "lwu", //
        "or", "ori", //
        "sb", "sd", "sh", "sw", "sll", "slli", "slliw", "sllw", //
        "slt", "slti", "sltiu", "sltu", //
        "sra", "srai", "sraiw", "sraw", "srl", "srli", "srliw", "srlw", //
        "sub", "subw", "xor", "xori",
    ];
    let rv64m = [
        "div", "divu", "divuw", "divw", "mul", "mulh", "mulhsu", "mulhu",
        "mulw", "rem", "remu", "remuw", "remw",
    ];
    // profile B (Zba/Zbb/Zbs/Zbc): one upstream suite per extension; the
    // image set covers every whitelisted bitmanip form (see doc comment).
    let bitmanip = [
        // rv64uzbb (Zbb + the W/unary forms)
        "andn", "clz", "clzw", "cpop", "cpopw", "ctz", "ctzw", "max", "maxu",
        "min", "minu", "orc_b", "orn", "rev8", "rol", "rolw", "rori", "roriw",
        "ror", "rorw", "sext_b", "sext_h", "xnor", "zext_h",
        // rv64uzba
        "add_uw", "sh1add", "sh1add_uw", "sh2add", "sh2add_uw", "sh3add",
        "sh3add_uw", "slli_uw",
        // rv64uzbs
        "bclr", "bclri", "bext", "bexti", "binv", "binvi", "bset", "bseti",
        // rv64uzbc
        "clmul", "clmulh", "clmulr",
    ];
    // profile C: the RV64I/M suites rebuilt with `-march=rv64imc` under a
    // c_ prefix — clang auto-compresses them into RVC forms, so every image
    // executes compressed instructions. (Upstream rvc.S is excluded: its
    // first case embeds .dword data inside the executable segment, which
    // the calc verifier deliberately rejects — see build.sh / README.
    // The calc-native rvc_forms/rvc_ebreak/rvc_unimp kernels take over the
    // per-form RVC conformance duty; execution-level form coverage is
    // proven by calc_isa_forms.rs.)
    let c_prefixed: Vec<String> = rv64i
        .iter()
        .chain(rv64m.iter())
        .map(|n| format!("c_{n}"))
        .collect();

    let rvc_kernels = ["rvc_forms", "rvc_ebreak", "rvc_unimp"];

    let dir = corpus_dir();
    let mut missing = Vec::new();
    let mut all_names: Vec<String> = Vec::new();
    all_names.extend(rv64i.iter().map(|s| s.to_string()));
    all_names.extend(rv64m.iter().map(|s| s.to_string()));
    all_names.extend(bitmanip.iter().map(|s| s.to_string()));
    all_names.extend(c_prefixed.iter().cloned());
    all_names.extend(rvc_kernels.iter().map(|s| s.to_string()));
    for name in &all_names {
        if !dir.join(name).with_extension("elf").exists() {
            missing.push(name.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "admitted-whitelist opcodes without a riscv-tests corpus image: {missing:?}"
    );
    let expected = rv64i.len()
        + rv64m.len()
        + bitmanip.len()
        + c_prefixed.len()
        + 6      // + simple, ld_st, st_ld and their c_ twins
        + rvc_kernels.len();
    assert_eq!(
        corpus_files().len(),
        expected,
        "corpus file set drifted from the upstream suites"
    );
}
