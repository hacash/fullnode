#[test]
fn kernel_dict_walk_smoke() {
    use vm::calc::{CalcFnObj, CalcRunLimits};
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sha256kernel");
    let code = std::fs::read(format!("{dir}/image_code.bin")).unwrap();
    let rodata = std::fs::read(format!("{dir}/image_rodata.bin")).unwrap();
    let container = vm::calc::pack_container(8, 0, 256, 32, code, rodata, vec![]).unwrap();
    let f = CalcFnObj::build(container).unwrap();
    let limits = CalcRunLimits { cycles: 2_000_000, resource: i64::MAX, value_size: usize::MAX, deadline: None };
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
