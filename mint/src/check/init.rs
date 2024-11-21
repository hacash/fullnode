
fn do_initialize(db: &mut dyn State) -> Rerr {
    
	let addr1 = Address::from_readable("12vi7DEZjh6KrK5PVmmqSgvuJPCsZMmpfi").unwrap();
	let addr2 = Address::from_readable("1LsQLqkd8FQDh3R7ZhxC5fndNf92WfhM19").unwrap();
	let addr3 = Address::from_readable("1NUgKsTgM6vQ5nxFHGz1C4METaYTPgiihh").unwrap();
	let amt1 = Amount::small(1, 244);
	let amt2 = Amount::small(12, 244);
    let bls1 = Balance::hac(amt1);
    let bls2 = Balance::hac(amt2);
    let mut state = CoreState::wrap(db);
    state.balance_set(&addr1, &bls2);
    state.balance_set(&addr2, &bls1);
    state.balance_set(&addr3, &bls1);

    // let stateread = CoreStateDisk::wrap(db);

    // ok
    Ok(())
} 
