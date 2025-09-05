
use space::{CtcKVMap, GKVMap, Heap, Stack};

#[allow(dead_code)]
fn execute_test_maincall(gas: i64, codes: Vec<u8>) -> VmrtRes<(i64, Vec<Value>, CallExit)> {

    let mut pc: usize = 0;
    let mut gas_limit: i64 = gas; // 2000
    // let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
    let cadr = ContractAddress::default();

    let mut statest = StateTest::default();
    let mut sta = VMState::wrap(&mut statest);

    let mut ctx = ExtCalTest::default(); 

    let mut ops = Stack::new(256);
    // do execute
    super::interpreter::execute_code(
        &mut pc,
        &codes,
        CallMode::Main,
        0,
        &mut gas_limit,
        &GasTable::new(1),
        &GasExtra::new(1),
        &SpaceCap::new(1),
        &mut ops,
        &mut Stack::new(256),
        &mut Heap::new(64),
        &mut GKVMap::new(20),
        &mut CtcKVMap::new(12),
        &mut ctx,
        &mut sta,
        &cadr,
        &cadr,
    ).map(|r|{
        (gas - gas_limit, ops.release(), r)
    })

}