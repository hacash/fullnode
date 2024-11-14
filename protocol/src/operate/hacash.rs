use crate::state::CoreState;


pub fn hac_transfer(_env: &Env, sta: &mut dyn State, _from: &Address, _to: &Address, _hacash: &Amount) -> Ret<Vec<u8>> {
    let mut state = CoreState::wrap(sta);
    let _bls = state.balance(&Address::DEFAULT)?;
    state.balance_set(&Address::DEFAULT, &Uint8::from(2))?;
    errf!("")
}

