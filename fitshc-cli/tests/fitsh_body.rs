//! End-to-end tests for native fitsh kernel bodies
//! (`function rv64im f(...) { ... }`) — no rust toolchain involved, so
//! these run in every default `cargo test` invocation.

use std::path::Path;

use fitshc_cli::{BuildConfig, compile_fitsh_with_calcs, run_container, single_body_kernel};
use vm::calc::SpaceCap;

/// score_sum-shaped body: byte-weighted mod-prime accumulation (the
/// HacdsDeFi wire form). `input` is folded as Σ bᵢ·31ⁱ mod P over u64.
fn fold_reference(input: &[u8]) -> u64 {
    let mut s: u64 = 0;
    for &b in input {
        s = (s.wrapping_mul(31).wrapping_add(b as u64)) % 1_000_000_007;
    }
    s
}

const BODY_CONTRACT: &str = r#"pragma fitsh 1.0.0
contract Fold {
    function rv64im fold(data: bytes) -> bytes {
        // calc: input_max = 4800
        // calc: output_max = 8
        var s = 0u64
        var i = 0u64
        var n = size(data)
        while i < n {
            s = (s * 31 + byte(data, i)) % 1000000007
            i += 1
        }
        return s as bytes
    }
    function external run(x: bytes) -> bytes { return self::fold(x) }
}
"#;

#[test]
fn body_kernel_builds_compiles_and_deploys() {
    let cap = SpaceCap::new(0);
    let cfg = BuildConfig::default();
    let dir = std::env::temp_dir().join("fitshc-body-e2e");
    let fitsh_path = dir.join("Fold.fitsh");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&fitsh_path, BODY_CONTRACT).unwrap();

    let (out, artifacts, warnings) =
        compile_fitsh_with_calcs(BODY_CONTRACT, &fitsh_path, &cfg, &cap).unwrap();
    // the only expected note: the default 64 KiB mem sits at the L1 cap
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("mem_kib 64 sits at the chain admission cap"));
    assert_eq!(artifacts.len(), 1);
    let a = &artifacts[0];
    assert_eq!(a.name, "fold");
    assert_eq!(a.origin, "fitsh-source");
    // the `// calc:` header in the body steers the container
    assert_eq!(a.input_max, 4800);
    assert_eq!(a.output_max, 8);
    assert!(a.container_bytes.starts_with(b"HCF1"));
    assert_eq!(a.profile_byte, 0x02, "default lowering is compressed");

    // artifacts on disk: .hcf1 + manifest
    let hcf1 = dir.join("fold.hcf1");
    assert_eq!(a.hcf1_path.as_ref().unwrap(), &hcf1);
    assert_eq!(std::fs::read(&hcf1).unwrap(), a.container_bytes);
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("fold.calc.manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["origin"], "fitsh-source");

    // the contract compiled: CALLSELFPURE path, kernel is a userfunc
    assert_eq!(out.3, "Fold");

    // execute through the same container the chain would admit
    let input: Vec<u8> = (0u8..=63).collect();
    let c = vm::fitshc::kernel::decode_container(&a.container_bytes).unwrap();
    let (o, _cycles, _res) = run_container(c, &input, 5_000_000).unwrap();
    assert_eq!(o, fold_reference(&input).to_be_bytes().to_vec());
}

#[test]
fn body_kernel_run_and_image_paths() {
    let cap = SpaceCap::new(0);
    let (name, bytes) = single_body_kernel(Path::new("Fold.fitsh"), BODY_CONTRACT, &cap).unwrap();
    assert_eq!(name, "fold");
    let c = vm::fitshc::kernel::decode_container(&bytes).unwrap();
    let input: Vec<u8> = b"fitsh-body-kernel".to_vec();
    let (o, _cycles, _res) = run_container(c, &input, 5_000_000).unwrap();
    assert_eq!(o, fold_reference(&input).to_be_bytes().to_vec());
}

#[test]
fn body_kernel_meta_errors_carry_through() {
    let cap = SpaceCap::new(0);
    // bad profile for a fitsh body
    let bad = BODY_CONTRACT.replace(
        "// calc: input_max = 4800",
        "// calc: profile = rv64imb\n// calc: input_max = 4800",
    );
    let err = single_body_kernel(Path::new("Fold.fitsh"), &bad, &cap).unwrap_err();
    assert!(err.contains("rv64im/rv64imc"), "{err}");

    // unknown key
    let bad2 = BODY_CONTRACT.replace("// calc: input_max = 4800", "// calc: turbo = on");
    let err2 = single_body_kernel(Path::new("Fold.fitsh"), &bad2, &cap).unwrap_err();
    assert!(err2.contains("unknown '// calc:' key"), "{err2}");
}

#[test]
fn body_kernel_rejects_outside_subset() {
    let cap = SpaceCap::new(0);
    // self-call inside a kernel body: outside the pinned subset
    let bad = BODY_CONTRACT.replace("s = (s * 31 + byte(data, i)) % 1000000007", "s = fold(data) as u64 / 1");
    let err = match single_body_kernel(Path::new("Fold.fitsh"), &bad, &cap) {
        Ok(_) => panic!("expected subset rejection"),
        Err(e) => e,
    };
    assert!(
        err.contains("outside the translatable kernel subset") || err.contains("rv64im kernel 'fold'"),
        "{err}"
    );
}

#[test]
fn no_body_kernel_reports_clearly() {
    let cap = SpaceCap::new(0);
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x }\n}\n";
    let err = single_body_kernel(Path::new("D.fitsh"), src, &cap).unwrap_err();
    assert!(err.contains("no native fitsh kernel body"), "{err}");
}

#[test]
fn external_override_wins_over_body() {
    // A prebuilt container override redirects the body kernel to a file —
    // use the body-compiled container itself as the override (round trip).
    let cap = SpaceCap::new(0);
    let (name, bytes) = single_body_kernel(Path::new("Fold.fitsh"), BODY_CONTRACT, &cap).unwrap();
    let dir = std::env::temp_dir().join("fitshc-body-override");
    std::fs::create_dir_all(&dir).unwrap();
    let fitsh_path = dir.join("Fold.fitsh");
    std::fs::write(&fitsh_path, BODY_CONTRACT).unwrap();
    std::fs::write(dir.join("fold.hcf1"), &bytes).unwrap();
    // file resolution order: fold.hcf1 (prebuilt) shadows the body? No —
    // the body wins over files (it is part of the declaration); but an
    // explicit --calc override must win over everything.
    let cfg = BuildConfig {
        overrides: [("fold".to_string(), dir.join("fold.hcf1"))]
            .into_iter()
            .collect(),
        ..BuildConfig::default()
    };
    let (_out, artifacts, _w) =
        compile_fitsh_with_calcs(BODY_CONTRACT, &fitsh_path, &cfg, &cap).unwrap();
    assert_eq!(artifacts[0].origin, "prebuilt-container");
    assert_eq!(artifacts[0].name, name);
}
