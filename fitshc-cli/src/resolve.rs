//! Request resolution: turn collected `function rv64im` declarations into
//! concrete kernel sources, mirroring the priority order of the design doc.
//!
//! For a declaration `function rv64im <name>(...)` without `= 0x...`, in the
//! directory of the `.fitsh` file:
//!
//! 1. `--calc <name>=<path>` — explicit override; the path may be a packed
//!    `.hcf1` container or a raw `.elf` (packed with the CLI IO flags or the
//!    defaults).
//! 2. `<name>.rs` — auto-compiled rust kernel (source of truth; skipped with
//!    `--no-build`).
//! 3. `calc/<name>.rs` — directory-grouped variant.
//! 4. `<name>.hcf1` — pre-packed container, decoded and verified.
//! 5. `<name>.elf` — pre-compiled ELF, packed with the CLI IO flags or the
//!    defaults.
//!
//! Source files win over prebuilt artifacts so a stale `.hcf1` can never be
//! deployed by accident; CI and rustc-less environments use `--no-build` or
//! `--calc`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use vm::calc::SpaceCap;
use vm::calc::container::CalcContainer;
use vm::calc::verify::verify_container_with_cap;
use vm::fitshc::CalcRequest;

use crate::meta::{KernelMeta, MemSpec, ProfileName, parse_kernel_meta};

/// Packing parameters used for bare `.elf` inputs (`--calc name=x.elf`,
/// `<name>.elf` fallback, and `fitshc pack`). An ELF carries no `// calc:`
/// header, so the CLI flags land here; they mirror the metadata defaults.
#[derive(Debug, Clone)]
pub struct ElfDefaults {
    pub profile: ProfileName,
    pub input_max: u16,
    pub output_max: u16,
    pub no_host_ecall: bool,
    /// Explicit memory admission or minimum-fit `auto`.
    pub mem: MemSpec,
}

impl Default for ElfDefaults {
    fn default() -> Self {
        // Mirrors the `// calc:` metadata defaults.
        ElfDefaults {
            profile: ProfileName::Rv64imc,
            input_max: 1024,
            output_max: 1024,
            no_host_ecall: true,
            mem: MemSpec::Auto,
        }
    }
}

/// How a request was satisfied.
#[derive(Debug, Clone)]
pub enum Resolution {
    /// Rust kernel source, auto-compiled by the build engine.
    Rs {
        path: PathBuf,
        meta: KernelMeta,
        has_input: bool,
    },
    /// Packed container file (`.hcf1` or an explicit override of one).
    Container(PathBuf),
    /// Raw ELF, packed later with the CLI's `ElfDefaults`.
    Elf(PathBuf),
}

impl Resolution {
    pub fn origin(&self) -> &'static str {
        match self {
            Resolution::Rs { .. } => "rust-source",
            Resolution::Container(_) => "prebuilt-container",
            Resolution::Elf { .. } => "prebuilt-elf",
        }
    }
}

/// Resolve every request. Duplicated names (a compile error later in
/// `compile_with_rv64im`) are collapsed here.
pub fn resolve_requests(
    fitsh_path: &Path,
    requests: &[CalcRequest],
    overrides: &BTreeMap<String, PathBuf>,
    no_build: bool,
) -> Result<Vec<(String, Resolution)>, String> {
    let dir = fitsh_path.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for req in requests {
        if !seen.insert(req.name.clone()) {
            continue;
        }
        let res = resolve_one(dir, req, overrides, no_build)?;
        out.push((req.name.clone(), res));
    }
    Ok(out)
}

fn resolve_one(
    dir: &Path,
    req: &CalcRequest,
    overrides: &BTreeMap<String, PathBuf>,
    no_build: bool,
) -> Result<Resolution, String> {
    let name = &req.name;
    let mut searched: Vec<String> = Vec::new();

    // 1. explicit override
    if let Some(path) = overrides.get(name) {
        return classify_override(path, req);
    }

    // 2./3. rust sources (skipped with --no-build)
    if !no_build {
        for cand in [
            dir.join(format!("{name}.rs")),
            dir.join(format!("calc/{name}.rs")),
        ] {
            searched.push(cand.display().to_string());
            if cand.is_file() {
                let source = std::fs::read_to_string(&cand)
                    .map_err(|e| format!("read {}: {e}", cand.display()))?;
                let meta = parse_kernel_meta(&cand, &source)?;
                return Ok(Resolution::Rs {
                    path: cand,
                    meta,
                    has_input: req.has_input,
                });
            }
        }
    }

    // 4. pre-packed container
    let hcf1 = dir.join(format!("{name}.hcf1"));
    searched.push(hcf1.display().to_string());
    if hcf1.is_file() {
        return Ok(Resolution::Container(hcf1));
    }

    // 5. pre-compiled ELF
    let elf = dir.join(format!("{name}.elf"));
    searched.push(elf.display().to_string());
    if elf.is_file() {
        return Ok(Resolution::Elf(elf));
    }

    let sig = if req.has_input {
        format!("pub fn {name}(input: &[u8], out: &mut [u8]) -> usize")
    } else {
        format!("pub fn {name}(out: &mut [u8]) -> usize")
    };
    Err(format!(
        "no kernel found for 'function rv64im {name}'\n  searched:\n    {}\n  \
         provide {name}.rs defining `{sig}` (see vm/doc/fitshc-calc.loc.md §2), \
         or a {name}.hcf1 / {name}.elf artifact, or pass --calc {name}=<path>",
        searched.join("\n    ")
    ))
}

fn classify_override(path: &Path, req: &CalcRequest) -> Result<Resolution, String> {
    if !path.is_file() {
        return Err(format!("--calc override {} does not exist", path.display()));
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => {
            let source = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let meta = parse_kernel_meta(path, &source)?;
            Ok(Resolution::Rs {
                path: path.to_path_buf(),
                meta,
                has_input: req.has_input,
            })
        }
        Some("hcf1") | Some("hcf") => Ok(Resolution::Container(path.to_path_buf())),
        Some("elf") => Ok(Resolution::Elf(path.to_path_buf())),
        other => Err(format!(
            "--calc override {} has unsupported type {:?} (expected .rs, .hcf1 or .elf)",
            path.display(),
            other
        )),
    }
}

/// Load and verify a `.hcf1` container file for injection.
pub fn load_container_file(
    path: &Path,
    cap: &SpaceCap,
) -> Result<(CalcContainer, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let (c, used) =
        CalcContainer::decode(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if used != bytes.len() {
        return Err(format!(
            "{}: trailing bytes after container",
            path.display()
        ));
    }
    verify_container_with_cap(&c, cap).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((c, bytes))
}

/// Pack a `.elf` file with the effective `ElfDefaults` (profile, IO sizes,
/// `mem`), verified under `cap`.
pub fn pack_elf_file(
    path: &Path,
    elf_defaults: &ElfDefaults,
    cap: &SpaceCap,
) -> Result<(CalcContainer, Vec<u8>), String> {
    let elf = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let meta = KernelMeta {
        profile: elf_defaults.profile,
        input_max: elf_defaults.input_max,
        output_max: elf_defaults.output_max,
        mem_kib: elf_defaults.mem,
        no_host_ecall: elf_defaults.no_host_ecall,
    };
    meta.validate(cap)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let c = match elf_defaults.mem {
        crate::meta::MemSpec::Auto => vm::calc::pack_elf_min_mem(
            cap,
            &elf,
            meta.profile_byte(),
            meta.input_max,
            meta.output_max,
        ),
        crate::meta::MemSpec::KiB(kib) => vm::calc::pack_elf(
            cap,
            &elf,
            meta.profile_byte(),
            kib,
            meta.input_max,
            meta.output_max,
        ),
    }
    .map_err(|e| format!("{}: {e}", path.display()))?;
    verify_container_with_cap(&c, cap).map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = c.encode().map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((c, bytes))
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fitshc-resolve-{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn req(name: &str, has_input: bool) -> CalcRequest {
        CalcRequest {
            name: name.to_string(),
            has_input,
        }
    }

    #[test]
    fn rs_wins_over_hcf1_and_elf() {
        let d = tmpdir("priority");
        fs::write(
            d.join("hash.rs"),
            "// calc: profile = rv64im\npub fn hash(_: &[u8], _: &mut [u8]) -> usize { 0 }",
        )
        .unwrap();
        fs::write(d.join("hash.hcf1"), b"stale").unwrap();
        fs::write(d.join("hash.elf"), b"\x7fELFstub").unwrap();
        let out = resolve_requests(
            &d.join("c.fitsh"),
            &[req("hash", true)],
            &BTreeMap::new(),
            false,
        )
        .unwrap();
        assert!(matches!(out[0].1, Resolution::Rs { .. }));
    }

    #[test]
    fn calc_subdir_is_searched() {
        let d = tmpdir("subdir");
        fs::create_dir_all(d.join("calc")).unwrap();
        fs::write(
            d.join("calc/hash.rs"),
            "pub fn hash(_: &[u8], _: &mut [u8]) -> usize { 0 }",
        )
        .unwrap();
        let out = resolve_requests(
            &d.join("c.fitsh"),
            &[req("hash", true)],
            &BTreeMap::new(),
            false,
        )
        .unwrap();
        let Resolution::Rs { path, meta, .. } = &out[0].1 else {
            panic!("expected Rs");
        };
        assert!(path.ends_with("calc/hash.rs"));
        assert_eq!(meta.profile, ProfileName::Rv64imc);
    }

    #[test]
    fn no_build_skips_rs_and_falls_through() {
        let d = tmpdir("nobuild");
        fs::write(
            d.join("hash.rs"),
            "pub fn hash(_: &[u8], _: &mut [u8]) -> usize { 0 }",
        )
        .unwrap();
        fs::write(d.join("hash.elf"), b"\x7fELFstub").unwrap();
        let out = resolve_requests(
            &d.join("c.fitsh"),
            &[req("hash", true)],
            &BTreeMap::new(),
            true,
        )
        .unwrap();
        assert!(matches!(out[0].1, Resolution::Elf { .. }));

        let d2 = tmpdir("nobuild-hcf");
        fs::write(
            d2.join("hash.rs"),
            "pub fn hash(_: &[u8], _: &mut [u8]) -> usize { 0 }",
        )
        .unwrap();
        fs::write(d2.join("hash.hcf1"), b"HCF1").unwrap();
        let out2 = resolve_requests(
            &d2.join("c.fitsh"),
            &[req("hash", true)],
            &BTreeMap::new(),
            true,
        )
        .unwrap();
        assert!(matches!(out2[0].1, Resolution::Container(_)));
    }

    #[test]
    fn override_beats_everything() {
        let d = tmpdir("override");
        fs::write(
            d.join("hash.rs"),
            "pub fn hash(_: &[u8], _: &mut [u8]) -> usize { 0 }",
        )
        .unwrap();
        let ext = tmpdir("override-ext");
        let extpath = ext.join("packed.hcf1");
        fs::write(&extpath, b"HCF1").unwrap();
        let mut ov = BTreeMap::new();
        ov.insert("hash".to_string(), extpath.clone());
        let out = resolve_requests(&d.join("c.fitsh"), &[req("hash", true)], &ov, false).unwrap();
        assert!(matches!(&out[0].1, Resolution::Container(p) if *p == extpath));
    }

    #[test]
    fn missing_kernel_lists_searched_paths_and_signature() {
        let d = tmpdir("missing");
        let err = resolve_requests(
            &d.join("c.fitsh"),
            &[req("hash", false)],
            &BTreeMap::new(),
            false,
        )
        .unwrap_err();
        assert!(err.contains("hash.rs"), "{err}");
        assert!(err.contains("calc/hash.rs"), "{err}");
        assert!(err.contains("hash.hcf1"), "{err}");
        assert!(
            err.contains("pub fn hash(out: &mut [u8]) -> usize"),
            "{err}"
        );
        assert!(err.contains("--calc hash="), "{err}");
    }

    #[test]
    fn meta_errors_from_rs_carry_through() {
        let d = tmpdir("badmeta");
        fs::write(
            d.join("hash.rs"),
            "// calc: profile = mips\npub fn hash(_: &[u8], _: &mut [u8]) -> usize { 0 }",
        )
        .unwrap();
        let err = resolve_requests(
            &d.join("c.fitsh"),
            &[req("hash", true)],
            &BTreeMap::new(),
            false,
        )
        .unwrap_err();
        assert!(err.contains("unknown profile"), "{err}");
    }
}
