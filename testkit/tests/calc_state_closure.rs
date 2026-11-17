//! calc result → storage state closure: end-to-end assertions that a calc
//! call result goes through a real contract into storage, gets confirmed by a
//! block, and reads back (`calc_tx_lifecycle` only observes the return
//! value/trip faces; this file adds the state face).
//!
//! The closure shape follows `calc_tx_lifecycle`: real signed type-3
//! transactions carrying `ContractDeploy`(40) / `ContractMainCall`(44) /
//! `ContractUpdate`(41), submit → `mine_block` → receipt; storage primitives
//! use fitsh's existing forms (`storage_stat` / `storage_new` /
//! `storage_edit` / `storage_rent` / `storage_load`, isomorphic to the
//! three-branch ledger writes in `doc/hacdsdefi4000.loc`). Read-back goes
//! through the sandbox query path (`TestContext` + `chain.state()` snapshots);
//! what is asserted is the committed state, not the call return value.
//!
//! Four closures:
//! 1. Write-read closure: rv64im kernel (arithmetic/bytes transforms) output
//!    → fitsh writes to storage → block → a fresh query reads back
//!    byte-identical (normal values / empty bytes / the 1280 boundary / wide
//!    u128 values).
//! 2. Trap isolation: kernel divide-by-zero fault 90 → block → storage
//!    unpolluted and the contract stays callable; the receipt failure face
//!    matches the CalcTrap pinned by `calc_tx_lifecycle` T-107.
//! 3. HVM isomorphism check: the same inputs and key slots run through a pure
//!    HVM implementation; both closures' storage end states agree (the anchor
//!    that the executor path does not affect state semantics).
//! 4. State continuity across update: after swapping the container/HVM source,
//!    old storage stays readable and the new logic keeps working on the old
//!    state.
//!
//! Run: cargo test -p testkit --test calc_state_closure

use std::sync::Arc;

use app::standard_registry_with_params;
use base::{hac_add_state, init_block_contract_storage_budget, ExecutionServices, TxRef};
use field::{Address, Amount, Uint1, Uint2, Uint4};
use protocol::tx_std::StdTransaction;
use sys::Account;
use testkit::sim::context::TestContext;
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
fn enc_b(imm: i32, rs2: u32, rs1: u32, f3: u32) -> [u8; 4] {
    // B-type branch (bne): imm[12|10:5] in [31:25], imm[4:1|11] in [11:7].
    let u = imm as u32;
    let w = (((u >> 12) & 1) << 31)
        | (((u >> 5) & 0x3F) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | (((u >> 1) & 0xF) << 8)
        | (((u >> 11) & 1) << 7)
        | 0x63;
    w.to_le_bytes()
}

/// Empty-output kernel: HALTs with output length 0 (a0 = exit code 0).
fn empty_code() -> Vec<u8> {
    enc_i(0, 0, 0, 10, 0x13) // addi a0, x0, 0
        .into_iter()
        .chain(enc_i(0, 0, 0, 11, 0x13)) // addi a1, x0, 0
        .chain(enc_i(0, 0, 0, 17, 0x13)) // addi a7, x0, 0 (HALT)
        .chain([0x73, 0, 0, 0]) // ecall
        .collect()
}

/// Fill kernel: writes 1280 bytes of 0xAB to the output region via a
/// countdown store loop, then HALTs with output length 1280 (the
/// `SpaceCap::value_size` boundary).
fn fill1280_code() -> Vec<u8> {
    enc_i(171, 0, 0, 5, 0x13) // addi t0, x0, 171   (byte 0xAB)
        .into_iter()
        .chain(enc_i(1280, 0, 0, 6, 0x13)) // addi t1, x0, 1280 (count)
        // loop: (branch below targets this byte offset)
        .chain(enc_s(0, 5, 12, 0, 0x23)) // sb t0, 0(a2)
        .chain(enc_i(1, 12, 0, 12, 0x13)) // addi a2, a2, 1
        .chain(enc_i(-1, 6, 0, 6, 0x13)) // addi t1, t1, -1
        .chain(enc_b(-12, 0, 6, 1)) // bne t1, x0, loop  (target idx2 = pc - 12)
        .chain(enc_i(0, 0, 0, 10, 0x13)) // addi a0, x0, 0
        .chain(enc_i(1280, 0, 0, 11, 0x13)) // addi a1, x0, 1280
        .chain(enc_i(0, 0, 0, 17, 0x13)) // addi a7, x0, 0
        .chain([0x73, 0, 0, 0]) // ecall
        .collect()
}

fn container_with(code: Vec<u8>, output_max: u16) -> Vec<u8> {
    // mem window must cover arena base (0x5000-0x1000 = 16 KiB above
    // CODE_BASE) + both IO regions + stack guard/size + a KiB of arena.
    pack_code(
        &SpaceCap::new(0),
        CALC_PROFILE_RV64IM,
        32,
        0x1000,
        64,
        output_max,
        code,
    )
    .unwrap()
    .encode()
    .unwrap()
}

// ------------------------------------------------------- fitsh sources ---

/// The calc-path contract: rv64im kernels produce the value (body-mode fitsh
/// kernel subset lowering + two raw containers injected via `external_rv64im`),
/// entries write it into contract storage with the canonical three-branch
/// ledger write (stat → new / edit / rent+edit), readbacks observe storage.
fn calc_contract_src() -> String {
    r#"
pragma fitsh 1.0.0
contract StateClosure {
    function rv64im empty(data: bytes)
    function rv64im echo(data: bytes) -> bytes {
        var b = byte(data, 0)
        return b as bytes
    }
    function rv64im wide(data: bytes) -> bytes {
        var b = byte(data, 0)
        var v = (b as u128) << 120
        return v as bytes
    }
    function rv64im divz(data: bytes) -> bytes {
        var b = byte(data, 0)
        var z = b - b
        var q = b / z
        return q as bytes
    }
    function rv64im fill1280(data: bytes)
    abstract Change() -> u64 {
        return 0
    }
    abstract Append() -> u64 {
        return 0
    }

    // three-branch writes (isomorphic to the DeFi ledger wr): nil → new /
    // live → edit / recover → rent+edit. Rent term 80 (x100 blocks/term):
    // storage_new bills unit*period; within the per-tx gas budget cap of
    // 111,911 (gas_max byte 99 ceiling) only <=84 terms fit.
    function wr(key: bytes, val: bytes) -> u32 {
        var st = storage_stat(key)
        if st is nil {
            storage_new(key, val, 80)
            return 0
        }
        var live = item_get(st, 0)
        if live > 0 {
            storage_edit(key, val)
            return 0
        }
        storage_rent(key, 80)
        storage_edit(key, val)
        return 0
    }
    function wr_wide(key: bytes, val: u128) -> u32 {
        var st = storage_stat(key)
        if st is nil {
            storage_new(key, val, 80)
            return 0
        }
        var live = item_get(st, 0)
        if live > 0 {
            storage_edit(key, val)
            return 0
        }
        storage_rent(key, 80)
        storage_edit(key, val)
        return 0
    }

    // calc path entries: kernel output → storage
    function external entry_normal(data: bytes) -> u32 {
        var out = self::echo(data)
        self.wr("kv:normal", out)
        return 0
    }
    function external entry_empty(data: bytes) -> u32 {
        var out = self::empty(data)
        self.wr("kv:empty", out)
        return 0
    }
    function external entry_fill(data: bytes) -> u32 {
        var out = self::fill1280(data)
        self.wr("kv:fill1280", out)
        return 0
    }
    function external entry_wide(data: bytes) -> u32 {
        var out = self::wide(data)
        var v = out as u128
        self.wr_wide("kv:wide", v)
        return 0
    }
    function external entry_trap(data: bytes) -> u32 {
        var out = self::divz(data)
        self.wr("kv:trap", out)
        return 0
    }

    // readbacks (only via the sandbox query path; observes committed state)
    function external read_bytes(key: bytes) -> bytes {
        var v = storage_load(key)
        if v is nil { return "" }
        return v
    }
    function external read_is_nil(key: bytes) -> u32 {
        var v = storage_load(key)
        if v is nil { return 1 }
        return 0
    }
    function external read_u128(key: bytes) -> u128 {
        var v = storage_load(key)
        if v is nil { return 0 }
        return v
    }
    function external read_live(key: bytes) -> u64 {
        var st = storage_stat(key)
        if st is nil { return 0 }
        return item_get(st, 0)
    }
}
"#
    .to_string()
}

/// The pure-HVM contract: same keys, same write/read shape, but the value is
/// computed in HVM (no calc kernel involved). Same-input → same terminal
/// storage state with the calc path is the executor-path anchor.
fn hvm_contract_src() -> String {
    r#"
pragma fitsh 1.0.0
contract StateClosure {
    function wr(key: bytes, val: bytes) -> u32 {
        var st = storage_stat(key)
        if st is nil {
            storage_new(key, val, 80)
            return 0
        }
        var live = item_get(st, 0)
        if live > 0 {
            storage_edit(key, val)
            return 0
        }
        storage_rent(key, 80)
        storage_edit(key, val)
        return 0
    }
    function wr_wide(key: bytes, val: u128) -> u32 {
        var st = storage_stat(key)
        if st is nil {
            storage_new(key, val, 80)
            return 0
        }
        var live = item_get(st, 0)
        if live > 0 {
            storage_edit(key, val)
            return 0
        }
        storage_rent(key, 80)
        storage_edit(key, val)
        return 0
    }

    // pure HVM entries: value computed without calc
    function external entry_normal(data: bytes) -> u32 {
        var out = byte(data, 0) as bytes
        self.wr("kv:normal", out)
        return 0
    }
    function external entry_empty(data: bytes) -> u32 {
        self.wr("kv:empty", "")
        return 0
    }
    function external entry_fill(data: bytes) -> u32 {
        // 1280 x 0xAB via doubling (linear CAT appends are quadratic and would blow the per-tx gas cap)
        var a = "\xab\xab\xab\xab\xab\xab\xab\xab"
        a = a ++ a
        a = a ++ a
        a = a ++ a
        a = a ++ a
        var b = a ++ a
        a = b ++ a
        a = a ++ a
        a = a ++ b
        var out = a ++ b
        self.wr("kv:fill1280", out)
        return 0
    }
    function external entry_wide(data: bytes) -> u32 {
        var v = (byte(data, 0) as u128) << 120
        self.wr_wide("kv:wide", v)
        return 0
    }

    function external read_bytes(key: bytes) -> bytes {
        var v = storage_load(key)
        if v is nil { return "" }
        return v
    }
    function external read_is_nil(key: bytes) -> u32 {
        var v = storage_load(key)
        if v is nil { return 1 }
        return 0
    }
    function external read_u128(key: bytes) -> u128 {
        var v = storage_load(key)
        if v is nil { return 0 }
        return v
    }
    function external read_live(key: bytes) -> u64 {
        var st = storage_stat(key)
        if st is nil { return 0 }
        return item_get(st, 0)
    }
}
"#
    .to_string()
}

/// Post-update source for the update-continuity closure: the `echo` kernel is
/// replaced (same sign, now the div0 body — container swap) and the HVM body
/// gains `entry_after`, which reads the OLD `kv:normal` value and derives a
/// new key from it (new logic continues on old state).
fn updated_contract_src() -> String {
    r#"
pragma fitsh 1.0.0
contract StateClosure {
    function rv64im empty(data: bytes)
    function rv64im echo(data: bytes) -> bytes {
        var b = byte(data, 0)
        var z = b - b
        var q = b / z
        return q as bytes
    }
    function rv64im wide(data: bytes) -> bytes {
        var b = byte(data, 0)
        var v = (b as u128) << 120
        return v as bytes
    }
    function rv64im divz(data: bytes) -> bytes {
        var b = byte(data, 0)
        var z = b - b
        var q = b / z
        return q as bytes
    }
    function rv64im fill1280(data: bytes)
    abstract Change() -> u64 {
        return 0
    }
    abstract Append() -> u64 {
        return 0
    }

    function wr(key: bytes, val: bytes) -> u32 {
        var st = storage_stat(key)
        if st is nil {
            storage_new(key, val, 80)
            return 0
        }
        var live = item_get(st, 0)
        if live > 0 {
            storage_edit(key, val)
            return 0
        }
        storage_rent(key, 80)
        storage_edit(key, val)
        return 0
    }
    function wr_wide(key: bytes, val: u128) -> u32 {
        var st = storage_stat(key)
        if st is nil {
            storage_new(key, val, 80)
            return 0
        }
        var live = item_get(st, 0)
        if live > 0 {
            storage_edit(key, val)
            return 0
        }
        storage_rent(key, 80)
        storage_edit(key, val)
        return 0
    }

    function external entry_normal(data: bytes) -> u32 {
        var out = self::echo(data)
        self.wr("kv:normal", out)
        return 0
    }
    function external entry_empty(data: bytes) -> u32 {
        var out = self::empty(data)
        self.wr("kv:empty", out)
        return 0
    }
    function external entry_fill(data: bytes) -> u32 {
        var out = self::fill1280(data)
        self.wr("kv:fill1280", out)
        return 0
    }
    function external entry_wide(data: bytes) -> u32 {
        var out = self::wide(data)
        var v = out as u128
        self.wr_wide("kv:wide", v)
        return 0
    }
    function external entry_trap(data: bytes) -> u32 {
        var out = self::divz(data)
        self.wr("kv:trap", out)
        return 0
    }

    // new HVM logic: derives a fresh key from the pre-update storage value
    function external entry_after(data: bytes) -> u32 {
        var old = storage_load("kv:normal")
        var out = old ++ data
        self.wr("kv:after", out)
        return 0
    }

    function external read_bytes(key: bytes) -> bytes {
        var v = storage_load(key)
        if v is nil { return "" }
        return v
    }
    function external read_is_nil(key: bytes) -> u32 {
        var v = storage_load(key)
        if v is nil { return 1 }
        return 0
    }
    function external read_u128(key: bytes) -> u128 {
        var v = storage_load(key)
        if v is nil { return 0 }
        return v
    }
    function external read_live(key: bytes) -> u64 {
        var st = storage_stat(key)
        if st is nil { return 0 }
        return item_get(st, 0)
    }
}
"#
    .to_string()
}

fn sto_with(src: &str, external: &[(&str, Vec<u8>)]) -> vm::contract::ContractSto {
    let (contract, _, _, _) = compiler::compile_with_rv64im(src, external)
        .expect("fitsh compiles with the rv64im kernels");
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

/// Serialize every test: the VM keeps a process-wide contract object cache
/// keyed by address, so parallel tests must not reuse addresses across chains
/// while one is mid-update.
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
    func: &str,
    input: &[u8],
    gas_max: u8,
) -> StdTransaction {
    // The main-call entry treats a non-zero-bytes return value as a revert
    // payload (entry.rs::check_vm_return_value), so the bytecode discards the
    // entry return and pushes Nil; the storage effect is observed through the
    // committed-state readback path.
    let mut codes = build_call_codes(func, &[Value::Bytes(input.to_vec())]).expect("call codes");
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

fn deploy_and_mine(
    chain: &mut MemChain,
    main: Address,
    acct: &Account,
    sto: vm::contract::ContractSto,
    nonce: u32,
) -> ContractAddress {
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

/// Submit a main-call tx to `func(data)`, mine a block, return the tx hash.
fn call_and_mine(
    chain: &mut MemChain,
    main: Address,
    acct: &Account,
    caddr: &ContractAddress,
    func: &str,
    input: &[u8],
    gas_max: u8,
) -> field::Hash {
    let tx = Arc::new(call_action_tx(main, acct, caddr, func, input, gas_max)) as TxRef;
    let hash = chain.submit(tx);
    prepare_block(chain);
    let block = chain.mine_block();
    if let Some(r) = block.receipt(&hash) {
        println!("[gas] {func} gas_max_byte={gas_max} success={} gas_remaining={}", r.is_success(), r.gas_remaining);
    }
    hash
}

/// Sandbox query against the committed chain state (a state clone, never the
/// pending layer): returns the raw Value the contract function returns.
fn sandbox_call(
    chain: &MemChain,
    main: Address,
    caddr: &ContractAddress,
    func: &str,
    args: &[Value],
) -> Value {
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
    let codes = build_call_codes(func, args).expect("sandbox codes");
    let (_, out) = main_call(&mut ctx, CodeType::Bytecode, codes.into()).expect("sandbox call");
    out
}

fn read_bytes_of(chain: &MemChain, main: Address, caddr: &ContractAddress, key: &str) -> Vec<u8> {
    let v = sandbox_call(
        chain,
        main,
        caddr,
        "read_bytes",
        &[Value::Bytes(key.as_bytes().to_vec())],
    );
    match v {
        Value::Bytes(b) => b,
        other => panic!("read_bytes({key}) returned non-bytes {other:?}"),
    }
}

fn read_is_nil(chain: &MemChain, main: Address, caddr: &ContractAddress, key: &str) -> u64 {
    let v = sandbox_call(
        chain,
        main,
        caddr,
        "read_is_nil",
        &[Value::Bytes(key.as_bytes().to_vec())],
    );
    v.extract_u128().expect("read_is_nil returns uint") as u64
}

fn read_u128_of(chain: &MemChain, main: Address, caddr: &ContractAddress, key: &str) -> u128 {
    let v = sandbox_call(
        chain,
        main,
        caddr,
        "read_u128",
        &[Value::Bytes(key.as_bytes().to_vec())],
    );
    v.extract_u128().expect("read_u128 returns uint")
}

fn read_live(chain: &MemChain, main: Address, caddr: &ContractAddress, key: &str) -> u64 {
    let v = sandbox_call(
        chain,
        main,
        caddr,
        "read_live",
        &[Value::Bytes(key.as_bytes().to_vec())],
    );
    v.extract_u128().expect("read_live returns uint") as u64
}

// ================================================================= tests

/// Closure 1 — write-read: calc kernel output goes through a real call into
/// storage; after block confirmation it reads back byte-identical from the
/// committed state. Covers: normal values, empty bytes, the 1280-boundary
/// output (value_size cap), and wide u128 values.
#[test]
fn calc_result_writes_storage_and_reads_back() {
    let (mut chain, main, acct) = funded_chain(0x41);
    let sto = sto_with(
        &calc_contract_src(),
        &[
            ("empty", container_with(empty_code(), 64)),
            ("fill1280", container_with(fill1280_code(), 1280)),
        ],
    );
    let caddr = deploy_and_mine(&mut chain, main, &acct, sto, 1);

    // -- normal value: the echo kernel echoes the first byte --
    let input = b"K";
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_normal", input, 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    let got = read_bytes_of(&chain, main, &caddr, "kv:normal");
    assert_eq!(got, vec![b'K'], "normal value written by the calc call");
    assert!(read_live(&chain, main, &caddr, "kv:normal") > 0, "key live");
    println!("[closure] kv:normal = {got:?} (input {input:?})");

    // -- empty bytes: a 0-length kernel output written to storage --
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_empty", b"K", 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    let got = read_bytes_of(&chain, main, &caddr, "kv:empty");
    let is_nil = read_is_nil(&chain, main, &caddr, "kv:empty");
    assert!(got.is_empty(), "empty value reads back empty, got {got:?}");
    assert_eq!(
        is_nil, 0,
        "stored empty bytes is a present (non-Nil) storage value"
    );
    println!("[closure] kv:empty = {:?} (is_nil={is_nil})", got);

    // -- 1280-boundary output: the fill kernel writes value_size full --
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_fill", b"K", 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    let got = read_bytes_of(&chain, main, &caddr, "kv:fill1280");
    assert_eq!(got, vec![0xABu8; 1280], "1280-byte boundary value round-trips");
    println!("[closure] kv:fill1280 = {} bytes of 0xAB", got.len());

    // -- wide u128 value: kernel 16-byte BE output → fitsh cast u128 → storage --
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_wide", input, 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    let got = read_u128_of(&chain, main, &caddr, "kv:wide");
    assert_eq!(got, 0x4Bu128 << 120, "u128-wide value survives the storage round-trip");
    println!("[closure] kv:wide = {got:#x}");
}

/// Closure 2 — trap isolation: a call fails on the calc kernel's
/// divide-by-zero fault 90 (CalcTrap, the same failure face pinned by
/// `calc_tx_lifecycle` T-107); after the block, storage is unpolluted by this
/// call and the contract body itself stays callable/writable.
#[test]
fn calc_trap_leaves_storage_untouched() {
    let (mut chain, main, acct) = funded_chain(0x42);
    let sto = sto_with(
        &calc_contract_src(),
        &[
            ("empty", container_with(empty_code(), 64)),
            ("fill1280", container_with(fill1280_code(), 1280)),
        ],
    );
    let caddr = deploy_and_mine(&mut chain, main, &acct, sto, 1);

    // write a normal value first to establish the protected state baseline
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_normal", b"Z", 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    assert_eq!(read_bytes_of(&chain, main, &caddr, "kv:normal"), vec![b'Z']);

    // trap call: divide-by-zero fault 90 → failure face matches T-107 (CalcTrap)
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_trap", b"T", 255);
    let receipt = chain.receipt(&hash).expect("receipt");
    receipt.expect_error_contains("CalcTrap");
    if let testkit::sim::memchain::TxOutput::Error(msg) = &receipt.output {
        println!("[trap] receipt error = {msg}");
    }
    // failed commit: this call's write (entry_trap's kv:trap) does not exist
    assert_eq!(read_is_nil(&chain, main, &caddr, "kv:trap"), 1, "trap call wrote nothing");
    assert_eq!(read_live(&chain, main, &caddr, "kv:trap"), 0);
    // the protected old state is unpolluted
    assert_eq!(read_bytes_of(&chain, main, &caddr, "kv:normal"), vec![b'Z'], "baseline value intact");

    // the contract body stays callable (the edit branch on an existing key must work too)
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_normal", b"W", 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    assert_eq!(
        read_bytes_of(&chain, main, &caddr, "kv:normal"),
        vec![b'W'],
        "contract stays writable after the trap"
    );
}

/// Closure 3 — HVM isomorphism check: same inputs, same key slots, a pure HVM
/// implementation (no calc) runs the same write-read closure; both closures'
/// storage end states agree key by key.
#[test]
fn hvm_pure_path_same_terminal_state() {
    let (mut chain, main, acct) = funded_chain(0x43);
    let calc_sto = sto_with(
        &calc_contract_src(),
        &[
            ("empty", container_with(empty_code(), 64)),
            ("fill1280", container_with(fill1280_code(), 1280)),
        ],
    );
    let hvm_sto = sto_with(&hvm_contract_src(), &[]);
    let calc_addr = deploy_and_mine(&mut chain, main, &acct, calc_sto, 1);
    let hvm_addr = deploy_and_mine(&mut chain, main, &acct, hvm_sto, 2);

    // run all four write paths with the same inputs (same block, one tx per path)
    let input = b"K";
    let entries = ["entry_normal", "entry_empty", "entry_fill", "entry_wide"];
    let mut hashes = Vec::new();
    for func in entries {
        hashes.push((
            func,
            chain.submit(Arc::new(call_action_tx(
                main, &acct, &calc_addr, func, input, 255,
            ))) as field::Hash,
            chain.submit(Arc::new(call_action_tx(
                main, &acct, &hvm_addr, func, input, 255,
            ))) as field::Hash,
        ));
    }
    prepare_block(&mut chain);
    let block = chain.mine_block();
    assert_eq!(block.len(), 8, "8 calls in one block");
    for (func, calc_h, hvm_h) in &hashes {
        block.receipt(calc_h).expect("calc receipt").expect_success();
        block.receipt(hvm_h).expect("hvm receipt").expect_success();
        println!("[isomorph] {func}: both paths sealed in one block");
    }

    // end states agree key by key
    for key in ["kv:normal", "kv:empty", "kv:fill1280"] {
        let a = read_bytes_of(&chain, main, &calc_addr, key);
        let b = read_bytes_of(&chain, main, &hvm_addr, key);
        assert_eq!(a, b, "terminal state of {key} identical across executor paths");
        println!("[isomorph] {key}: {} bytes, both paths equal", a.len());
    }
    assert_eq!(read_bytes_of(&chain, main, &calc_addr, "kv:normal"), vec![b'K']);
    assert_eq!(read_bytes_of(&chain, main, &calc_addr, "kv:fill1280"), vec![0xABu8; 1280]);
    let (wa, wb) = (
        read_u128_of(&chain, main, &calc_addr, "kv:wide"),
        read_u128_of(&chain, main, &hvm_addr, "kv:wide"),
    );
    assert_eq!(wa, wb, "u128 terminal value identical");
    assert_eq!(wa, 0x4Bu128 << 120);
    for key in ["kv:normal", "kv:empty", "kv:fill1280", "kv:wide"] {
        assert_eq!(
            read_is_nil(&chain, main, &calc_addr, key),
            read_is_nil(&chain, main, &hvm_addr, key),
            "nil-ness of {key} identical"
        );
        assert!(read_live(&chain, main, &calc_addr, key) > 0);
        assert!(read_live(&chain, main, &hvm_addr, key) > 0);
    }
}

/// Closure 4 — state continuity across update: ContractUpdate swaps the
/// container (the echo kernel becomes a divide-by-zero body) and the HVM
/// source (adds entry_after, deriving a new key from the old state). Old
/// storage stays readable, the new logic keeps working on the old state, and
/// the swapped kernel's isolation semantics are unchanged.
#[test]
fn update_keeps_state_and_new_logic_continues() {
    let (mut chain, main, acct) = funded_chain(0x44);
    let sto = sto_with(
        &calc_contract_src(),
        &[
            ("empty", container_with(empty_code(), 64)),
            ("fill1280", container_with(fill1280_code(), 1280)),
        ],
    );
    let caddr = deploy_and_mine(&mut chain, main, &acct, sto, 1);

    // writes before the update
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_normal", b"Q", 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    assert_eq!(read_bytes_of(&chain, main, &caddr, "kv:normal"), vec![b'Q']);

    // ContractUpdate: full userfuncs replacement (same sign, new container + new HVM function)
    let new_sto = sto_with(
        &updated_contract_src(),
        &[
            ("empty", container_with(empty_code(), 64)),
            ("fill1280", container_with(fill1280_code(), 1280)),
        ],
    );
    let mut act = ContractUpdate::new();
    act.address = caddr.to_addr();
    act.edit.new_revision = Uint2::from(1u16);
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

    // old storage stays readable (not lost with the update)
    assert_eq!(
        read_bytes_of(&chain, main, &caddr, "kv:normal"),
        vec![b'Q'],
        "pre-update storage survives the update"
    );

    // new HVM logic keeps working on the old state: kv:after = old kv:normal ++ this input
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_after", b"NEXT", 255);
    chain.receipt(&hash).expect("receipt").expect_success();
    assert_eq!(
        read_bytes_of(&chain, main, &caddr, "kv:after"),
        [b'Q'].into_iter().chain(b"NEXT".iter().copied()).collect::<Vec<u8>>(),
        "new logic derives from the old storage state"
    );

    // isolation semantics unchanged after the container swap: the echo kernel is now a divide-by-zero body, the call fails and pollutes nothing
    let hash = call_and_mine(&mut chain, main, &acct, &caddr, "entry_normal", b"R", 255);
    chain
        .receipt(&hash)
        .expect("receipt")
        .expect_error_contains("CalcTrap");
    assert_eq!(
        read_bytes_of(&chain, main, &caddr, "kv:normal"),
        vec![b'Q'],
        "post-update trap also leaves storage untouched"
    );
}
