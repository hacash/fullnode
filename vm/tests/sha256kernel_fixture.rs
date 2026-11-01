//! The sha256 kernel fixture (`tests/fixtures/sha256kernel`, RV64IM)
//! no_std, built by `build.sh` with the riscv64 toolchain; the image bins
//! are committed so tests do not need the toolchain).
//!
//! Two modes, dispatched on the first input byte:
//!   * raw  — anything not starting with `{`: plain sha256 of the input;
//!   * dict — a `{k=v,...}` flat-kv document (keys a-z, values
//!     alphanumeric, the flags=0 grammar of the HVM `ascii_parse_flat_kv`
//!     native): walk from key `start`, each visited value names the next
//!     key, `end` terminates; sha256 over the concatenated values. Any
//!     grammar or walk error yields an empty output.
//!
//! The three-channel (fitsh HVM / calc interp / calc AOT) neutral
//! comparison on this same workload lives in
//! `testkit/tests/dict_digest_three_paths.rs`.

use vm::calc::container::CalcContainer;
use vm::calc::verify::verify_container;
use vm::calc::{
    pack_container_with_bss, CalcConfig, CalcFnObj, CalcRunLimits, HashKind, CALC_CONFIG_V1,
};

fn kernel_cfg(aot: bool) -> CalcConfig {
    CALC_CONFIG_V1
        .with_hash_builtins(true)
        .with_rodata_readonly(true)
        .with_stack_guard(true)
        .with_aot_mount(aot)
}

fn fixture(name: &str) -> Vec<u8> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sha256kernel");
    std::fs::read(format!("{dir}/{name}")).expect("fixture present (run build.sh)")
}

fn kernel() -> (CalcFnObj, CalcContainer) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sha256kernel");
    let entry: u32 = u32::from_str_radix(
        std::fs::read_to_string(format!("{dir}/entry.txt"))
            .expect("fixture entry.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .expect("entry hex");
    let bss_len = u16::from_str_radix(
        std::fs::read_to_string(format!("{dir}/bss_len.txt"))
            .expect("fixture bss_len.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .expect("bss_len hex") as u64;
    let container = pack_container_with_bss(
        vm::calc::CALC_PROFILE_RV64IM,
        8, // KiB
        entry,
        256,
        32,
        fixture("image_code.bin"),
        fixture("image_rodata.bin"),
        fixture("image_init.bin"),
        bss_len,
        &CALC_CONFIG_V1,
    )
    .expect("kernel container packs");
    let obj = CalcFnObj::build_with_config(container.clone(), kernel_cfg(false))
        .expect("kernel container verifies");
    (obj, container)
}

fn limits() -> CalcRunLimits {
    CalcRunLimits {
        // vm resource default: the 1M-cycle consensus cap per call
        cycles: 1_000_000,
        resource: i64::MAX,
        value_size: usize::MAX,
        deadline: None,
    }
}

fn ref_sha256(msg: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    vm::calc::run_hash(HashKind::Sha2, msg, &mut out);
    out
}

#[test]
fn kernel_container_verifies() {
    let (_, container) = kernel();
    verify_container(&container).expect("committed image passes the verifier");
}

#[test]
fn kernel_raw_sha256_mode() {
    let (f, _) = kernel();
    for msg in [&b""[..], b"abc", b"hacash calc rv64imc kernel test"] {
        let e = f.run(msg, limits());
        let out = e.result.expect("raw mode ok");
        assert_eq!(&out[..], &ref_sha256(msg)[..], "raw sha256 of {msg:?}");
    }
}

/// Valid walk chains and the value concatenation they must digest.
const WALKS: &[(&[u8], &[u8])] = &[
    (
        b"{start=beta,beta=gamma,gamma=end,other=42,zz=9}",
        b"betagammaend",
    ),
    (b"{start=end}", b"end"),
    (b"{zz=1,start=aa,aa=bb,bb=end}", b"aabbend"),
    (
        b"{start=beta,beta=gamma,gamma=delta,delta=end,other=42,zz=9}",
        b"betagammadeltaend",
    ),
];

#[test]
fn kernel_dict_walk_mode() {
    let (f, _) = kernel();
    for (doc, acc) in WALKS {
        let e = f.run(doc, limits());
        let out = e.result.expect("dict mode ok");
        assert_eq!(&out[..], &ref_sha256(acc)[..], "walk digest of {doc:?}");
    }
}

#[test]
fn kernel_dict_walk_error_paths() {
    let (f, _) = kernel();
    // Grammar / walk errors all surface as an empty output (out_len 0),
    // never as a VM-level abort.
    for doc in [
        &b"{start=beta}"[..],                   // walk breaks: no beta key
        b"{start=beta,beta=end,start=1}",       // duplicate key
        b"{Start=beta,beta=end}",               // key class violation (uppercase)
        b"{start=be ta,beta=end}",              // value class violation (space)
        b"{start=beta,beta=end",                // missing close
        b"{start=beta,beta=end} trailing",      // trailing bytes
        b"{}",                                  // empty doc
        b"{start=BETA,BETA=end}",               // values must be alnum (ok) but
                                                 // chain: BETA key missing
    ] {
        let e = f.run(doc, limits());
        let out = e.result.unwrap_or_default();
        assert!(out.is_empty(), "doc {doc:?} must yield empty output, got {out:?}");
    }
}

#[cfg(feature = "calc-aot")]
#[test]
fn kernel_aot_parity() {
    let (plain, container) = kernel();
    let mounted = CalcFnObj::build_with_config(container, kernel_cfg(true)).expect("AOT image compiles");
    let mut docs: Vec<&[u8]> = vec![b"", b"abc", b"hacash"];
    for (doc, _) in WALKS {
        docs.push(doc);
    }
    docs.extend_from_slice(&[
        b"{start=beta}",
        b"{Start=x,}",
        b"{}",
    ]);
    for doc in docs {
        let e_interp = plain.run(doc, limits());
        let e_aot = mounted.run(doc, limits());
        assert_eq!(
            e_interp.result, e_aot.result,
            "AOT output parity on {doc:?}"
        );
        assert_eq!(
            e_interp.cycles_used, e_aot.cycles_used,
            "AOT cycle parity on {doc:?}"
        );
    }
}
