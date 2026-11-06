//! Build recipes: how each ISA profile turns a user's `kernel.rs` into a
//! linked bare-metal ELF that `pack_elf` can containerize.
//!
//! Two families, validated end to end against the container verifier:
//!
//! * **Custom target JSON + build-std** (profiles `rv64im`/`rv64imb`, and
//!   the `--strict` variants of the C profiles): rustc has no tier-2
//!   `riscv64im` target, and the prebuilt `riscv64imac` core/compiler_builtins
//!   contain C (and A) encodings. `-Zbuild-std=core,compiler_builtins`
//!   rebuilds them for the exact feature set. `RUSTC_BOOTSTRAP=1` lets a
//!   stable toolchain accept the `-Z` flags (works on current stable; a
//!   nightly toolchain works too).
//! * **Builtin `riscv64imac-unknown-none-elf`** (profiles `rv64imc`/
//!   `rv64imcb`): plain stable cross build with `-C target-feature=-a`.
//!   Caveats inherited from the prebuilt core: pulling in
//!   `core::sync::atomic` links A-extension helpers the verifier rejects,
//!   and LLVM may emit 2-byte `unimp` (all-zero) fillers on unreachable
//!   paths, which RVC-profile decode treats as `Trap`. The generated wrapper
//!   avoids those sites (real `loop` backedges, no `noreturn` asm options).
//!
//! The linker script emits ELF `PT_LOAD` at `CODE_BASE` with `calc_entry`
//! first. `pack_elf` stores those program headers as-is (`.bss` is
//! `memsz - filesz`). Link with `-n` so segments are not page-aligned.

use crate::meta::ProfileName;

/// Bump when any template or flag below changes; part of the cache key.
pub const RECIPE_VERSION: u32 = 3;

/// Everything one profile needs from the toolchain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// Target triple passed to `cargo build --target`.
    pub target: String,
    /// File name of the target spec written into the build dir (custom JSON
    /// recipes only; `None` for builtin targets).
    pub target_json: Option<(&'static str, String)>,
    /// `-C target-feature=...` additions (builtin recipes only).
    pub target_features: Option<&'static str>,
    /// Needs `-Zbuild-std` (+ `RUSTC_BOOTSTRAP=1`).
    pub build_std: bool,
    /// Stable identifier for manifests / cache keys.
    pub id: &'static str,
}

fn custom_target_json(features: &str) -> String {
    format!(
        r#"{{
  "arch": "riscv64",
  "cpu": "generic-rv64",
  "data-layout": "e-m:e-p:64:64-i64:64-i128:128-n32:64-S128",
  "eh-frame-header": false,
  "emit-debug-gdb-scripts": false,
  "features": "{features}",
  "linker": "rust-lld",
  "linker-flavor": "gnu-lld",
  "llvm-abiname": "lp64",
  "llvm-target": "riscv64",
  "max-atomic-width": 0,
  "atomic-cas": false,
  "os": "none",
  "panic-strategy": "abort",
  "relocation-model": "static",
  "target-pointer-width": 64,
  "vendor": "unknown"
}}
"#
    )
}

impl ProfileName {
    /// Recipe for the profile. `strict` rebuilds core/compiler_builtins even
    /// for the C profiles (drops the prebuilt +A core caveat).
    pub fn recipe(&self, strict: bool) -> Recipe {
        match self {
            ProfileName::Rv64im => Recipe {
                target: "riscv64im_zicclsm.json".to_string(),
                target_json: Some((
                    "riscv64im_zicclsm.json",
                    custom_target_json("+m,-a,-c,-d,-f"),
                )),
                target_features: None,
                build_std: true,
                id: "custom-riscv64im-zicclsm",
            },
            ProfileName::Rv64imb => Recipe {
                target: "riscv64imb_zicclsm.json".to_string(),
                target_json: Some((
                    "riscv64imb_zicclsm.json",
                    custom_target_json("+m,+zba,+zbb,+zbs,+zbc,-a,-c,-d,-f"),
                )),
                target_features: None,
                build_std: true,
                id: "custom-riscv64imb-zicclsm",
            },
            ProfileName::Rv64imc if !strict => Recipe {
                target: "riscv64imac-unknown-none-elf".to_string(),
                target_json: None,
                target_features: Some("-a"),
                build_std: false,
                id: "builtin-riscv64imac",
            },
            ProfileName::Rv64imc => Recipe {
                target: "riscv64imc_zicclsm.json".to_string(),
                target_json: Some((
                    "riscv64imc_zicclsm.json",
                    custom_target_json("+m,+c,-a,-d,-f"),
                )),
                target_features: None,
                build_std: true,
                id: "custom-riscv64imc-zicclsm",
            },
            ProfileName::Rv64imcb if !strict => Recipe {
                target: "riscv64imac-unknown-none-elf".to_string(),
                target_json: None,
                target_features: Some("-a,+zba,+zbb,+zbs,+zbc"),
                build_std: false,
                id: "builtin-riscv64imac-b",
            },
            ProfileName::Rv64imcb => Recipe {
                target: "riscv64imcb_zicclsm.json".to_string(),
                target_json: Some((
                    "riscv64imcb_zicclsm.json",
                    custom_target_json("+m,+c,+zba,+zbb,+zbs,+zbc,-a,-d,-f"),
                )),
                target_features: None,
                build_std: true,
                id: "custom-riscv64imcb-zicclsm",
            },
        }
    }
}

/// Linker script emitting PT_LOAD at `CODE_BASE`. Combined with `-n`
/// (`--nmagic`) so rust-lld does not page-align segments.
///
/// The `/DISCARD/` of `.eh_frame`/`.eh_frame_hdr` is load-bearing: any panic
/// or bounds-check path links `core::panicking`, and the *prebuilt*
/// `riscv64imac` core (the stable recipes for profiles `rv64imc`/`rv64imcb`)
/// carries unwind tables. Without the discard the linker drops `.eh_frame`
/// in as an orphan *before* `.text`.
pub const CONTAINER_LD: &str = r#"/* PT_LOAD projection. IMAGE is the L1 guest window (64 KiB usable
   minus a 12 KiB host reserve for input+output+stack+guard).
   Link with -n so lld does not page-align PT_LOAD. */
ENTRY(calc_entry)
MEMORY
{
  IMAGE (rwx) : ORIGIN = 0x1000, LENGTH = 0xD000
}
PHDRS
{
  text PT_LOAD FLAGS(5);
  rodata PT_LOAD FLAGS(4);
  data PT_LOAD FLAGS(6);
}
SECTIONS
{
  . = ORIGIN(IMAGE);
  .text : { *(.text.calc_entry) *(.text .text.*) } :text
  .rodata : {
    *(.rodata .rodata.*) *(.srodata .srodata.*) *(.rodata.cst*)
  } :rodata
  .data : { *(.data .data.*) *(.sdata .sdata.*) } :data
  .bss : { *(.sbss .sbss.*) *(.bss .bss.*) *(COMMON) } :data
  /DISCARD/ : { *(.eh_frame) *(.eh_frame_hdr) }
  ASSERT(. <= ORIGIN(IMAGE) + LENGTH(IMAGE), "guest image overlaps L1 host window");
}
"#;

/// Cargo manifest of the generated wrapper crate. No dependencies: only
/// `core` (rebuilt for custom targets). The wrapper crate is throwaway; the
/// user's file is pulled in via `#[path]`, never copied.
pub const WRAPPER_CARGO_TOML: &str = r#"[package]
name = "calc_kernel"
version = "0.0.0"
edition = "2021"

[profile.release]
panic = "abort"
opt-level = "z"
lto = true
codegen-units = 1
debug = false

[workspace]
"#;

/// Generate the wrapper `main.rs` that owns the whole kernel ABI.
///
/// Contract for the user's `<name>.rs` (a plain module, not a crate root):
///
/// ```ignore
/// pub fn kernel(input: &[u8], out: &mut [u8]) -> usize   // with input arg
/// pub fn kernel(out: &mut [u8]) -> usize                 // zero-arg form
/// ```
///
/// The wrapper:
/// * builds `input` from `(a0, a1)` and `out` from `(a2, a3 = output_max)`;
/// * returns the written length (`ra` is the HALT trampoline);
/// * traps when the returned length exceeds `out.len()`;
/// * keeps a real `loop {}` around `ebreak` (a `noreturn` asm would let
///   LLVM drop the backedge and emit a 2-byte all-zero `unimp` filler);
/// * forces the 32-bit `ebreak` encoding with `.option norvc` so the trap
///   path is profile-independent;
/// * provides the `#[panic_handler]`, so panics become deterministic traps.
///
/// A wrong or missing user signature is a normal rustc compile error against
/// the original file — no Rust source parsing on our side.
pub fn wrapper_source(user_module_path: &str, kernel_name: &str, has_input: bool) -> String {
    let call = if has_input {
        format!("user::{kernel_name}(input, out)")
    } else {
        format!("user::{kernel_name}(out)")
    };
    format!(
        r#"#![no_std]
#![no_main]

#[path = {user_module_path:?}]
mod user;

#[no_mangle]
pub extern "C" fn calc_entry(
    a0: *const u8,
    a1: u64,
    a2: *mut u8,
    a3: u64,
) -> u64 {{
    let input = unsafe {{ core::slice::from_raw_parts(a0, a1 as usize) }};
    let out = unsafe {{ core::slice::from_raw_parts_mut(a2, a3 as usize) }};
    let n = {call};
    if n > out.len() {{
        calc_trap()
    }}
    n as u64
}}

/// Deterministic trap: 32-bit ebreak under every profile (`.option norvc`
/// keeps it 4 bytes even for RVC targets).
#[inline(never)]
fn calc_trap() -> ! {{
    loop {{
        unsafe {{
            core::arch::asm!(
                ".option push\n.option norvc\nebreak\n.option pop",
            );
        }}
    }}
}}

#[panic_handler]
fn calc_panic(_: &core::panic::PanicInfo) -> ! {{
    calc_trap()
}}
"#
    )
}

/// `RUSTFLAGS` for the cargo invocation (without the recipe's
/// `-C target-feature`, appended by the caller from [`Recipe`]).
///
/// The `-T` script path and the user-directory remap below never affect the
/// produced image bytes (the script path is not embedded; the remap rewrites
/// panic-location paths to the fixed `/calc/src` prefix on every machine), so
/// the caller may compute the cache key from a placeholder `-T` path and then
/// run with the absolute one.
pub fn base_rustflags(
    container_ld_path: &std::path::Path,
    user_file: &std::path::Path,
) -> Vec<String> {
    let mut flags = vec![
        // Single-token `-Ckey=value` forms: each element of
        // CARGO_ENCODED_RUSTFLAGS is one argv entry, so a `-C key=value`
        // pair with an inner space cannot be expressed there.
        "-Crelocation-model=static".to_string(),
        format!("-Clink-arg=-T{}", container_ld_path.display()),
        // Do not page-align PT_LOAD; the guest map is not a Linux process.
        "-Clink-arg=-n".to_string(),
    ];
    // `--remap-path-prefix` is a top-level rustc flag (`-C remap-path-prefix`
    // was removed from the codegen options); skip the empty parent of a
    // bare relative file name.
    if let Some(dir) = user_file.parent() {
        if !dir.as_os_str().is_empty() {
            flags.push(format!("--remap-path-prefix={}=/calc/src", dir.display()));
        }
    }
    flags
}

/// Extra `cargo build` arguments of a recipe (`-Zbuild-std` etc.).
pub fn cargo_extra_args(recipe: &Recipe) -> Vec<String> {
    if recipe.build_std {
        vec![
            "-Zbuild-std=core,compiler_builtins".to_string(),
            "-Zbuild-std-features=compiler-builtins-mem".to_string(),
        ]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod recipe_tests {
    use super::*;

    #[test]
    fn wrapper_calls_user_kernel_with_and_without_input() {
        let with = wrapper_source("/abs/kernel.rs", "kernel", true);
        assert!(with.contains("#[path = \"/abs/kernel.rs\"]"));
        assert!(with.contains("user::kernel(input, out)"));
        assert!(with.contains("n as u64"));
        assert!(!with.contains("fn calc_halt"));
        assert!(with.contains(".option norvc"));
        assert!(!with.contains("options(noreturn)"));
        let without = wrapper_source("/abs/kernel.rs", "kernel", false);
        assert!(without.contains("user::kernel(out)"));
    }

    #[test]
    fn recipes_cover_all_profiles() {
        for p in ProfileName::ALL {
            for strict in [false, true] {
                let r = p.recipe(strict);
                assert!(!r.target.is_empty());
                assert!(r.id.contains(if r.build_std { "custom" } else { "builtin" }));
                if r.build_std {
                    assert!(r.target_json.is_some());
                    assert!(!cargo_extra_args(&r).is_empty());
                } else {
                    assert!(r.target_features.is_some());
                    assert!(cargo_extra_args(&r).is_empty());
                }
            }
        }
    }

    #[test]
    fn target_jsons_disable_a_c_d_f() {
        let (_, json) = ProfileName::Rv64im.recipe(false).target_json.unwrap();
        assert!(json.contains("\"+m,-a,-c,-d,-f\""));
        let (_, json) = ProfileName::Rv64imb.recipe(false).target_json.unwrap();
        assert!(json.contains("+zba"));
        assert!(json.contains("-a,-c,-d,-f"));
    }
}
