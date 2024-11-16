
pub fn hac_transfer(ctx: &mut dyn Context, _from: &Address, _to: &Address, _hacash: &Amount) -> Ret<Vec<u8>> {
    let mut state = CoreState::wrap(ctx.state());
    let _bls = state.balance(&Address::DEFAULT)?;
    state.balance_set(&Address::DEFAULT, &Uint8::from(2))?;
    errf!("")
}

