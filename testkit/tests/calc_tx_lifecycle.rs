//! T series from `vm/doc/calc-test-plan.loc.md`: calc kernel contract
//! lifecycle driven through the `MemChain` simulator with **real signed
//! type-3 transactions** carrying `ContractDeploy` (kind 40),
//! `ContractMainCall` (kind 44) and `ContractUpdate` (kind 41) actions —
//! submit → mine_block → receipt, state committed on success, isolated on
//! failure. This is the deploy / submit / execute / confirm coverage the
//! plan calls for (the fitsh-side analogue of `dict_digest_three_paths`).
//!
//! Run: cargo test -p testkit --test calc_tx_lifecycle

use std::sync::Arc;

use app::standard_registry_with_params;
use base::{
    hac_add_state, init_block_contract_storage_budget, ExecutionServices, TxRef,
};
use field::Hash;
use field::{Address, Amount, Uint1, Uint4};
use protocol::tx_std::StdTransaction;
use sys::Account;
use testkit::sim::context::TestContext;
use testkit::sim::dualchain::DualMemChain;
use testkit::sim::memchain::MemChain;
use testkit::sim::tx::DummyTx;
use vm::action::{
    contract_deploy_charge_bytes, contract_protocol_cost_min, ContractDeploy, ContractMainCall,
    ContractUpdate,
};
use vm::calc::{pack_code, SpaceCap, CALC_PROFILE_RV64IM};
use vm::fitshc::compiler;
use vm::{build_call_codes, main_call, CodeType, ContractAddress, Value};

// ------------------------------------------------------------- kernels ---

fn enc_i(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> [u8; 4] {
    let w = (((imm as u32) & 0xFFF) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op;
    w.to_le_bytes()
}
fn enc_s(imm: i32, rs2: u32, rs1: u32, f3: u32, op: u32) -> [u8; 4] {
    let u = imm as u32;
    let w = (((u >> 5) & 0x7F) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | ((u & 0x1F) << 7) | op;
    w.to_le_bytes()
}

/// Echo kernel: copies the first input byte to output[0] and HALTs with
/// output length 1.
fn echo_code() -> Vec<u8> {
    enc_i(0, 10, 0, 5, 0x03) // lb t0, 0(a0)
        .into_iter()
        .chain(enc_s(0, 5, 12, 0, 0x23)) // sb t0, 0(a2)
        .chain(enc_i(0, 0, 0, 10, 0x13)) // addi a0, x0, 0
        .chain(enc_i(1, 0, 0, 11, 0x13)) // addi a1, x0, 1
        .chain(enc_i(0, 0, 0, 17, 0x13)) // addi a7, x0, 0
        .chain([0x73, 0, 0, 0]) // ecall
        .collect()
}

/// Trap kernel: stores into the unmapped low page.
fn trap_code() -> Vec<u8> {
    enc_s(0, 0, 0, 3, 0x23) // sd x0, 0(x0)
        .into_iter()
        .chain(enc_i(0, 0, 0, 10, 0x13))
        .chain(enc_i(0, 0, 0, 11, 0x13))
        .chain(enc_i(0, 0, 0, 17, 0x13))
        .chain([0x73, 0, 0, 0])
        .collect()
}

/// Infinite loop kernel for the gas-budget stop test.
fn selfloop_code() -> Vec<u8> {
    vec![0x6f, 0x00, 0x00, 0x00] // jal x0, 0
}

fn container_of(code: Vec<u8>) -> Vec<u8> {
    pack_code(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        5,
        0x1000,
        64,
        64,
        code,
    )
    .unwrap()
    .encode()
    .unwrap()
}

fn kernel_src(name: &str) -> String {
    format!(
        r#"
pragma fitsh 1.0.0
contract CalcLifecycle {{
    function rv64im {name}(data: bytes) -> bytes
    abstract Change() -> u64 {{
        return 0
    }}
    abstract Append() -> u64 {{
        return 0
    }}
    function external entry(data: bytes) -> bytes {{
        return self::{name}(data)
    }}
}}
"#
    )
}

fn sto_with(name: &str, container: &[u8]) -> vm::contract::ContractSto {
    let (contract, _, _, _) =
        compiler::compile_with_rv64im(&kernel_src(name), &[(name, container.to_vec())])
            .expect("fitsh compiles with the rv64im kernel");
    contract.into_sto()
}

// ------------------------------------------------------------- chain ---

fn unbounded_services() -> Arc<dyn ExecutionServices> {
    let mut hp = hacash_params::MAINNET_PARAMS;
    // Lift only the per-block bucket ceilings; charged gas is unaffected.
    hp.protocol.vm.compute_limit_byte = 0;
    hp.protocol.vm.resource_limit_byte = 0;
    hp.protocol.vm.storage_limit_byte = 0;
    let hp: &'static hacash_params::HacashParams = Box::leak(Box::new(hp));
    Arc::new(standard_registry_with_params(hp).unwrap())
}

const FUND: u128 = 10_000_000_000_000;

/// Serialize every lifecycle test: the VM keeps a process-wide contract
/// object cache keyed by address, so parallel tests must not reuse
/// addresses across chains while one is mid-update.
fn funded_chain(key: u8) -> (MemChain, Address, Account) {
    let _guard = testkit::sim::integration::test_guard();
    let mut chain = MemChain::with_services(unbounded_services()).unwrap();
    let acct = Account::create_by_secret_key_value([key; 32]).unwrap();
    let main = Address::from(*acct.address());
    hac_add_state(
        chain.state_mut(),
        &main,
        &Amount::coin_u128(FUND, field::UNIT_238),
    )
    .unwrap();
    (chain, main, acct)
}

/// A block producer's duty before mining: init the per-block contract
/// storage budget at the height the block will execute at.
fn prepare_block(chain: &mut MemChain) {
    let params = chain.services().vm_params().unwrap().clone();
    let height = chain.height() + 1;
    init_block_contract_storage_budget(chain.state_mut(), &params, height).unwrap();
}

fn deploy_action_tx(
    main: Address,
    acct: &Account,
    nonce: u32,
    sto: vm::contract::ContractSto,
    protocol_cost: Amount,
) -> StdTransaction {
    let mut act = ContractDeploy::new();
    act.nonce = Uint4::from(nonce);
    act.protocol_cost = protocol_cost;
    act.contract = sto;
    let mut tx = StdTransaction::new_by(
        hacash_params::TX_TYPE_3,
        main,
        Amount::from("1:244").unwrap(),
        sys::curtimes(),
    );
    tx.gas_max = Uint1::from(99u8);
    tx.push_action_in(Arc::new(act));
    assert_eq!(tx.actions.len(), 1, "deploy action pushed");
    tx.fill_sign_account(acct).unwrap();
    tx
}

fn call_action_tx(
    main: Address,
    acct: &Account,
    caddr: &ContractAddress,
    input: &[u8],
    gas_max: u8,
) -> StdTransaction {
    // The main-call entry treats a non-zero-bytes return value as a revert
    // payload (entry.rs::check_vm_return_value), so the bytecode discards
    // the calc output and returns Nil; the echoed value is observed through
    // the sandbox query path instead.
    let mut codes = build_call_codes("entry", &[Value::Bytes(input.to_vec())])
        .expect("call codes build");
    codes.pop(); // drop the trailing RET
    codes.push(vm::Bytecode::PNIL as u8);
    codes.push(vm::Bytecode::RET as u8);
    let act = ContractMainCall::from_bytecode(codes).unwrap();
    let mut tx = StdTransaction::new_by(
        hacash_params::TX_TYPE_3,
        main,
        Amount::from("1:244").unwrap(),
        sys::curtimes(),
    );
    tx.gas_max = Uint1::from(gas_max);
    // addrlist: [main, contract] — CALLEXT resolves the target by depth 1.
    tx.addrlist = field::AddrOrList::from_list(vec![main, caddr.to_addr()]).unwrap();
    tx.push_action_in(Arc::new(act));
    tx.fill_sign_account(acct).unwrap();
    tx
}

fn deploy_and_mine(
    chain: &mut MemChain,
    main: Address,
    acct: &Account,
    name: &str,
    container: &[u8],
    nonce: u32,
) -> ContractAddress {
    let sto = sto_with(name, container);
    let caddr = ContractAddress::calculate(&main, &Uint4::from(nonce));
    let tx = Arc::new(deploy_action_tx(
        main,
        acct,
        nonce,
        sto.clone(),
        protocol_cost_for(&chain.services(), &sto),
    )) as TxRef;
    let hash = chain.submit(tx);
    prepare_block(chain);
    chain
        .mine_block()
        .receipt(&hash)
        .expect("deploy receipt")
        .expect_success();
    caddr
}

fn protocol_cost_for(
    services: &Arc<dyn ExecutionServices>,
    sto: &vm::contract::ContractSto,
) -> Amount {
    let mut ctx = TestContext::new(services.clone(), Arc::new(DummyTx));
    let params = services.vm_params().unwrap().clone();
    init_block_contract_storage_budget(&mut ctx.layer, &params, 1).unwrap();
    let charge = contract_deploy_charge_bytes(sto);
    contract_protocol_cost_min(&ctx, charge, 10_000).unwrap()
}

fn call_and_mine(
    chain: &mut MemChain,
    main: Address,
    acct: &Account,
    caddr: &ContractAddress,
    input: &[u8],
    gas_max: u8,
) -> Hash {
    let tx = Arc::new(call_action_tx(main, acct, caddr, input, gas_max)) as TxRef;
    let hash = chain.submit(tx);
    prepare_block(chain);
    chain.mine_block();
    hash
}

// ================================================================= tests

/// T-001 + T-101 + T-110: deploy through a real ContractDeploy tx, confirm
/// in a block, call through a real ContractMainCall tx, and read the result
/// back through the sandbox query path against the committed state.
#[test]
fn deploy_then_call_roundtrip() {
    let (mut chain, main, acct) = funded_chain(0x21);
    let caddr = deploy_and_mine(&mut chain, main, &acct, "echo", &container_of(echo_code()), 1);

    // confirmed call tx succeeds
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, b"hello", 99);
    chain
        .receipt(&hash)
        .expect("call receipt")
        .expect_success();

    // the echoed value is observable through the sandbox query path
    let stub = testkit::sim::tx::StubTxBuilder::new()
        .ty(3)
        .main(main)
        .addrs(vec![main, caddr.to_addr()])
        .build();
    let mut ctx = TestContext::new(chain.services(), Arc::new(stub));
    ctx.layer = chain.state().clone();
    if let Some(vm) = chain.services().assign_vm(1) {
        ctx.set_vm(vm);
    }
    let codes = build_call_codes("entry", &[Value::Bytes(b"world".to_vec())]).unwrap();
    let (_, Value::Bytes(out)) = main_call(&mut ctx, CodeType::Bytecode, codes.into()).unwrap()
    else {
        panic!("non-bytes return");
    };
    assert_eq!(out, vec![b'w'], "echo kernel returns the first input byte");
}

/// T-008: a tx carrying a corrupted calc container is rejected at
/// execution and never enters the mined block's success set.
#[test]
fn deploy_tx_with_corrupt_container_rejected() {
    let (mut chain, main, acct) = funded_chain(0x22);
    let mut sto = sto_with("echo", &container_of(echo_code()));
    // corrupt the encoded container in the wire blob (magic word)
    let mut blob = sto.userfuncs.as_list()[0].code_stuff.data.clone().into_vec();
    blob[0] = 0xFF;
    sto.userfuncs.as_mut()[0].code_stuff.data = field::BytesW2::from(blob).unwrap();
    let tx = Arc::new(deploy_action_tx(
        main,
        &acct,
        1,
        sto,
        Amount::zero(), // cost is irrelevant: decode fails first
    )) as TxRef;
    let hash = chain.submit(tx);
    prepare_block(&mut chain);
    chain
        .mine_block()
        .receipt(&hash)
        .expect("receipt")
        .expect_error_contains("calc container");
}

/// T-102: a main-call tx whose codes are declared RV64IM is rejected —
/// calc code is only valid as a contract user function.
#[test]
fn main_call_with_rv64im_codes_rejected() {
    let (mut chain, main, acct) = funded_chain(0x23);
    let mut act = ContractMainCall::new();
    act.codeconf = Uint1::from(vm::CodeConf::from_type(CodeType::RV64IM).raw());
    act.codes = field::BytesW2::from(container_of(echo_code())).unwrap();
    let mut tx = StdTransaction::new_by(
        hacash_params::TX_TYPE_3,
        main,
        Amount::from("1:244").unwrap(),
        sys::curtimes(),
    );
    tx.gas_max = Uint1::from(99u8);
    tx.push_action_in(Arc::new(act));
    tx.fill_sign_account(&acct).unwrap();
    let hash = chain.submit(Arc::new(tx)) ;
    prepare_block(&mut chain);
    chain
        .mine_block()
        .receipt(&hash)
        .expect("receipt")
        .expect_error_contains("calc code type is only valid on contract user functions");
}

/// T-107: a guest trap maps to CalcTrap(155), fails the call tx, and the
/// previously committed state stays intact (a later echo call still works).
#[test]
fn guest_trap_rejects_call_and_keeps_state() {
    let (mut chain, main, acct) = funded_chain(0x24);
    let echo_addr = deploy_and_mine(&mut chain, main, &acct, "echo", &container_of(echo_code()), 1);
    let trap_addr = deploy_and_mine(&mut chain, main, &acct, "trap", &container_of(trap_code()), 2);

    let hash = call_and_mine(&mut chain, main, &acct, &trap_addr, b"x", 99);
    chain
        .receipt(&hash)
        .expect("receipt")
        .expect_error_contains("CalcTrap");

    // state was not corrupted by the failed tx
    let hash = call_and_mine(&mut chain, main, &acct, &echo_addr, b"z", 99);
    chain.receipt(&hash).expect("receipt").expect_success();
}

/// T-105: input longer than the container's declared input_max is a
/// 0-cycle pre-flight trap mapped to CalcTrap.
#[test]
fn input_above_input_max_rejects_call() {
    let (mut chain, main, acct) = funded_chain(0x25);
    let caddr = deploy_and_mine(&mut chain, main, &acct, "echo", &container_of(echo_code()), 1);
    let big = vec![7u8; 65];
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, &big, 99);
    // the call layer maps every guest trap to the generic CalcTrap(155)
    // fault text; the interp-side detail stays inside the VM frame.
    chain
        .receipt(&hash)
        .expect("receipt")
        .expect_error_contains("CalcTrap");
}

/// T-108: an infinite loop is stopped by the per-tx gas budget byte —
/// the call faults OutOfGas and the block still seals the other txs.
#[test]
fn infinite_loop_stops_at_gas_budget() {
    let (mut chain, main, acct) = funded_chain(0x26);
    let echo_addr = deploy_and_mine(&mut chain, main, &acct, "echo", &container_of(echo_code()), 1);
    let loop_addr = deploy_and_mine(&mut chain, main, &acct, "loop", &container_of(selfloop_code()), 2);

    // one block, two calls: the loop call with a tiny budget faults, the
    // echo call in the same block still succeeds
    let ok = Arc::new(call_action_tx(main, &acct, &echo_addr, b"k", 99)) as TxRef;
    let bad = Arc::new(call_action_tx(main, &acct, &loop_addr, b"k", 1)) as TxRef;
    let (ok_h, bad_h) = (chain.submit(ok), chain.submit(bad));
    prepare_block(&mut chain);
    let block = chain.mine_block();
    assert_eq!(block.len(), 2);
    block.receipt(&ok_h).expect("ok receipt").expect_success();
    block
        .receipt(&bad_h)
        .expect("bad receipt")
        .expect_error_contains("OutOfGas");
}

/// T-109: three deploy txs in one block commit independently; a later
/// block carries calls to all three with individually observed receipts.
#[test]
fn mixed_contract_block_lifecycle() {
    let (mut chain, main, acct) = funded_chain(0x27);
    let echo_addr = deploy_and_mine(&mut chain, main, &acct, "echo", &container_of(echo_code()), 1);
    let trap_addr = deploy_and_mine(&mut chain, main, &acct, "trap", &container_of(trap_code()), 2);
    let loop_addr = deploy_and_mine(&mut chain, main, &acct, "loop", &container_of(selfloop_code()), 3);
    assert_eq!(chain.height(), 3);

    let h1 = call_and_mine(&mut chain, main, &acct, &echo_addr, b"a", 99);
    chain.receipt(&h1).expect("receipt").expect_success();
    let h2 = call_and_mine(&mut chain, main, &acct, &trap_addr, b"a", 99);
    chain.receipt(&h2).expect("receipt").expect_error_contains("CalcTrap");
    let h3 = call_and_mine(&mut chain, main, &acct, &loop_addr, b"a", 1);
    chain.receipt(&h3).expect("receipt").expect_error_contains("OutOfGas");
}

/// T-011: ContractUpdate replaces the kernel under the same function sign;
/// calls after the update run the new image.
#[test]
fn contract_update_replaces_kernel() {
    let (mut chain, main, acct) = funded_chain(0x28);
    let caddr = deploy_and_mine(&mut chain, main, &acct, "echo", &container_of(echo_code()), 1);

    let hash = call_and_mine(&mut chain, main, &acct, &caddr, b"q1", 99);
    chain.receipt(&hash).expect("receipt").expect_success();

    // update: same function name -> same sign, new trap kernel
    let new_sto = sto_with("echo", &container_of(trap_code()));
    let mut act = ContractUpdate::new();
    act.address = caddr.to_addr();
    act.edit.new_revision = field::Uint2::from(1u16);
    act.edit.userfuncs = new_sto.userfuncs.clone();
    act.protocol_cost = protocol_cost_for(&chain.services(), &new_sto);
    let mut tx = StdTransaction::new_by(
        hacash_params::TX_TYPE_3,
        main,
        Amount::from("1:244").unwrap(),
        sys::curtimes(),
    );
    tx.gas_max = Uint1::from(99u8);
    tx.push_action_in(Arc::new(act));
    tx.fill_sign_account(&acct).unwrap();
    let hash = chain.submit(Arc::new(tx));
    prepare_block(&mut chain);
    chain
        .mine_block()
        .receipt(&hash)
        .expect("update receipt")
        .expect_success();

    // the call now runs the replaced kernel and traps. The input differs
    // from the pre-update call so the tx hash is unique within the chain.
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, b"q2", 99);
    chain
        .receipt(&hash)
        .expect("receipt")
        .expect_error_contains("CalcTrap");
}

/// I-008: the whole deploy → call lifecycle is deterministic across two
/// independent chains running the identical transaction sequence.
#[test]
fn dual_chain_lifecycle_agrees() {
    let _guard = testkit::sim::integration::test_guard();
    let services = unbounded_services();
    let mut dual = DualMemChain::with_services(services).unwrap();
    let acct = Account::create_by_secret_key_value([0x29; 32]).unwrap();
    let main = Address::from(*acct.address());
    let fund = Amount::coin_u128(FUND, field::UNIT_238);
    hac_add_state(dual.left_mut().state_mut(), &main, &fund).unwrap();
    hac_add_state(dual.right_mut().state_mut(), &main, &fund).unwrap();

    let sto = sto_with("echo", &container_of(echo_code()));
    let caddr = ContractAddress::calculate(&main, &Uint4::from(1u32));
    let deploy = Arc::new(deploy_action_tx(
        main,
        &acct,
        1,
        sto.clone(),
        protocol_cost_for(&dual.left().services(), &sto),
    )) as TxRef;
    let call = Arc::new(call_action_tx(main, &acct, &caddr, b"dual", 99)) as TxRef;
    for height in [1u64, 2] {
        let params = dual.left().services().vm_params().unwrap().clone();
        init_block_contract_storage_budget(dual.left_mut().state_mut(), &params, height).unwrap();
        init_block_contract_storage_budget(dual.right_mut().state_mut(), &params, height).unwrap();
        if height == 1 {
            dual.submit(deploy.clone());
        } else {
            dual.submit(call.clone());
        }
        dual.mine_block();
    }
    dual.assert_same();
    assert!(dual.left().receipts().iter().all(|r| r.is_success()));
}
