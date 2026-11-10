//! End-to-end gas tests for pure calc calls (`CALLSELFPURE` / `CALLUSEPURE`):
//! deploy a contract carrying a calc kernel, call it through the HVM
//! interpreter and assert gas on success, trap and budget-exhaustion.

#![cfg(test)]
#![cfg(feature = "execute")]

use crate::calc::container::CalcContainer;
use crate::contract::{ContractAbstCall, ContractCalcFunc, ContractEdit, ContractSto, ContractUserFunc};
use crate::machine::test_ctx::TestCtx;
use crate::machine::VmRequest;
use crate::rt::{
    AbstCall, Bytecode::*, CodePkg, CodeType, CodeStuff, FuncArgvTypes, GasExtra, SpaceCap,
    calc_func_sign,
};
use crate::state::{VMState, VMStateRead};
use crate::value::ContractAddress;
use crate::Value;
use base::Context as _;

fn container_with(code: Vec<u8>) -> CalcContainer {
    CalcContainer::from_code(
            crate::calc::CALC_PROFILE_RV64IM,
            8,
            crate::calc::CALC_CODE_BASE as u32,
            code,
            128,
            128,
        )
}

fn halt_len(n: i32) -> Vec<u8> {
    let addi = |rd: u8, imm: i32| -> [u8; 4] {
        (0x13u32 | ((rd as u32) << 7) | (((imm as u32) & 0xFFF) << 20)).to_le_bytes()
    };
    let mut v = Vec::new();
    v.extend_from_slice(&addi(10, 0));
    v.extend_from_slice(&addi(11, n));
    v.extend_from_slice(&addi(17, 0));
    v.extend_from_slice(&0x0000_0073u32.to_le_bytes());
    v
}

/// lbu t0,0(a0) ; sd t0,0(a2) ; HALT with a1=8 — echoes 8 bytes.
fn echo8_container() -> CalcContainer {
    let mut code = vec![0x83, 0x42, 0x05, 0x00, 0x23, 0x30, 0x56, 0x00];
    code.extend_from_slice(&halt_len(8));
    container_with(code)
}

/// jal x0, 0 self-loop.
fn loop_container() -> CalcContainer {
    container_with(vec![0x6f, 0x00, 0x00, 0x00])
}

/// addi t0,x0,0 ; sd t0,0(t0) ; HALT — stores into the unmapped/code region (trap).
fn trap_container() -> CalcContainer {
    let mut code = vec![0x93, 0x28, 0x02, 0x00, 0x23, 0xb0, 0x52, 0x00];
    code.extend_from_slice(&halt_len(0));
    container_with(code)
}

fn make_sto(container: &CalcContainer) -> ContractSto {
    let pkg = CodePkg { conf: CodeType::RV64IM as u8, data: container.encode().expect("encode") };
    let mut cf = ContractUserFunc::default();
    cf.sign = field::Fixed4::from(calc_func_sign("echo8"));
    cf.code_stuff = CodeStuff::try_from(pkg).unwrap();
    let mut sto = ContractSto::default();
    sto.userfuncs.push(cf).unwrap();
    sto
}

/// Push input bytes; CALLSELFPURE echo8; POP the result; RET (nil for Construct).
fn abst_body(input: &[u8]) -> Vec<u8> {
    let mut prog = vec![PBUF as u8, input.len() as u8];
    prog.extend_from_slice(input);
    prog.push(CALLSELFPURE as u8);
    prog.extend_from_slice(&calc_func_sign("echo8"));
    prog.push(POP as u8);
    prog.push(RET as u8);
    prog
}

fn install(ctx: &mut TestCtx, sto: &ContractSto, addr: &ContractAddress) {
    let mut st = VMState::wrap(&mut ctx.layer);
    st.contract_set(addr, sto);
    st.contract_edition_set(addr, &sto.calc_edition());
}

fn run_construct(
    ctx: &mut TestCtx,
    addr: &ContractAddress,
    body: Vec<u8>,
) -> sys::Ret<(base::GasBuckets, Value)> {
    let mut sto = VMStateRead::wrap(&ctx.layer).contract(addr).unwrap().unwrap();
    let mut abst = ContractAbstCall::default();
    abst.sign = field::Fixed1::from([AbstCall::Construct as u8]);
    abst.code_stuff =
        CodeStuff::try_from(CodePkg { conf: CodeType::Bytecode as u8, data: body }).unwrap();
    sto.abstcalls.push(abst).unwrap();
    install(ctx, &sto, addr);
    ctx.vm_call(base::VmEntry::Raw(Box::new(VmRequest::Abst {
        kind: AbstCall::Construct,
        contract_addr: addr.clone(),
        intent_scope: None,
        param: Value::Nil,
    })))
    .map(|(g, v)| {
        let v = *v.downcast::<Value>().unwrap();
        (g, v)
    })
}

#[test]
fn deploy_check_accepts_calcfunc() {
    let sto = make_sto(&echo8_container());
    let params = crate::machine::test_ctx::STUB_VM_PARAMS;
    let gas = GasExtra::new(0, &params);
    let cap = SpaceCap::new(0);
    let reg: std::sync::Arc<dyn base::ExecutionServices> = std::sync::Arc::new(crate::machine::test_ctx::StubServices::new(params.clone()));
    sto.check(0, &cap, &gas, reg.as_ref()).expect("valid container accepted");

    let mut bad = make_sto(&echo8_container());
    bad.userfuncs.as_mut()[0].fncnf = field::Fixed1::from([0x80]);
    let err = bad.check(0, &cap, &gas, reg.as_ref()).expect_err("external calc");
    assert!(err.1.contains("cannot be external"), "{}", err.1);

    let mut reserved = ContractSto::default();
    reserved.calcfuncs.push(ContractCalcFunc::default()).unwrap();
    let err = reserved.check(0, &cap, &gas, reg.as_ref()).expect_err("reserved");
    assert!(err.1.contains("calcfuncs reserved"), "{}", err.1);

    // garbage container bytes must be rejected too
    let mut bad2 = make_sto(&echo8_container());
    let pkg = CodePkg { conf: CodeType::RV64IM as u8, data: vec![0xff; 40] };
    bad2.userfuncs.as_mut()[0].code_stuff = CodeStuff::try_from(pkg).unwrap();
    assert!(bad2.check(0, &cap, &gas, reg.as_ref()).is_err());
}

#[test]
fn deploy_check_height_gate_restores_pre_calc_behavior() {
    let mut sto = make_sto(&echo8_container());
    let params = crate::machine::test_ctx::STUB_VM_PARAMS;
    let gas = GasExtra::new(0, &params);
    let cap = SpaceCap::new(0);
    let reg: std::sync::Arc<dyn base::ExecutionServices> =
        std::sync::Arc::new(crate::machine::test_ctx::StubServices::new(params.clone()));
    let gate = crate::contract::CALCFUNC_DISABLE_FROM_HEIGHT;

    // inside the test window the container validates
    sto.check(gate - 1, &cap, &gas, reg.as_ref()).expect("window open below the gate");

    // at and past the gate the pre-calc rejection is restored, byte-identical
    for hei in [gate, 790_001, 793_880] {
        let e = sto
            .check(hei, &cap, &gas, reg.as_ref())
            .expect_err("calcfunc must be rejected past the gate");
        assert_eq!(e.0, crate::rt::ItrErrCode::ContractError);
        assert!(e.1.contains("calcfunc not enabled yet"), "got: {}", e.1);
    }

    // the upgrade path is gated the same way
    let mut edit = ContractEdit::default();
    edit.new_revision = field::Uint2::from(1u16);
    edit.userfuncs = sto.userfuncs.clone();
    let err = sto
        .apply_edit(&edit, gate, &cap, &gas, reg.as_ref())
        .expect_err("calcfunc edit must be rejected past the gate");
    assert!(err.1.contains("calcfunc not enabled yet"), "got: {}", err.1);
    sto.apply_edit(&edit, gate - 1, &cap, &gas, reg.as_ref())
        .expect("calcfunc edit accepted inside the window");
}

#[test]
fn calc_call_success_gas_deterministic() {
    let sto = make_sto(&echo8_container());
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    let addr = ContractAddress::must([1u8, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7]);
    install(&mut ctx, &sto, &addr);
    let input = b"calc-call-gas-test!!".to_vec();
    // Construct must return nil, so the echoed bytes are POPped; the kernel
    // output itself is asserted via the keep-result variant below.
    let (gas, val) = run_construct(&mut ctx, &addr, abst_body(&input)).expect("run ok");
    assert!(matches!(val, Value::Nil), "got {val:?}");
    assert!(gas.compute > 0 && gas.resource > 0, "gas={gas:?}");
    // determinism: fresh ctx, identical program -> bit-identical gas
    let sto2 = make_sto(&echo8_container());
    let mut ctx2 = TestCtx::new();
    ctx2.install_native_vm();
    let addr2 = ContractAddress::must([1u8, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7]);
    install(&mut ctx2, &sto2, &addr2);
    let (gas2, _) = run_construct(&mut ctx2, &addr2, abst_body(&input)).expect("rerun ok");
    assert_eq!(gas, gas2, "gas must be bit-identical across runs");

    // keep-result variant: without the POP the Construct return check
    // reverts, but the error carries the kernel output — proving the echoed
    // bytes reached the operand stack — and gas was still charged.
    let mut body = vec![PBUF as u8, input.len() as u8];
    body.extend_from_slice(&input);
    body.push(CALLSELFPURE as u8);
    body.extend_from_slice(&calc_func_sign("echo8"));
    body.push(RET as u8);
    let err = run_construct(&mut ctx, &addr, body).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("6300"), "kernel output must surface in return: {msg}");
}

#[test]
fn calc_call_trap_charges_consumed_only() {
    let sto = make_sto(&trap_container());
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    let addr = ContractAddress::must([1u8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8]);
    install(&mut ctx, &sto, &addr);
    let err = run_construct(&mut ctx, &addr, abst_body(b"x")).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("CalcTrap") || msg.contains("calc"), "err={msg}");
}

#[test]
fn calc_call_host_remaining_caps_unpaid_cycles() {
    let sto = make_sto(&loop_container());
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    ctx.gas = 500;
    let addr = ContractAddress::must([1u8, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]);
    install(&mut ctx, &sto, &addr);
    let err = run_construct(&mut ctx, &addr, abst_body(b"loop")).unwrap_err();
    let msg = err.to_string();
    // The cycle budget is host gas left after the resource reservation.
    // That must be what stops the loop. Ignoring host remaining would size
    // the run from an unlimited compute_limit and then fail settle with
    // "test ctx out of gas".
    assert!(
        msg.contains("calc cycle budget exhausted"),
        "host remaining must cap calc cycles before settle overruns: {msg}"
    );
}

#[test]
fn calc_call_nil_rejected_when_pmdf_declares_bytes() {
    let mut sto = make_sto(&echo8_container());
    sto.userfuncs.as_mut()[0].pmdf =
        FuncArgvTypes::from_types(Some(crate::ValueTy::Bytes), vec![crate::ValueTy::Bytes])
            .unwrap();
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    let addr = ContractAddress::must([1u8, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6]);
    install(&mut ctx, &sto, &addr);
    let body = vec![
        PNIL as u8,
        CALLSELFPURE as u8,
    ]
    .into_iter()
    .chain(calc_func_sign("echo8"))
    .chain([RET as u8])
    .collect();
    let err = run_construct(&mut ctx, &addr, body).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("CallArgvTypeFail") || msg.contains("type"),
        "Nil must not coerce to empty bytes when pmdf is [Bytes]: {msg}"
    );
}

#[test]
fn calc_call_infinite_loop_stops_at_budget() {
    let sto = make_sto(&loop_container());
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    // Default test gas is 1<<30. A self-loop is bounded only by remaining
    // compute, so give this call a small bucket or it runs for minutes.
    ctx.gas = 2_000;
    let addr = ContractAddress::must([1u8, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9]);
    install(&mut ctx, &sto, &addr);
    let err = run_construct(&mut ctx, &addr, abst_body(b"loop")).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("OutOfGas") || msg.contains("gas limit") || msg.contains("budget"),
        "err={msg}"
    );
}

fn sha256_container_bytes() -> Vec<u8> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sha256kernel");
    let entry: u32 = u32::from_str_radix(
        std::fs::read_to_string(format!("{dir}/entry.txt"))
            .expect("entry.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .expect("entry");
    let read = |name: &str| std::fs::read(format!("{dir}/{name}")).expect(name);
    let bss_len = u16::from_str_radix(
        std::fs::read_to_string(format!("{dir}/bss_len.txt"))
            .expect("bss_len.txt")
            .trim()
            .trim_start_matches("0x"),
        16,
    )
    .expect("bss_len") as u32;
    let tmp = crate::calc::container::CalcContainer::from_regions(
        crate::calc::CALC_PROFILE_RV64IM,
        // 8 KiB fit the old no-guard layout; the guard band + minimum stack
        // (4 KiB above the image) needs this headroom for bss 0x400.
        10,
        entry,
        read("image_code.bin"),
        read("image_rodata.bin"),
        read("image_init.bin"),
        256,
        32,
    )
    .with_bss(bss_len);
    crate::calc::pack_container(
        &crate::calc::SpaceCap::new(0),
        tmp.profile,
        tmp.mem_kib,
        tmp.entry,
        tmp.input_max,
        tmp.output_max,
        tmp.segs,
    )
    .expect("pack sha256")
    .encode()
    .expect("encode sha256")
}

fn run_external(
    sto: &ContractSto,
    addr: &ContractAddress,
    func: &str,
    input: &[u8],
) -> sys::Ret<(base::GasBuckets, Value)> {
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    ctx.env.tx.addrs = vec![addr.to_addr()];
    install(&mut ctx, sto, addr);
    let mut prog = vec![PBUF as u8, input.len() as u8];
    prog.extend_from_slice(input);
    prog.push(CALLEXT as u8);
    prog.push(0);
    prog.extend_from_slice(&calc_func_sign(func));
    prog.push(RET as u8);
    ctx.vm_call(base::VmEntry::Raw(Box::new(VmRequest::SandboxMain {
        code_type: CodeType::Bytecode,
        codes: std::sync::Arc::from(prog),
    })))
    .map(|(g, v)| {
        let v = *v.downcast::<Value>().unwrap();
        (g, v)
    })
}

#[test]
fn fitsh_calc_executes_echo8_and_sha256_and_rejects_sign_collision() {
    let blob = format!("0x{}", hex::encode({
        let c = echo8_container();
        c.encode().unwrap()
    }));
    let src = format!(
        r#"
pragma fitsh 1.0.0
contract EchoExec {{
    function rv64im echo8(data: bytes) -> bytes = {blob}
    function external probe(data: bytes) -> bytes {{
        return self::echo8(data)
    }}
}}
"#
    );
    let (contract, _, _, _) = crate::fitshc::compile(&src).expect("compile echo");
    let sto = contract.into_sto();
    let params = crate::machine::test_ctx::STUB_VM_PARAMS;
    let gas = GasExtra::new(0, &params);
    let cap = SpaceCap::new(0);
    let reg: std::sync::Arc<dyn base::ExecutionServices> =
        std::sync::Arc::new(crate::machine::test_ctx::StubServices::new(params));
    sto.check(0, &cap, &gas, reg.as_ref()).expect("echo contract checks");
    assert_eq!(sto.calcfuncs.length(), 0);
    assert_eq!(sto.userfuncs[0].sign.0, calc_func_sign("echo8"));
    assert_eq!(sto.userfuncs[0].code_stuff.conf.uint(), CodeType::RV64IM as u8);

    let addr = ContractAddress::must([1u8, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3]);
    let input = b"abcdefghij";
    let (spent, val) = run_external(&sto, &addr, "probe", input).expect("echo8 run");
    assert!(spent.compute > 0 && spent.resource > 0, "gas={spent:?}");
    match val {
        Value::Bytes(buf) => assert_eq!(buf, vec![b'a', 0, 0, 0, 0, 0, 0, 0]),
        other => panic!("echo8 returned {other:?}"),
    }

    let sha_src = r#"
pragma fitsh 1.0.0
contract ShaExec {
    function rv64im sha256kernel(data: bytes) -> bytes
    function external digest(data: bytes) -> bytes {
        return self::sha256kernel(data)
    }
}
"#;
    let (contract, _, _, _) = crate::fitshc::compile_with_rv64im(
        sha_src,
        &[("sha256kernel", sha256_container_bytes())],
    )
    .expect("compile sha256");
    let sto = contract.into_sto();
    sto.check(0, &cap, &gas, reg.as_ref()).expect("sha256 contract checks");
    assert_eq!(sto.calcfuncs.length(), 0);
    let sha = sto
        .userfuncs
        .iter()
        .find(|f| f.sign.0 == calc_func_sign("sha256kernel"))
        .expect("sha256kernel");
    assert_eq!(sha.code_stuff.conf.uint(), CodeType::RV64IM as u8);
    let addr = ContractAddress::must([1u8, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4]);
    let (spent, val) = run_external(&sto, &addr, "digest", b"abc").expect("sha256 run");
    assert!(spent.compute > 0 && spent.resource > 0, "gas={spent:?}");
    match val {
        Value::Bytes(buf) => assert_eq!(buf, crate::calc::sha256_bytes(b"abc").to_vec()),
        other => panic!("sha256 returned {other:?}"),
    }

    let mut bad = make_sto(&echo8_container());
    let mut user = ContractUserFunc::default();
    user.sign = bad.userfuncs[0].sign.clone();
    bad.userfuncs.push(user).unwrap();
    let err = bad.check(0, &cap, &gas, reg.as_ref()).expect_err("collision");
    assert!(
        err.1.contains("userfunc sign already exists"),
        "err={}",
        err.1
    );
}

#[test]
fn library_calc_callusepure_and_non_pure_rejected() {
    let blob = format!("0x{}", hex::encode(echo8_container().encode().unwrap()));
    let kern_addr = ContractAddress::must([1u8, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5]);
    let caller_addr = ContractAddress::must([1u8, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6]);
    let kern_src = format!(
        r#"
pragma fitsh 1.0.0
contract Kern {{
    function rv64im echo8(data: bytes) -> bytes = {blob}
    function external bad_edit(data: bytes) -> bytes {{ return self.echo8(data) }}
    function external bad_view(data: bytes) -> bytes {{ return self:echo8(data) }}
}}
"#
    );
    let (kern, _, _, _) = crate::fitshc::compile(&kern_src).expect("compile kern");
    let kern_sto = kern.into_sto();
    let caller_src = format!(
        r#"
pragma fitsh 1.0.0
contract Caller {{
    library [Kern: {}]
    function external probe(data: bytes) -> bytes {{
        return Kern::echo8(data)
    }}
}}
"#,
        kern_addr.to_readable()
    );
    let (caller, _, _, _) = crate::fitshc::compile(&caller_src).expect("compile caller");
    let caller_sto = caller.into_sto();
    let probe = caller_sto
        .userfuncs
        .iter()
        .find(|f| f.sign.0 == calc_func_sign("probe"))
        .expect("probe");
    let mut needle = vec![CALLUSEPURE as u8, 0];
    needle.extend_from_slice(&calc_func_sign("echo8"));
    assert!(
        probe.code_stuff.data.windows(needle.len()).any(|w| w == needle.as_slice()),
        "missing CALLUSEPURE: {:?}",
        probe.code_stuff.data
    );

    let params = crate::machine::test_ctx::STUB_VM_PARAMS;
    let gas = GasExtra::new(0, &params);
    let cap = SpaceCap::new(0);
    let reg: std::sync::Arc<dyn base::ExecutionServices> =
        std::sync::Arc::new(crate::machine::test_ctx::StubServices::new(params));
    kern_sto.check(0, &cap, &gas, reg.as_ref()).expect("kern checks");
    caller_sto.check(0, &cap, &gas, reg.as_ref()).expect("caller checks");

    let input = b"abcdefghij";
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    ctx.env.tx.addrs = vec![caller_addr.to_addr()];
    install(&mut ctx, &kern_sto, &kern_addr);
    install(&mut ctx, &caller_sto, &caller_addr);
    let mut prog = vec![PBUF as u8, input.len() as u8];
    prog.extend_from_slice(input);
    prog.push(CALLEXT as u8);
    prog.push(0);
    prog.extend_from_slice(&calc_func_sign("probe"));
    prog.push(RET as u8);
    let (spent, val) = ctx
        .vm_call(base::VmEntry::Raw(Box::new(VmRequest::SandboxMain {
            code_type: CodeType::Bytecode,
            codes: std::sync::Arc::from(prog),
        })))
        .map(|(g, v)| {
            let v = *v.downcast::<Value>().unwrap();
            (g, v)
        })
        .expect("lib echo8");
    assert!(spent.compute > 0 && spent.resource > 0, "gas={spent:?}");
    match val {
        Value::Bytes(buf) => assert_eq!(buf, vec![b'a', 0, 0, 0, 0, 0, 0, 0]),
        other => panic!("library echo8 returned {other:?}"),
    }

    for name in ["bad_edit", "bad_view"] {
        let err = run_external(&kern_sto, &kern_addr, name, input).expect_err(name);
        let msg = err.to_string();
        assert!(msg.contains("requires pure call"), "{name}: {msg}");
    }
}

/// Halt with empty output when `a1 == expect`, otherwise store to address 0.
fn expect_len_kernel(expect: u32) -> Vec<u8> {
    let load_expect = if expect == 4096 {
        // lui t0, 1
        0x0000_12b7u32.to_le_bytes()
    } else {
        assert!(expect <= 2047, "addi imm");
        // addi t0, x0, expect
        (0x13u32 | (5 << 7) | (expect << 20)).to_le_bytes()
    };
    let mut code = Vec::new();
    code.extend_from_slice(&load_expect);
    // bne a1, t0, +20 (faulting store past the halt)
    code.extend_from_slice(&0x0055_9a63u32.to_le_bytes());
    code.extend_from_slice(&halt_len(0));
    // addi t1, x0, 0; sd t1, 0(t1)
    code.extend_from_slice(&0x0000_0313u32.to_le_bytes());
    code.extend_from_slice(&0x0063_3023u32.to_le_bytes());
    code
}

fn sto_named(name: &str, container: &CalcContainer) -> ContractSto {
    let pkg = CodePkg {
        conf: CodeType::RV64IM as u8,
        data: container.encode().expect("encode"),
    };
    let mut cf = ContractUserFunc::default();
    cf.sign = field::Fixed4::from(calc_func_sign(name));
    cf.code_stuff = CodeStuff::try_from(pkg).unwrap();
    let mut sto = ContractSto::default();
    sto.userfuncs.push(cf).unwrap();
    sto
}

fn wide_container(code: Vec<u8>, input_max: u16, output_max: u16) -> CalcContainer {
    let mut c = container_with(code);
    c.mem_kib = 64;
    c.input_max = input_max;
    c.output_max = output_max;
    c
}

fn push_pbuf(prog: &mut Vec<u8>, data: &[u8]) {
    assert!(data.len() <= 255);
    prog.push(PBUF as u8);
    prog.push(data.len() as u8);
    prog.extend_from_slice(data);
}

fn push_pbufl(prog: &mut Vec<u8>, data: &[u8]) {
    assert!(data.len() <= 1280);
    prog.push(PBUFL as u8);
    prog.extend_from_slice(&(u16::try_from(data.len()).unwrap()).to_be_bytes());
    prog.extend_from_slice(data);
}

fn call_pure(name: &str, chunks: &[Vec<u8>], pack: u8) -> Vec<u8> {
    let mut prog = Vec::new();
    for chunk in chunks {
        if chunk.len() <= 255 {
            push_pbuf(&mut prog, chunk);
        } else {
            push_pbufl(&mut prog, chunk);
        }
    }
    if chunks.len() != 1 || pack != 0 {
        prog.push(PU8 as u8);
        prog.push(chunks.len() as u8);
        prog.push(pack);
    }
    prog.push(CALLSELFPURE as u8);
    prog.extend_from_slice(&calc_func_sign(name));
    prog.push(POP as u8);
    prog.push(RET as u8);
    prog
}

fn run_named(container: &CalcContainer, name: &str, body: Vec<u8>) -> sys::Ret<()> {
    let sto = sto_named(name, container);
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    let addr = ContractAddress::must([
        1, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    ]);
    install(&mut ctx, &sto, &addr);
    let (_, val) = run_construct(&mut ctx, &addr, body)?;
    assert!(matches!(val, Value::Nil), "got {val:?}");
    Ok(())
}

#[test]
fn rv64im_list_inputs_cross_value_slot() {
    let wide = wide_container(expect_len_kernel(4096), 4096, 16);
    let chunks = vec![
        vec![1u8; 1280],
        vec![2u8; 1280],
        vec![3u8; 1280],
        vec![4u8; 256],
    ];
    run_named(&wide, "wide", call_pure("wide", &chunks, PACKLIST as u8)).expect("list 4096");

    let mut over = chunks.clone();
    over[3] = vec![4u8; 257];
    let err = run_named(&wide, "wide", call_pure("wide", &over, PACKLIST as u8)).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("CalcTrap") || msg.contains("calc kernel trap"),
        "err={msg}"
    );

    let list_c = wide_container(expect_len_kernel(150), 4096, 16);
    let parts = vec![vec![9u8; 100], vec![8u8; 50]];
    // A Tuple is packed argv, not a byte-string carrier: rejected before the
    // calc call. The same chunks in a one-layer list flatten.
    let err = run_named(&list_c, "two", call_pure("two", &parts, PACKTUPLE as u8)).unwrap_err();
    assert!(
        err.to_string().contains("CastBeCallDataFail"),
        "err={err}"
    );
    run_named(&list_c, "two", call_pure("two", &parts, PACKLIST as u8)).expect("list concat");

    let one = wide_container(expect_len_kernel(1280), 4096, 16);
    run_named(
        &one,
        "one",
        call_pure("one", &[vec![7u8; 1280]], 0),
    )
    .expect("single bytes slot");

    let params = crate::machine::test_ctx::STUB_VM_PARAMS;
    let gas = GasExtra::new(0, &params);
    let cap = SpaceCap::new(0);
    let reg: std::sync::Arc<dyn base::ExecutionServices> =
        std::sync::Arc::new(crate::machine::test_ctx::StubServices::new(params));
    let fat = sto_named("fat", &wide_container(halt_len(0), 16, 4096));
    let err = fat.check(0, &cap, &gas, reg.as_ref()).unwrap_err();
    assert!(
        err.1.contains("output_max exceeds HVM value_size"),
        "{}",
        err.1
    );

    let echoed = wide_container(halt_len(8), 128, 128);
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    let addr = ContractAddress::must([
        1, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    ]);
    install(&mut ctx, &sto_named("out8", &echoed), &addr);
    let mut body = call_pure("out8", &[b"xxxxxxxx".to_vec()], 0);
    body.pop();
    body.pop();
    body.push(RET as u8);
    let (_, val) = run_construct(&mut ctx, &addr, body).expect("8-byte output fits value_size");
    assert_eq!(val, Value::Bytes(vec![0; 8]));
}

/// A compiled `function rv64im` declares `(data: bytes)`. Feeding it a
/// one-layer `list { ... }` must be allowed (it is the deferred-CAT form,
/// flattened under `call_data_size`, with the kernel's `input_max` checked by
/// the calc pre-flight), while `tuple(...)` is packed argv and is rejected.
/// An oversized *scalar* must still hit the 1,280 B HVM cap.
#[test]
fn compiled_rv64im_list_input_crosses_scalar_cap_but_scalar_does_not() {
    let container = wide_container(expect_len_kernel(4096), 4096, 16);
    let image = container.encode().expect("encode");
    let src = r#"
pragma fitsh 1.0.0
contract WideCompiled {
    function rv64im probe(data: bytes) -> bytes
    function external call4(a: bytes, b: bytes, c: bytes, d: bytes) -> bytes {
        return self::probe(list { a, b, c, d })
    }
    function external call4t(a: bytes, b: bytes, c: bytes, d: bytes) -> bytes {
        return self::probe(tuple(a, b, c, d))
    }
    function external call1(a: bytes) -> bytes {
        return self::probe(a)
    }
}
"#;
    let (contract, _, _, _) = crate::fitshc::compile_with_rv64im(src, &[("probe", image)])
        .expect("compiled rv64im contract");
    let sto = contract.into_sto();
    let addr = ContractAddress::must([
        1, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    ]);
    let chunks = [
        Value::Bytes(vec![1u8; 1024]),
        Value::Bytes(vec![2u8; 1024]),
        Value::Bytes(vec![3u8; 1024]),
        Value::Bytes(vec![4u8; 1024]),
    ];
    let (_, val) = run_external_args(&sto, &addr, "call4", &chunks).expect("list 4096");
    assert_eq!(val, Value::Bytes(vec![]), "kernel saw the full 4 KiB");

    let err = run_external_args(&sto, &addr, "call4t", &chunks).unwrap_err();
    assert!(
        err.to_string().contains("CallArgvTypeFail"),
        "tuple is packed argv, not bytes: {err}"
    );

    let one = [Value::Bytes(vec![7u8; 4096])];
    let err = run_external_args(&sto, &addr, "call1", &one).unwrap_err();
    assert!(
        err.to_string().contains("OutOfValueSize"),
        "scalar 4096 must still be capped: {err}"
    );
}

fn run_external_args(
    sto: &ContractSto,
    addr: &ContractAddress,
    func: &str,
    args: &[Value],
) -> sys::Ret<(base::GasBuckets, Value)> {
    let mut ctx = TestCtx::new();
    ctx.install_native_vm();
    ctx.env.tx.addrs = vec![addr.to_addr()];
    install(&mut ctx, sto, addr);
    let mut prog = crate::machine::build_call_codes(func, args)?;
    // `build_call_codes` targets the sandbox library (index 1); a plain
    // SandboxMain program calls the deployed contract with index 0.
    let n = prog.len();
    prog[n - 6] = 0;
    ctx.vm_call(base::VmEntry::Raw(Box::new(VmRequest::SandboxMain {
        code_type: CodeType::Bytecode,
        codes: std::sync::Arc::from(prog),
    })))
    .map(|(g, v)| {
        let v = *v.downcast::<Value>().unwrap();
        (g, v)
    })
}
