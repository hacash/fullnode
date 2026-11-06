//! Optional RISC-V architecture-test harness for the calc interpreter.
//!
//! Ignored by default: the official `riscv-arch-test` suite is not
//! vendored. See `tests/archtest/README.md` for fetch/build/run.

use std::fs;
use std::path::{Path, PathBuf};

use vm::calc::container::CalcContainer;
use vm::calc::interp::{self, CalcAbort};
use vm::calc::verify::verify_container_with_cap;
use vm::calc::container::CALC_MEM_KIB_ENGINE_MAX;
use vm::calc::{SpaceCap, CALC_CODE_BASE, CALC_PROFILE_RV64IM};

fn images_dir() -> Option<PathBuf> {
    std::env::var_os("CALC_ARCHTEST_DIR").map(PathBuf::from)
}

/// Smallest `mem_kib` whose own layout clears the image (arch-test images
/// can be large, so this walks the engine ceiling rather than a test cap).
fn needed_mem_kib(guest_end: u64) -> Result<u32, String> {
    vm::calc::container::min_mem_kib_for(guest_end, 0, 4096)
        .ok_or_else(|| format!("image needs more than {CALC_MEM_KIB_ENGINE_MAX} KiB"))
}

fn pack(code: Vec<u8>, rodata: Vec<u8>, init: Vec<u8>, entry: u32) -> Result<CalcContainer, String> {
    pack_at(code, CALC_CODE_BASE, rodata, 0, init, 0, entry)
}

fn pack_at(
    code: Vec<u8>,
    code_va: u64,
    rodata: Vec<u8>,
    ro_va: u64,
    init: Vec<u8>,
    init_va: u64,
    entry: u32,
) -> Result<CalcContainer, String> {
    use vm::calc::container::CalcSegment;
    let mut segs = vec![CalcSegment::rx(code_va as u32, code)];
    if !rodata.is_empty() {
        segs.push(CalcSegment::ro(ro_va as u32, rodata));
    }
    if !init.is_empty() {
        let n = init.len() as u32;
        segs.push(CalcSegment::rw(init_va as u32, init, n));
    }
    segs.sort_by_key(|s| s.vaddr);
    let mut c = CalcContainer {
        profile: CALC_PROFILE_RV64IM,
        mem_kib: CALC_MEM_KIB_ENGINE_MAX,
        entry,
        segs,
        input_max: 0,
        output_max: 4096,
    };
    c.mem_kib = needed_mem_kib(c.guest_end())?;
    Ok(c)
}

fn read_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn read_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn read_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn elf_symbol(bytes: &[u8], want: &str) -> Option<u64> {
    if bytes.len() < 64 {
        return None;
    }
    let e_shoff = read_u64(bytes, 40) as usize;
    let e_shentsize = read_u16(bytes, 58) as usize;
    let e_shnum = read_u16(bytes, 60) as usize;
    if e_shentsize < 64 {
        return None;
    }
    let mut symtab: Option<(usize, usize, usize, usize)> = None; // off, size, entsize, link
    for i in 0..e_shnum {
        let o = e_shoff.checked_add(i * e_shentsize)?;
        if o + 64 > bytes.len() {
            return None;
        }
        let sh_type = read_u32(bytes, o + 4);
        if sh_type == 2 {
            let sh_offset = read_u64(bytes, o + 24) as usize;
            let sh_size = read_u64(bytes, o + 32) as usize;
            let sh_link = read_u32(bytes, o + 40) as usize;
            let sh_entsize = read_u64(bytes, o + 56) as usize;
            symtab = Some((sh_offset, sh_size, sh_entsize, sh_link));
            break;
        }
    }
    let (sym_off, sym_size, ent, link) = symtab?;
    if ent < 24 {
        return None;
    }
    let stro = e_shoff.checked_add(link * e_shentsize)?;
    if stro + 40 > bytes.len() {
        return None;
    }
    let str_off = read_u64(bytes, stro + 24) as usize;
    let str_size = read_u64(bytes, stro + 32) as usize;
    if str_off + str_size > bytes.len() {
        return None;
    }
    let strs = &bytes[str_off..str_off + str_size];
    let mut p = 0;
    while p + ent <= sym_size {
        let so = sym_off + p;
        if so + 24 > bytes.len() {
            break;
        }
        let st_name = read_u32(bytes, so) as usize;
        let st_value = read_u64(bytes, so + 8);
        if st_name < strs.len() {
            let end = strs[st_name..].iter().position(|&b| b == 0).unwrap_or(strs.len() - st_name);
            if let Ok(n) = std::str::from_utf8(&strs[st_name..st_name + end]) {
                if n == want {
                    return Some(st_value);
                }
            }
        }
        p += ent;
    }
    None
}

/// Map a little-endian RISC-V ELF64 linked at CODE_BASE into a calc container.
fn load_elf(bytes: &[u8], name: &str) -> Result<CalcContainer, String> {
    if bytes.len() < 64 || &bytes[0..4] != b"\x7fELF" {
        return Err(format!("{name}: not ELF"));
    }
    if bytes[4] != 2 || bytes[5] != 1 {
        return Err(format!("{name}: need ELF64LE"));
    }
    let e_machine = read_u16(bytes, 18);
    if e_machine != 243 {
        return Err(format!("{name}: e_machine {e_machine} is not RISC-V"));
    }
    let e_entry = read_u64(bytes, 24);
    let e_phoff = read_u64(bytes, 32) as usize;
    let e_phentsize = read_u16(bytes, 54) as usize;
    let e_phnum = read_u16(bytes, 56) as usize;
    let mut code: Option<(u64, Vec<u8>)> = None;
    let mut rodata: Option<(u64, Vec<u8>)> = None;
    let mut init: Option<(u64, Vec<u8>)> = None;
    for i in 0..e_phnum {
        let o = e_phoff + i * e_phentsize;
        if o + 56 > bytes.len() {
            return Err(format!("{name}: truncated phdr"));
        }
        let p_type = read_u32(bytes, o);
        if p_type != 1 {
            continue;
        }
        let p_flags = read_u32(bytes, o + 4);
        let p_offset = read_u64(bytes, o + 8) as usize;
        let p_vaddr = read_u64(bytes, o + 16);
        let p_filesz = read_u64(bytes, o + 32) as usize;
        let p_memsz = read_u64(bytes, o + 40) as usize;
        if p_offset + p_filesz > bytes.len() {
            return Err(format!("{name}: truncated segment"));
        }
        let exec = p_flags & 1 != 0;
        let write = p_flags & 2 != 0;
        // RX: filesz only — memsz padding is zeros, which decode as compressed.
        // RW: memsz so BSS zeros are part of init.
        let n = if exec { p_filesz } else { p_memsz };
        let mut seg = vec![0u8; n];
        let copy = p_filesz.min(n);
        seg[..copy].copy_from_slice(&bytes[p_offset..p_offset + copy]);
        if exec {
            if code.is_some() {
                return Err(format!("{name}: multiple RX segments"));
            }
            code = Some((p_vaddr, seg));
        } else if write {
            if init.is_some() {
                return Err(format!("{name}: multiple RW segments"));
            }
            init = Some((p_vaddr, seg));
        } else {
            if rodata.is_some() {
                return Err(format!("{name}: multiple R segments"));
            }
            rodata = Some((p_vaddr, seg));
        }
    }
    let (code_va, code) = code.ok_or_else(|| format!("{name}: no RX segment"))?;
    if code_va != CALC_CODE_BASE {
        return Err(format!(
            "{name}: code VMA {code_va:#x} != CODE_BASE {CALC_CODE_BASE:#x}"
        ));
    }
    if code.is_empty() || code.len() % 4 != 0 {
        return Err(format!("{name}: empty or mis-sized code ({})", code.len()));
    }
    let (ro_va, ro_bytes) = rodata.unwrap_or((0, Vec::new()));
    let (ini_va, ini_bytes) = init.unwrap_or((0, Vec::new()));
    if e_entry < CALC_CODE_BASE || e_entry >= CALC_CODE_BASE + code.len() as u64 || e_entry % 4 != 0
    {
        return Err(format!("{name}: entry {e_entry:#x} not in code"));
    }
    pack_at(code, CALC_CODE_BASE, ro_bytes, ro_va, ini_bytes, ini_va, e_entry as u32)
}

fn load_bin(path: &Path) -> Result<CalcContainer, String> {
    let code = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if code.is_empty() || code.len() % 4 != 0 {
        return Err(format!(
            "{}: code_len must be a positive multiple of 4",
            path.display()
        ));
    }
    let stem = path.with_extension("");
    let rodata = fs::read(stem.with_extension("rodata")).unwrap_or_default();
    let init = fs::read(stem.with_extension("dat")).unwrap_or_default();
    let entry = if let Ok(s) = fs::read_to_string(stem.with_extension("entry")) {
        u32::from_str_radix(s.trim().trim_start_matches("0x"), 16)
            .map_err(|e| format!("{}: entry: {e}", path.display()))?
    } else {
        CALC_CODE_BASE as u32
    };
    pack(code, rodata, init, entry)
}

fn parse_sig_text(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for (i, line) in s.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let h = t.trim_start_matches("0x").trim_start_matches("0X");
        if h.len() == 8 {
            let v = u32::from_str_radix(h, 16).map_err(|e| format!("sig line {i}: {e}"))?;
            out.extend_from_slice(&v.to_le_bytes());
        } else if h.len() == 16 {
            let v = u64::from_str_radix(h, 16).map_err(|e| format!("sig line {i}: {e}"))?;
            out.extend_from_slice(&v.to_le_bytes());
        } else {
            return Err(format!("sig line {i}: bad width {}", h.len()));
        }
    }
    Ok(out)
}

fn load_sig(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let p = path.with_extension("sig");
    if !p.exists() {
        return Ok(None);
    }
    let raw = fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    if raw.iter().all(|b| b.is_ascii_hexdigit() || b.is_ascii_whitespace()) {
        Ok(Some(parse_sig_text(&String::from_utf8_lossy(&raw))?))
    } else {
        Ok(Some(raw))
    }
}

/// `CalcFnObj::run` returns only the ABI output window. Arch-test signatures
/// are an ELF symbol span that can exceed `output_max`, so the guest memory
/// is read from the interpreter after the same `verify_container_with` the
/// object constructor uses.
fn verify_image(container: &CalcContainer) -> Result<vm::calc::verify::CalcModule, String> {
    // Arch-test images may exceed the L1 memory cap, so admit under the
    // engine ceiling. Object construction is the consensus deploy path
    // (verify); `run` is not used here because it returns only the ABI
    // output while the signature span lives elsewhere in guest memory.
    let cap = SpaceCap {
        calc_mem_kib_max: CALC_MEM_KIB_ENGINE_MAX,
        ..SpaceCap::new(0)
    };
    vm::calc::CalcFnObj::build(container.clone(), &cap, vm::calc::CalcHostConfig::new())
        .map_err(|e| format!("build: {e}"))?;
    verify_container_with_cap(container, &cap).map_err(|e| format!("verify: {e}"))
}

enum Outcome {
    Pass,
    Fail(String),
    Exclude(String),
}

#[test]
#[ignore = "set CALC_ARCHTEST_DIR to a directory of riscv-arch-test images; see tests/archtest/README.md"]
fn riscv_arch_test_rv64i_m() {
    let dir = images_dir().unwrap_or_else(|| {
        panic!("CALC_ARCHTEST_DIR is unset; see vm/tests/archtest/README.md")
    });
    assert!(
        dir.is_absolute(),
        "CALC_ARCHTEST_DIR must be an absolute path (cargo runs this test from vm/): {}",
        dir.display()
    );
    assert!(
        dir.is_dir(),
        "CALC_ARCHTEST_DIR is not a directory: {}",
        dir.display()
    );
    let ents: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();

    let mut results: Vec<(String, Outcome)> = Vec::new();
    let mut excludes: Vec<PathBuf> = ents
        .iter()
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("exclude"))
        .cloned()
        .collect();
    excludes.sort();
    for p in &excludes {
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        let reason = fs::read_to_string(p)
            .unwrap_or_default()
            .trim()
            .to_string();
        results.push((name, Outcome::Exclude(reason)));
    }

    let mut images: Vec<PathBuf> = ents
        .iter()
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("elf") | Some("bin")
            )
        })
        .cloned()
        .collect();
    images.sort();
    for img in images {
        let name = img.file_stem().unwrap().to_string_lossy().into_owned();
        if results.iter().any(|(n, _)| n == &name) {
            continue;
        }
        let is_elf = img.extension().and_then(|s| s.to_str()) == Some("elf");
        let elf_bytes = if is_elf {
            match fs::read(&img) {
                Ok(b) => Some(b),
                Err(e) => {
                    results.push((name, Outcome::Fail(format!("read: {e}"))));
                    continue;
                }
            }
        } else {
            None
        };
        let container = if let Some(b) = elf_bytes.as_deref() {
            match load_elf(b, &name) {
                Ok(c) => c,
                Err(e) => {
                    results.push((name, Outcome::Fail(e)));
                    continue;
                }
            }
        } else {
            match load_bin(&img) {
                Ok(c) => c,
                Err(e) => {
                    results.push((name, Outcome::Fail(e)));
                    continue;
                }
            }
        };
        let module = match verify_image(&container) {
            Ok(m) => m,
            Err(e) => {
                results.push((name, Outcome::Fail(e)));
                continue;
            }
        };
        let (run, _regs, mem, _trace) =
            interp::run_with_state(&module, &container, &[], 50_000_000);
        match run {
            Ok(_) => match load_sig(&img) {
                Ok(Some(want)) => {
                    let got = if let Some(b) = elf_bytes.as_deref() {
                        match (elf_symbol(b, "begin_signature"), elf_symbol(b, "end_signature")) {
                            (Some(lo), Some(hi)) if lo <= hi && (hi as usize) <= mem.len() => {
                                mem[lo as usize..hi as usize].to_vec()
                            }
                            (lo, hi) => {
                                results.push((
                                    name,
                                    Outcome::Fail(format!(
                                        "signature symbols begin={lo:?} end={hi:?} mem_len={}",
                                        mem.len()
                                    )),
                                ));
                                continue;
                            }
                        }
                    } else {
                        results.push((
                            name,
                            Outcome::Fail("raw .bin images need ELF symbols to dump the signature".into()),
                        ));
                        continue;
                    };
                    if got == want {
                        results.push((name, Outcome::Pass));
                    } else {
                        let n = got.len().min(want.len());
                        let off = (0..n).find(|&i| got[i] != want[i]);
                        results.push((
                            name,
                            Outcome::Fail(format!(
                                "signature mismatch: got {} bytes, want {} bytes, first_diff={off:?}",
                                got.len(),
                                want.len()
                            )),
                        ));
                    }
                }
                Ok(None) => results.push((name, Outcome::Pass)),
                Err(e) => results.push((name, Outcome::Fail(e))),
            },
            Err((CalcAbort::GuestExit(code), used)) => results.push((
                name,
                Outcome::Fail(format!("guest exit {code} after {used} cycles")),
            )),
            Err((abort, used)) => results.push((
                name,
                Outcome::Fail(format!("abort {abort:?} after {used} cycles")),
            )),
        }
    }

    let mut n_pass = 0usize;
    let mut n_fail = 0usize;
    let mut n_ex = 0usize;
    for (name, o) in &results {
        match o {
            Outcome::Pass => {
                n_pass += 1;
                eprintln!("PASS    {name}");
            }
            Outcome::Fail(r) => {
                n_fail += 1;
                eprintln!("FAIL    {name}: {r}");
            }
            Outcome::Exclude(r) => {
                n_ex += 1;
                eprintln!("EXCLUDE {name}: {r}");
            }
        }
    }
    eprintln!("riscv-arch-test: {n_pass} pass, {n_fail} fail, {n_ex} excluded (justified)");
    assert!(
        n_pass + n_fail + n_ex > 0,
        "no images or exclusions in {} (see tests/archtest/README.md)",
        dir.display()
    );
    assert!(
        n_fail == 0,
        "{n_fail} riscv-arch-test image(s) failed (see EXCLUDE list for justified gaps)"
    );
}
