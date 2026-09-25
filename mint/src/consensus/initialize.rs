use base::{CoreState, StateLayer};
use field::{Address, Amount, Balance};
use sys::Rerr;

/// Total HAC seeded by [`initialize`], expressed in 238 units (10^-10 HAC).
/// Must mirror the `Balance::hac` amounts below: unit 244 = 10^-4 HAC, so
/// mantissa M at unit 244 is M x 10^6 in 238 units. This one-time genesis
/// allocation is NOT counted by the `/query/supply` issuance formula, so a
/// full-ledger audit will find exactly this much extra "held" HAC.
pub const GENESIS_INIT_TOTAL_238: u128 =
    12u128 * 1_000_000 + 1u128 * 1_000_000 + 1u128 * 1_000_000 + 549u128 * 1_000_000 + 527u128 * 1_000_000;

pub fn initialize(layer: &mut dyn StateLayer, diamond_form: bool) -> Rerr {
    let addr1 = Address::from_readable("12vi7DEZjh6KrK5PVmmqSgvuJPCsZMmpfi").unwrap();
    let addr2 = Address::from_readable("1LsQLqkd8FQDh3R7ZhxC5fndNf92WfhM19").unwrap();
    let addr3 = Address::from_readable("1NUgKsTgM6vQ5nxFHGz1C4METaYTPgiihh").unwrap();
    let addr4 = Address::from_readable("1HVMPyUt3ZR3JCyGA5p2ptCvusZsiX6YV9").unwrap();
    let addr5 = Address::from_readable("1FSse2degBjVAAiiMzC36t6NgzjeUkxopG").unwrap();

    let bls1 = Balance::hac(Amount::small(1, 244));
    let bls2 = Balance::hac(Amount::small(12, 244));
    let bls3 = Balance::hac(Amount::coin(549, 244));
    let bls4 = Balance::hac(Amount::coin(527, 244));

    let mut state = CoreState::wrap(layer);
    state.balance_set(&addr1, &bls2);
    state.balance_set(&addr2, &bls1);
    state.balance_set(&addr3, &bls1);
    state.balance_set(&addr4, &bls3);
    state.balance_set(&addr5, &bls4);
    layer.set(crate::DIAMOND_FORM_STATE_KEY, vec![diamond_form as u8]);
    Ok(())
}
