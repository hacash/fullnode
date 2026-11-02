#[test]
fn kernel_dict_walk_smoke() {
    use vm::calc::{CalcFnObj, CalcRunLimits};
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sha256kernel");
    let code = std::fs::read(format!("{dir}/image_code.bin")).unwrap();
    let rodata = std::fs::read(format!("{dir}/image_rodata.bin")).unwrap();
    let init = std::fs::read(format!("{dir}/image_init.bin")).unwrap();
    let entry: u32 = u32::from_str_radix(
        std::fs::read_to_string(format!("{dir}/entry.txt"))
            .expect("entry.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap_or(vm::calc::CALC_CODE_BASE as u32);
    let bss_len = u16::from_str_radix(
        std::fs::read_to_string(format!("{dir}/bss_len.txt"))
            .unwrap()
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap() as u64;
    let container = vm::calc::pack_container_with_bss(
        vm::calc::CALC_PROFILE_RV64IM,
        8,
        entry,
        256,
        32,
        code,
        rodata,
        init,
        bss_len,
        &vm::calc::CALC_CONFIG_V1,
    )
    .unwrap();
    let f = CalcFnObj::build(container).unwrap();
    let limits = CalcRunLimits { cycles: 2_000_000, resource: i64::MAX, deadline: None };
    // raw mode: sha256("abc")
    let e = f.run(b"abc", limits);
    assert!(e.result.is_ok(), "{:?}", e.result);
    // dict mode
    let doc = b"{start=beta,beta=gamma,gamma=end,other=42,zz=9}";
    let e = f.run(doc, limits);
    assert!(e.result.is_ok(), "{:?}", e.result);
    let out = e.result.unwrap();
    assert_eq!(out.len(), 32);
    // reference: sha256("betagamma" + "42" + "end") via the vm hash builtin
    let mut want = [0u8; 32];
    vm::calc::run_hash(vm::calc::HashKind::Sha2, b"betagammaend", &mut want);
    assert_eq!(&out[..], &want[..], "dict-walk digest mismatch");
    println!("dict-walk ok, cycles {}", e.cycles_used);
}
