//! Real-kernel integration test for the calc interpreter.
//!
//! Runs the prebuilt rustc→RV64IM kernel fixture (loop + table lookup +
//! multiply/divide mixing — a hash-style mixing workload) through the
//! container verifier and interpreter, and compares the digest against the
//! value cross-validated under qemu-riscv64-user and ckb-vm 0.24.13 in the
//! research phase (`doc/calcfunc-rv64im-research.md` §4.4).
//!
//! The fixture is committed; rebuild it with
//! `vm/tests/fixtures/calckernel/build.sh` (needs the riscv64imac target).

#[cfg(feature = "execute")]
#[test]
fn calckernel_fixture_digest_matches_oracles() {
    use vm::calc::container::CalcContainer;
    use vm::calc::verify::verify_container;
    use vm::calc::interp;

    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/calckernel");
    let code = std::fs::read(format!("{dir}/image_code.bin")).expect("fixture image_code.bin");
    let rodata = std::fs::read(format!("{dir}/image_rodata.bin")).expect("fixture image_rodata");
    let init = std::fs::read(format!("{dir}/image_init.bin")).expect("fixture image_init");
    let entry: u32 = u32::from_str_radix(
        std::fs::read_to_string(format!("{dir}/entry.txt"))
            .expect("fixture entry.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .expect("entry hex");

    let container = CalcContainer::from_regions(
        vm::calc::CALC_PROFILE_RV64IM,
        16,
        entry,
        code,
        rodata,
        init,
        64,
        64,
    );
    let module = verify_container(&container).expect("container verifies");

    // Input identical to the oracle harness's KINPUT.
    let input: [u8; 64] = [
        0x61, 0x63, 0x64, 0x63, 0x61, 0x63, 0x21, 0x68, 0x61, 0x63, 0x61, 0x73, 0x68, 0x21,
        0x72, 0x76, 0x36, 0x34, 0x69, 0x6d, 0x21, 0x6b, 0x65, 0x72, 0x6e, 0x65, 0x6c, 0x21,
        0x74, 0x65, 0x73, 0x74, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
        0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
        0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10,
    ];
    // qemu-user 8.2.2 and ckb-vm 0.24.13 both produce (research §4.4):
    let expected = [
        0xff, 0x15, 0x21, 0x62, 0x93, 0x48, 0x7e, 0x76, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    let run = |budget: u64| interp::run(&module, &container, &input, budget, None).expect("run ok");
    let first = run(1_000_000);
    assert_eq!(first.output, expected, "digest must match the oracles");
    // Determinism: identical result and cycle count on re-run.
    let second = run(1_000_000);
    assert_eq!(first.cycles_used, second.cycles_used);
    assert_eq!(first.output, second.output);
    // The kernel must also survive a exactly-fitting budget and stop
    // deterministically one cycle-block below it.
    let tight = run(first.cycles_used);
    assert_eq!(tight.output, expected);
    let under = interp::run(&module, &container, &input, first.cycles_used - 1, None);
    assert!(
        matches!(under, Err((interp::CalcAbort::OutOfGas, _))),
        "budget below exact usage must abort"
    );
    println!(
        "calckernel fixture: {} bytes code + {} rodata, {} cycles for 64-byte input ({:.1} cycles/byte)",
        container.exec_filesz(),
        container.ro_filesz(),
        first.cycles_used,
        first.cycles_used as f64 / 64.0
    );
}

