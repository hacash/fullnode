//! End-to-end kernel build tests. Ignored by default: they drive the real
//! rust/cargo toolchain (stable + `RUSTC_BOOTSTRAP=1` for the build-std
//! recipes and the `riscv64imac-unknown-none-elf` target must be installed —
//! `rustup target add riscv64imac-unknown-none-elf`, `rustup component add
//! rust-src`). Run with:
//!
//! ```sh
//! cargo test -p fitshc-cli --test e2e_build -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fitshc_cli::meta::{MemSpec, ProfileName, parse_kernel_meta};
use fitshc_cli::rustbuild::KernelBuildRequest;
use fitshc_cli::{BuildConfig, compile_fitsh_with_calcs, run_container};
use vm::CodeType;
use vm::calc::SpaceCap;

/// Deterministic reference implementation of the test kernel (mirrored in
/// the generated `*.rs` below) so the test can check real execution output.
fn mix64(input: &[u8]) -> u64 {
    let mut h: u64 = 0x9e3779b97f4a7c15;
    for (i, b) in input.iter().enumerate() {
        h ^= (*b as u64).wrapping_mul(0x0100_0000_01b3);
        h = h.rotate_left(13);
        if i % 8 == 7 {
            h ^= h >> 31;
        }
    }
    h
}

fn kernel_source(profile: ProfileName) -> String {
    format!(
        "// calc: profile = {}\n\
         // calc: input_max = 1024\n\
         // calc: output_max = 64\n\n\
         pub fn hash(input: &[u8], out: &mut [u8]) -> usize {{\n\
         \x20   let mut h: u64 = 0x9e3779b97f4a7c15;\n\
         \x20   for (i, b) in input.iter().enumerate() {{\n\
         \x20       h ^= (*b as u64).wrapping_mul(0x0100_0000_01b3);\n\
         \x20       h = h.rotate_left(13);\n\
         \x20       if i % 8 == 7 {{ h ^= h >> 31; }}\n\
         \x20   }}\n\
         \x20   let n = 8.min(out.len());\n\
         \x20   for j in 0..n {{ out[j] = (h >> (j * 8)) as u8; }}\n\
         \x20   n\n\
         }}\n",
        profile.as_str()
    )
}

struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(tag: &str) -> Project {
        let dir = std::env::temp_dir().join(format!("fitshc-e2e-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("calc")).unwrap();
        Project { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    fn cache(&self) -> PathBuf {
        self.dir.join(".cache")
    }
}

const INPUT: &[u8] = b"hacash rv64im kernel e2e";

fn expected_output() -> Vec<u8> {
    mix64(INPUT).to_le_bytes().to_vec()
}

fn build_and_run(tag: &str, profile: ProfileName, strict: bool) -> (Vec<u8>, bool, u32) {
    let proj = Project::new(tag);
    let src = proj.write("hash.rs", &kernel_source(profile));
    let meta = parse_kernel_meta(&src, &kernel_source(profile)).unwrap();
    let cap = SpaceCap::new(0);
    let built = KernelBuildRequest {
        name: "hash",
        source_path: &src,
        meta: &meta,
        has_input: true,
        cap: &cap,
        strict,
        cache_root: Some(proj.cache()),
    }
    .build()
    .expect("kernel builds");
    assert_eq!(built.container.profile & 0b011, profile.profile_bits());
    let (out, _cycles, _) = run_container(built.container.clone(), INPUT, 1_000_000).unwrap();
    assert_eq!(
        out,
        expected_output(),
        "profile {profile:?} strict={strict}"
    );
    (out, built.cache_hit, built.mem_kib) // first build is never a hit
}

#[test]
#[ignore = "drives the real rust toolchain"]
fn all_profiles_build_verify_and_execute() {
    let _ = build_and_run("p0", ProfileName::Rv64im, false);
    let _ = build_and_run("p1", ProfileName::Rv64imb, false);
    let _ = build_and_run("p2", ProfileName::Rv64imc, false);
    let _ = build_and_run("p3", ProfileName::Rv64imcb, false);
    // strict variants of the C profiles (build-std path)
    let _ = build_and_run("p2s", ProfileName::Rv64imc, true);
    let _ = build_and_run("p3s", ProfileName::Rv64imcb, true);
}

#[test]
#[ignore = "drives the real rust toolchain"]
fn second_build_hits_cache_and_meta_change_invalidates() {
    let proj = Project::new("cache");
    let cap = SpaceCap::new(0);

    let source = kernel_source(ProfileName::Rv64im);
    let src = proj.write("hash.rs", &source);
    let meta = parse_kernel_meta(&src, &source).unwrap();
    let build = || {
        KernelBuildRequest {
            name: "hash",
            source_path: &src,
            meta: &meta,
            has_input: true,
            cap: &cap,
            strict: false,
            cache_root: Some(proj.cache()),
        }
        .build()
        .unwrap()
    };
    let first = build();
    assert!(!first.cache_hit, "first build must compile");
    let second = build();
    assert!(second.cache_hit, "identical request must hit the cache");
    assert_eq!(first.container_bytes, second.container_bytes);

    // A metadata change must rebuild and produce a different image.
    let changed = source.replace("input_max = 1024", "input_max = 512");
    let src2 = proj.write("hash.rs", &changed);
    let meta2 = parse_kernel_meta(&src2, &changed).unwrap();
    let third = KernelBuildRequest {
        name: "hash",
        source_path: &src2,
        meta: &meta2,
        has_input: true,
        cap: &cap,
        strict: false,
        cache_root: Some(proj.cache()),
    }
    .build()
    .unwrap();
    assert!(!third.cache_hit);
    assert_ne!(first.container_bytes, third.container_bytes);
    assert_eq!(third.container.input_max, 512);
}

#[test]
#[ignore = "drives the real rust toolchain"]
fn zero_arg_kernel_and_contract_injection() {
    let proj = Project::new("contract");
    // Zero-arg kernel: no input parameter.
    proj.write(
        "tick.rs",
        "// calc: profile = rv64im\npub fn tick(out: &mut [u8]) -> usize {\n    out[0] = 0x2a;\n    1\n}\n",
    );
    // One-arg kernel in the calc/ subdirectory.
    proj.write("calc/hash.rs", &kernel_source(ProfileName::Rv64imc));
    let fitsh = proj.write(
        "demo.fitsh",
        r#"pragma fitsh 1.0.0
contract Demo {
    function rv64im hash(data: bytes) -> bytes
    function rv64im tick() -> bytes
    function external fingerprint(x: bytes) -> bytes { return self::hash(x) }
}
"#,
    );
    let source = std::fs::read_to_string(&fitsh).unwrap();
    let cfg = BuildConfig {
        cache_root: Some(proj.cache()),
        ..BuildConfig::default()
    };
    let cap = SpaceCap::new(0);
    let ((contract, _, _, name), artifacts, _) =
        compile_fitsh_with_calcs(&source, &fitsh, &cfg, &cap).expect("contract compiles");
    assert_eq!(name, "Demo");
    assert_eq!(artifacts.len(), 2);

    let sto = contract.into_sto();
    for (fname, want) in [("hash", expected_output()), ("tick", vec![0x2a])] {
        let f = sto
            .userfuncs
            .iter()
            .find(|f| f.sign.0 == vm::calc_func_sign(fname))
            .unwrap_or_else(|| panic!("{fname} userfunc"));
        assert_eq!(f.code_stuff.conf.uint(), CodeType::RV64IM as u8);
        let (c, _) = vm::calc::container::CalcContainer::decode(&f.code_stuff.data).unwrap();
        let (out, _, _) = run_container(c, INPUT, 1_000_000).unwrap();
        assert_eq!(out, want, "{fname}");
    }
    // Artifacts written next to the fitsh.
    assert!(proj.path("hash.hcf1").is_file());
    assert!(proj.path("hash.calc.manifest.json").is_file());
    assert!(proj.path("tick.hcf1").is_file());
}

#[test]
#[ignore = "drives the real rust toolchain"]
fn no_build_uses_hcf1_and_elf_fallbacks() {
    let proj = Project::new("prebuilt");
    // Build once to produce a .hcf1 next to the fitsh.
    proj.write("hash.rs", &kernel_source(ProfileName::Rv64im));
    let fitsh = proj.write(
        "demo.fitsh",
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im hash(data: bytes) -> bytes\n}\n",
    );
    let source = std::fs::read_to_string(&fitsh).unwrap();
    let cfg = BuildConfig {
        cache_root: Some(proj.cache()),
        ..BuildConfig::default()
    };
    let cap = SpaceCap::new(0);
    let (_, artifacts, _) =
        compile_fitsh_with_calcs(&source, &fitsh, &cfg, &cap).expect("initial build");
    let bytes = artifacts[0].container_bytes.clone();

    // --no-build: the .rs is ignored, the sibling .hcf1 is used verbatim.
    let _ = std::fs::remove_dir_all(proj.cache());
    let cfg_nobuild = BuildConfig {
        no_build: true,
        ..cfg.clone()
    };
    let ((contract, ..), artifacts2, _) =
        compile_fitsh_with_calcs(&source, &fitsh, &cfg_nobuild, &cap).expect("no-build build");
    assert_eq!(artifacts2[0].origin, "prebuilt-container");
    assert_eq!(artifacts2[0].container_bytes, bytes);
    let sto = contract.into_sto();
    let f = sto
        .userfuncs
        .iter()
        .find(|f| f.sign.0 == vm::calc_func_sign("hash"))
        .unwrap();
    assert_eq!(f.code_stuff.data.to_vec(), bytes);
}

#[test]
#[ignore = "drives the real rust toolchain"]
fn explicit_mem_kib_is_honored() {
    let proj = Project::new("mem");
    let source = kernel_source(ProfileName::Rv64im).replace(
        "// calc: input_max = 1024",
        "// calc: mem_kib = 16\n// calc: input_max = 1024",
    );
    let src = proj.write("hash.rs", &source);
    let meta = parse_kernel_meta(&src, &source).unwrap();
    assert_eq!(meta.mem_kib, MemSpec::KiB(16));
    let cap = SpaceCap::new(0);
    let built = KernelBuildRequest {
        name: "hash",
        source_path: &src,
        meta: &meta,
        has_input: true,
        cap: &cap,
        strict: false,
        cache_root: Some(proj.cache()),
    }
    .build()
    .unwrap();
    assert_eq!(built.container.mem_kib, 16);
    let (out, _, _) = run_container(built.container.clone(), INPUT, 1_000_000).unwrap();
    assert_eq!(out, expected_output());
}

#[test]
#[ignore = "drives the real rust toolchain"]
fn binary_run_and_build_subcommands() {
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let proj = Project::new("bin");
    proj.write("hash.rs", &kernel_source(ProfileName::Rv64imc));
    let fitsh = proj.write(
        "demo.fitsh",
        "pragma fitsh 1.0.0\ncontract Demo {\n    function rv64im hash(data: bytes) -> bytes\n    function external fingerprint(x: bytes) -> bytes { return self::hash(x) }\n}\n",
    );

    let out = std::process::Command::new(exe)
        .arg("run")
        .arg(proj.path("hash.rs"))
        .arg("--input")
        .arg(format!("0x{}", hex::encode(INPUT)))
        .env("FITSHC_CACHE", proj.cache())
        .output()
        .expect("run fitshc");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let want = format!("output: 0x{}", hex::encode(expected_output()));
    assert!(stdout.contains(&want), "stdout: {stdout}");

    let out = std::process::Command::new(exe)
        .arg("build")
        .arg(&fitsh)
        .env("FITSHC_CACHE", proj.cache())
        .output()
        .expect("run fitshc build");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Compile success!"), "stdout: {stdout}");
    assert!(stdout.contains("Calc kernel 'hash'"), "stdout: {stdout}");
    assert!(proj.path("demo.deploy.json").is_file());
    assert!(proj.path("hash.hcf1").is_file());

    // Legacy positional form must keep working.
    let out = std::process::Command::new(exe)
        .arg(fitsh)
        .env("FITSHC_CACHE", proj.cache())
        .output()
        .expect("legacy invocation");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "drives the real rust toolchain"]
fn wrong_signature_reports_expected_contract() {
    let proj = Project::new("badsig");
    let src = proj.write(
        "hash.rs",
        "// calc: profile = rv64im\npub fn hash(x: u32) -> u32 { x }\n",
    );
    let meta = parse_kernel_meta(&src, "// calc: profile = rv64im\n").unwrap();
    let cap = SpaceCap::new(0);
    let err = match (KernelBuildRequest {
        name: "hash",
        source_path: &src,
        meta: &meta,
        has_input: true,
        cap: &cap,
        strict: false,
        cache_root: Some(proj.cache()),
    })
    .build()
    {
        Ok(_) => panic!("expected signature error"),
        Err(e) => e,
    };
    assert!(
        err.contains("pub fn hash(input: &[u8], out: &mut [u8]) -> usize"),
        "{err}"
    );
    assert!(
        err.contains("mismatched types") || err.contains("expected"),
        "{err}"
    );
}

/// Extra guard: overrides beat auto-discovery (no toolchain needed beyond a
/// cached container, so this runs alongside the ignored set).
#[test]
#[ignore = "drives the real rust toolchain"]
fn override_wins_over_rs() {
    let proj = Project::new("override");
    proj.write("hash.rs", &kernel_source(ProfileName::Rv64im));
    let fitsh = proj.write(
        "demo.fitsh",
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im hash(data: bytes) -> bytes\n}\n",
    );
    let source = std::fs::read_to_string(&fitsh).unwrap();
    // Build to get a container, then move the .rs away and point --calc at
    // the hcf1: the explicit override must be preferred over nothing else.
    let cfg = BuildConfig {
        cache_root: Some(proj.cache()),
        ..BuildConfig::default()
    };
    let cap = SpaceCap::new(0);
    let (_, artifacts, _) = compile_fitsh_with_calcs(&source, &fitsh, &cfg, &cap).unwrap();
    let bytes = artifacts[0].container_bytes.clone();
    std::fs::remove_file(proj.path("hash.rs")).unwrap();
    let mut overrides = BTreeMap::new();
    overrides.insert("hash".to_string(), proj.path("hash.hcf1"));
    let cfg = BuildConfig {
        overrides,
        cache_root: Some(proj.cache()),
        ..BuildConfig::default()
    };
    let (_, artifacts2, _) = compile_fitsh_with_calcs(&source, &fitsh, &cfg, &cap).unwrap();
    assert_eq!(artifacts2[0].container_bytes, bytes);
}
