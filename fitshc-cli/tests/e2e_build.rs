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
use std::path::PathBuf;

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
        no_cache: false,
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
    let _ = build_and_run("p0s", ProfileName::Rv64im, true);
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
            no_cache: false,
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
        no_cache: false,
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
    proj.write("calc/hash.rs", &kernel_source(ProfileName::Rv64im));
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
    let elf = artifacts[0]
        .build_dir
        .as_ref()
        .map(|d| std::fs::read(d.join("out/kernel.elf")).ok())
        .flatten()
        .expect("cached ELF");

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

    // The bare `.elf` fallback: no .rs, no .hcf1 — the sibling ELF is packed
    // with `elf_defaults` (including an explicit `--mem`).
    std::fs::remove_file(proj.path("hash.hcf1")).unwrap();
    std::fs::write(proj.path("hash.elf"), &elf).unwrap();
    let cfg_elf = BuildConfig {
        no_build: true,
        elf_defaults: fitshc_cli::resolve::ElfDefaults {
            profile: ProfileName::Rv64im,
            input_max: 1024,
            output_max: 64,
            hash: false,
            mem: MemSpec::KiB(12),
        },
        ..cfg.clone()
    };
    let ((contract, ..), artifacts3, _) =
        compile_fitsh_with_calcs(&source, &fitsh, &cfg_elf, &cap).expect(".elf fallback build");
    assert_eq!(artifacts3[0].origin, "prebuilt-elf");
    assert_eq!(artifacts3[0].mem_kib, 12, "--mem must reach the .elf pack");
    assert_eq!(artifacts3[0].input_max, 1024);
    assert_eq!(artifacts3[0].output_max, 64);
    let (out, _, _) = run_container(
        vm::calc::container::CalcContainer::decode(&artifacts3[0].container_bytes)
            .unwrap()
            .0,
        INPUT,
        1_000_000,
    )
    .unwrap();
    assert_eq!(out, expected_output());
    let sto = contract.into_sto();
    let f = sto
        .userfuncs
        .iter()
        .find(|f| f.sign.0 == vm::calc_func_sign("hash"))
        .unwrap();
    assert_eq!(f.code_stuff.data.to_vec(), artifacts3[0].container_bytes);
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
        no_cache: false,
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
    proj.write("hash.rs", &kernel_source(ProfileName::Rv64im));
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
        no_cache: false,
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

/// P0-1 regression: an *unprovable* bounds check links `core::panicking`,
/// whose prebuilt riscv64imac core carries `.eh_frame`. The linker script
/// must discard it, or the default (stable, non-strict) rv64im recipe fails
/// with `unknown allocated section` / `.text` off CODE_BASE.
#[test]
#[ignore = "drives the real rust toolchain"]
fn panic_path_kernel_builds_on_default_profile() {
    for profile in [ProfileName::Rv64im] {
        let proj = Project::new(&format!("panic-{}", profile.as_str()));
        let source = format!(
            "// calc: profile = {}\n\
             pub fn hash(input: &[u8], out: &mut [u8]) -> usize {{\n\
             \x20   // Unprovable index: the bounds check (and its panic path,
             \x20   // and .eh_frame behind it) cannot be optimized away.
             \x20   let idx = (input.first().copied().unwrap_or(0) as usize) % out.len();
             \x20   out[idx] = 1;
             \x20   out[0] = input.len() as u8;
             \x20   1
             }}\n",
            profile.as_str()
        );
        let src = proj.write("hash.rs", &source);
        let meta = parse_kernel_meta(&src, &source).unwrap();
        let cap = SpaceCap::new(0);
        let built = KernelBuildRequest {
            name: "hash",
            source_path: &src,
            meta: &meta,
            has_input: true,
            cap: &cap,
            strict: false,
            cache_root: Some(proj.cache()),
            no_cache: false,
        }
        .build()
        .unwrap_or_else(|e| panic!("profile {profile:?}: {e}"));
        let (out, _, _) = run_container(built.container.clone(), &[7, 9], 1_000_000).unwrap();
        assert_eq!(out, vec![2], "profile {profile:?}");
    }
}

/// P0-2 regression: editing a `mod helpers;` dependency must invalidate the
/// cache even though the kernel file itself did not change.
#[test]
#[ignore = "drives the real rust toolchain"]
fn helpers_change_invalidates_cache() {
    let proj = Project::new("helpers");
    let cap = SpaceCap::new(0);
    proj.write("helpers.rs", "pub const VALUE: u8 = 1;\n");
    let source = "// calc: profile = rv64im\nmod helpers;\npub fn hash(_input: &[u8], out: &mut [u8]) -> usize {\n    out[0] = helpers::VALUE;\n    1\n}\n";
    let src = proj.write("hash.rs", source);
    let meta = parse_kernel_meta(&src, source).unwrap();
    fn request<'a>(
        src: &'a std::path::Path,
        meta: &'a fitshc_cli::meta::KernelMeta,
        cap: &'a SpaceCap,
        cache: &'a std::path::Path,
    ) -> KernelBuildRequest<'a> {
        KernelBuildRequest {
            name: "hash",
            source_path: src,
            meta,
            has_input: true,
            cap,
            strict: false,
            cache_root: Some(cache.to_path_buf()),
            no_cache: false,
        }
    }
    let first = request(&src, &meta, &cap, &proj.cache()).build().unwrap();
    assert!(!first.cache_hit);
    let (out, _, _) = run_container(first.container, &[0xaa], 1_000_000).unwrap();
    assert_eq!(out, vec![1]);

    // Same kernel file, changed helper → must rebuild and reflect the change.
    proj.write("helpers.rs", "pub const VALUE: u8 = 2;\n");
    let second = request(&src, &meta, &cap, &proj.cache()).build().unwrap();
    assert!(
        !second.cache_hit,
        "changed helper must invalidate the cache"
    );
    let (out, _, _) = run_container(second.container.clone(), &[0xaa], 1_000_000).unwrap();
    assert_eq!(out, vec![2]);

    // Unchanged helper → hit.
    let third = request(&src, &meta, &cap, &proj.cache()).build().unwrap();
    assert!(third.cache_hit, "unchanged helper must hit the cache");
    assert_eq!(third.container_bytes, second.container_bytes);
}

/// P1-3: the build must be hermetic against ambient flag/toolchain env, and
/// survive paths with spaces (cache root and kernel dir).
#[test]
#[ignore = "drives the real rust toolchain"]
fn hermetic_env_and_spaces_in_paths() {
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let base = std::env::temp_dir().join("fitshc-e2e spaces");
    let _ = std::fs::remove_dir_all(&base);
    let proj_dir = base.join("kernel dir");
    std::fs::create_dir_all(&proj_dir).unwrap();
    let kernel = proj_dir.join("hash.rs");
    std::fs::write(&kernel, kernel_source(ProfileName::Rv64im)).unwrap();

    // (a) ambient RUSTFLAGS / CARGO_ENCODED_RUSTFLAGS must be scrubbed: the
    // bogus +A feature would otherwise break the layout or fail verify.
    let out = std::process::Command::new(exe)
        .arg("run")
        .arg(&kernel)
        .arg("--input")
        .arg(format!("0x{}", hex::encode(INPUT)))
        .env("FITSHC_CACHE", base.join("cache with space"))
        .env("RUSTFLAGS", "-C target-feature=+a")
        .env("CARGO_ENCODED_RUSTFLAGS", "-C\x1ftarget-feature=+a")
        .output()
        .expect("run fitshc");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let want = format!("output: 0x{}", hex::encode(expected_output()));
    assert!(stdout.contains(&want), "stdout: {stdout}");
}

/// P1-5: `fitshc run` on a zero-arg kernel (signature inference), plus the
/// `--no-input` pin and the rejected `--no-build` combination.
#[test]
#[ignore = "drives the real rust toolchain"]
fn run_handles_zero_arg_kernel() {
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let proj = Project::new("zeroarg");
    proj.write(
        "tick.rs",
        "/// Zero-arg entry; `fn tick()` in a doc comment must be ignored.\n\
         // fn tick(input: &[u8])\n\
         pub fn tick(out: &mut [u8]) -> usize {\n    out[0] = 0x2a;\n    1\n}\n",
    );
    let out = std::process::Command::new(exe)
        .arg("run")
        .arg(proj.path("tick.rs"))
        .env("FITSHC_CACHE", proj.cache())
        .output()
        .expect("run fitshc");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("output: 0x2a"), "stdout: {stdout}");

    // The two-arg shape is still inferred when `fn narrow(...)` also appears
    // in comments / string literals before the definition.
    proj.write(
        "narrow.rs",
        "// calc: profile = rv64im\n\
         const NOTE: &str = \"fn narrow()\";\n\
         /* fn narrow(x: u32) */\n\
         pub fn narrow(_input: &[u8], out: &mut [u8]) -> usize {\n\
         \x20   out[0] = 0x5a;\n\
         \x20   1\n\
         }\n",
    );
    let out = std::process::Command::new(exe)
        .arg("run")
        .arg(proj.path("narrow.rs"))
        .arg("--input")
        .arg("0x01")
        .env("FITSHC_CACHE", proj.cache())
        .output()
        .expect("run fitshc");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("output: 0x5a"),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // --no-build with a .rs kernel is rejected instead of misleading.
    let out = std::process::Command::new(exe)
        .arg("run")
        .arg(proj.path("tick.rs"))
        .arg("--no-build")
        .env("FITSHC_CACHE", proj.cache())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("--no-build"));
}

/// P1-6: `--no-build` in a read-only source dir must write the contract
/// outputs to `--out-dir` (created on demand) and skip kernel side artifacts.
#[test]
#[ignore = "drives the real rust toolchain"]
#[cfg(unix)]
fn no_build_works_in_read_only_dir() {
    use std::os::unix::fs::PermissionsExt;
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let base = std::env::temp_dir().join("fitshc-e2e-readonly");
    let out_dir = std::env::temp_dir().join("fitshc-e2e-readonly-out");
    let _ = std::fs::remove_dir_all(&base);
    let _ = std::fs::remove_dir_all(&out_dir);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(
        base.join("demo.fitsh"),
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im hash(data: bytes) -> bytes\n}\n",
    )
    .unwrap();
    std::fs::write(base.join("hash.rs"), kernel_source(ProfileName::Rv64im)).unwrap();
    // Build once (writable) to produce hash.hcf1 next to the fitsh.
    let out = std::process::Command::new(exe)
        .arg("build")
        .arg(base.join("demo.fitsh"))
        .arg("--no-artifacts")
        .env("FITSHC_CACHE", base.join(".cache"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cache_out = std::fs::read_dir(base.join(".cache"))
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with("k-"))
        .map(|e| e.path().join("out/kernel.hcf1"))
        .expect("cache entry");
    std::fs::copy(cache_out, base.join("hash.hcf1")).unwrap();
    // The initial build wrote contract outputs next to the fitsh; clear them
    // so the read-only run proves they are not re-created there.
    let _ = std::fs::remove_file(base.join("demo.deploy.json"));
    let _ = std::fs::remove_file(base.join("demo.contractmap.json"));

    // Read-only dir + --no-build + --out-dir (absent: must be created).
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o555)).unwrap();
    let out = std::process::Command::new(exe)
        .arg("build")
        .arg(base.join("demo.fitsh"))
        .arg("--no-build")
        .arg("--out-dir")
        .arg(&out_dir)
        .env("FITSHC_CACHE", base.join(".cache"))
        .output()
        .unwrap();
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "exit={:?} stdout={stdout} stderr={stderr}",
        out.status.code()
    );
    assert!(stdout.contains("Calc kernel 'hash'"), "stdout: {stdout}");
    assert!(
        out_dir.join("demo.deploy.json").is_file(),
        "deploy json must land in --out-dir: {stdout}"
    );
    assert!(
        out_dir.join("demo.contractmap.json").is_file(),
        "contract map must land in --out-dir: {stdout}"
    );
    // No kernel side artifacts: --no-build keeps emit_artifacts off.
    assert!(!out_dir.join("hash.hcf1").exists(), "stdout: {stdout}");
    assert!(
        !base.join("demo.deploy.json").exists(),
        "read-only source dir must stay untouched"
    );
}

/// P3: `pack` refuses to overwrite its input; invalid flags error early.
#[test]
fn pack_guards_and_flag_validation() {
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let out = std::process::Command::new(exe)
        .arg("pack")
        .arg("/tmp/definitely-missing.elf")
        .output()
        .unwrap();
    assert!(!out.status.success());

    // -o pointing at the input is rejected before writing anything.
    let dir = std::env::temp_dir().join("fitshc-pack-guard");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let elf = dir.join("kernel.elf");
    std::fs::write(&elf, b"\x7fELFstub").unwrap();
    let out = std::process::Command::new(exe)
        .arg("pack")
        .arg(&elf)
        .arg("-o")
        .arg(&elf)
        .output()
        .unwrap();
    assert!(!out.status.success(), "overwrite must be rejected");
    assert!(String::from_utf8_lossy(&out.stdout).contains("overwrite"));

    // Unknown flags and invalid profiles/periods fail at parse.
    for args in [
        vec!["build", "/tmp/none.fitsh", "--turbo"],
        vec!["pack", "/tmp/none.elf", "--strict"],
        vec!["pack", "/tmp/none.elf", "--no-build"],
        vec!["run", "/tmp/none.rs", "--calc", "x=y"],
        vec!["build", "/tmp/none.fitsh", "--profile", "avx"],
        vec!["build", "/tmp/none.fitsh", "--profile", "rv64imc"],
        vec!["pack", "/tmp/none.elf", "--hash"],
        vec!["build", "/tmp/none.fitsh", "--periods", "3"],
    ] {
        let out = std::process::Command::new(exe)
            .args(&args)
            .output()
            .unwrap();
        assert!(!out.status.success(), "accepted invalid args: {args:?}");
        assert!(
            String::from_utf8_lossy(&out.stdout).starts_with("Error:"),
            "args {args:?}"
        );
    }
}

/// P1: a relative or symlinked `FITSHC_CACHE` must build (the `-T` path is
/// made absolute) and must hit on the second run (dep-info paths and the
/// build-dir exclusion agree after canonicalization).
#[test]
#[ignore = "drives the real rust toolchain"]
fn relative_and_symlinked_cache_roots_hit() {
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let proj = Project::new("cachepaths");
    let kernel = proj.write("hash.rs", &kernel_source(ProfileName::Rv64im));
    let run = |cache: &std::path::Path| {
        std::process::Command::new(exe)
            .arg("run")
            .arg(&kernel)
            .arg("--input")
            .arg(format!("0x{}", hex::encode(INPUT)))
            .current_dir(&proj.dir)
            .env("FITSHC_CACHE", cache)
            .output()
            .unwrap()
    };
    let report = |out: &std::process::Output| {
        format!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };

    // Relative cache root (the original linker-script failure).
    let first = run(std::path::Path::new(".cache"));
    assert!(first.status.success(), "{}", report(&first));
    let second = run(std::path::Path::new(".cache"));
    assert!(second.status.success(), "{}", report(&second));
    assert!(
        String::from_utf8_lossy(&second.stdout).contains("[cache]"),
        "relative cache root must hit: {}",
        report(&second)
    );

    // Symlinked cache root (the original dep-info exclusion failure).
    #[cfg(unix)]
    {
        let real = proj.dir.join("real-cache");
        let link = proj.dir.join("link-cache");
        std::fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let first = run(&link);
        assert!(first.status.success(), "{}", report(&first));
        let second = run(&link);
        assert!(second.status.success(), "{}", report(&second));
        assert!(
            String::from_utf8_lossy(&second.stdout).contains("[cache]"),
            "symlinked cache root must hit: {}",
            report(&second)
        );
    }
}

/// P2: `--no-cache` must not leave its throwaway build dir (or a cache root)
/// behind.
#[test]
#[ignore = "drives the real rust toolchain"]
#[cfg(unix)]
fn no_cache_removes_temp_build_dir() {
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let proj = Project::new("nocache");
    proj.write("hash.rs", &kernel_source(ProfileName::Rv64im));
    let tmp = proj.dir.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let cache = proj.dir.join("never-cache");
    let out = std::process::Command::new(exe)
        .arg("run")
        .arg(proj.path("hash.rs"))
        .arg("--input")
        .arg(format!("0x{}", hex::encode(INPUT)))
        .arg("--no-cache")
        .env("TMPDIR", &tmp)
        .env("FITSHC_CACHE", &cache)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let leftovers: Vec<String> = std::fs::read_dir(&tmp)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("fitshc-build-k-"))
        .collect();
    assert!(leftovers.is_empty(), "leaked build dirs: {leftovers:?}");
    assert!(
        !cache.exists(),
        "--no-cache must not create the cache root {}",
        cache.display()
    );
}

/// `fitshc image` writes the bytes a `function rv64im = 0x` body stores.
#[test]
#[ignore = "drives the real rust toolchain"]
fn image_writes_admitted_function_body() {
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let proj = Project::new("image-ok");
    proj.write("hash.rs", &kernel_source(ProfileName::Rv64im));
    let out_path = proj.path("hash.hcf1");
    let out = std::process::Command::new(exe)
        .arg("image")
        .arg(proj.path("hash.rs"))
        .arg("-o")
        .arg(&out_path)
        .arg("--hex")
        .arg("--no-cache")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let hex_line = String::from_utf8_lossy(&out.stdout);
    let hex_line = hex_line.trim();
    assert!(
        hex_line.starts_with("0x4843463101"),
        "container hex must be HCF1 version 1, got {hex_line:.40}"
    );
    let file = std::fs::read(&out_path).unwrap();
    assert_eq!(file.first().copied(), Some(b'H'));
    assert_eq!(file.get(4).copied(), Some(1), "version byte");
    let func = vm::contract::Func::new("hash")
        .unwrap()
        .rv64im_container_bytes(file.clone())
        .expect("image bytes must pass function rv64im admission");
    let sto = vm::contract::Contract::new().func(func).into_sto();
    assert_eq!(
        sto.userfuncs[0].code_stuff.conf.uint() & CodeType::TYPE_MASK,
        CodeType::RV64IM as u8
    );
    assert_eq!(sto.userfuncs[0].code_stuff.data.to_vec(), file);
}
