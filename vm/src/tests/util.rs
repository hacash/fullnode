
use protocol::state::EmptyLogs;
use space::{CtcKVMap, GKVMap, Heap, Stack};

pub type VmTestExecRes = (CallExit, i64, Vec<Value>, Heap);

#[allow(dead_code)]
fn execute_test_maincall(gas: i64, codes: Vec<u8>) -> VmrtRes<VmTestExecRes> {
    execute_test_with_argv(gas, codes, None)
}


#[allow(dead_code)]
fn execute_test_with_argv(gas_limit: i64, codes: Vec<u8>, argv: Option<Value>) -> VmrtRes<VmTestExecRes> {

    let mut pc: usize = 0;
    let mut gas: i64 = gas_limit; // 2000
    // let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
    let cadr = ContractAddress::default();

    let mut statest = StateMem::default();
    let mut sta = VMState::wrap(&mut statest);

    let mut ctx = ExtCallMem::default(); 

    let mut ops = Stack::new(256);
    if let Some(v) = argv {
        ops.push(v).unwrap();
    }

    let mut heap = Heap::new(64);

    // do execute
    super::interpreter::execute_code(
        &mut pc,
        &codes,
        CallMode::Main,
        0,
        &mut gas,
        &GasTable::new(1),
        &GasExtra::new(1),
        &SpaceCap::new(1),
        &mut ops,
        &mut Stack::new(256),
        &mut heap,
        &mut GKVMap::new(20),
        &mut CtcKVMap::new(12),
        &mut ctx,
        &mut EmptyLogs{},
        &mut sta,
        &cadr,
        &cadr,
    ).map(|r|{
        (r, gas_limit - gas, ops.release(), heap)
    })



}
