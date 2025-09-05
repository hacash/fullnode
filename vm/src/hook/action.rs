
pub fn try_action_hook(kid: u16, action: &dyn Any, ctx: &mut dyn Context, _gas: &mut u32) -> Rerr {

    use AbstCall::*;

    match kid {
        HacFromToTrs::KIND
        | HacFromTrs::KIND
        | HacToTrs::KIND
            => coin_asset_transfer_call(PermitHAC, PayableHAC, action, ctx),
        | SatFromToTrs::KIND
        | SatFromTrs::KIND
        | SatToTrs::KIND
            => coin_asset_transfer_call(PermitSAT, PayableSAT, action, ctx),
        | DiaSingleTrs::KIND
        | DiaFromToTrs::KIND
        | DiaFromTrs::KIND
        | DiaToTrs::KIND 
            => coin_asset_transfer_call(PermitHACD, PayableHACD, action, ctx),
        | AssetFromToTrs::KIND
        | AssetFromTrs::KIND
        | AssetToTrs::KIND 
            => coin_asset_transfer_call(PermitAsset, PayableAsset, action, ctx),
        _ => Ok(())
    }

}


fn coin_asset_transfer_call(abstfnf: AbstCall, abstfnt: AbstCall, action: &dyn Any, ctx: &mut dyn Context) -> Rerr {

    let addrs = &ctx.env().tx.addrs;
    let mut from = ctx.env().tx.main;
    let mut to = from.clone();
    let amtargv: Vec<u8>;
    let calldpt: isize = CallDepth::new(1).into();
    let absty = CallMode::Abst as u8;
    
    let asset_param = |asset: &AssetAmt| {
        vec![asset.serial.uint().to_be_bytes(), 
            asset.amount.uint().to_be_bytes()
        ].concat()
    };
    // HAC
    if let Some(act) = action.downcast_ref::<HacToTrs>() {
        to = act.to.real(addrs)?;
        amtargv = act.hacash.serialize();
    }else if let Some(act) = action.downcast_ref::<HacFromTrs>() {
        from = act.from.real(addrs)?;
        amtargv = act.hacash.serialize();
    }else if let Some(act) = action.downcast_ref::<HacFromToTrs>() {
        from = act.from.real(addrs)?;
        to = act.to.real(addrs)?;
        amtargv = act.hacash.serialize();
    // SAT
    }else if let Some(act) = action.downcast_ref::<SatToTrs>() {
        to = act.to.real(addrs)?;
        amtargv = act.satoshi.serialize();
    }else if let Some(act) = action.downcast_ref::<SatFromTrs>() {
        from = act.from.real(addrs)?;
        amtargv = act.satoshi.serialize();
    }else if let Some(act) = action.downcast_ref::<SatFromToTrs>() {
        from = act.from.real(addrs)?;
        to = act.to.real(addrs)?;
        amtargv = act.satoshi.serialize();
    // HACD
    }else if let Some(act) = action.downcast_ref::<DiaSingleTrs>() {
        to = act.to.real(addrs)?;
        amtargv = vec![vec![1], act.diamond.serialize()].concat();
    }else if let Some(act) = action.downcast_ref::<DiaToTrs>() {
        to = act.to.real(addrs)?;
        amtargv = act.diamonds.serialize();
    }else if let Some(act) = action.downcast_ref::<DiaFromTrs>() {
        from = act.from.real(addrs)?;
        amtargv = act.diamonds.serialize();
    }else if let Some(act) = action.downcast_ref::<DiaFromToTrs>() {
        from = act.from.real(addrs)?;
        to = act.to.real(addrs)?;
        amtargv = act.diamonds.serialize();
    // Asset
    }else if let Some(act) = action.downcast_ref::<AssetToTrs>() {
        to = act.to.real(addrs)?;
        amtargv = asset_param(&act.asset);
    }else if let Some(act) = action.downcast_ref::<AssetFromTrs>() {
        from = act.from.real(addrs)?;
        amtargv = asset_param(&act.asset);
    }else if let Some(act) = action.downcast_ref::<AssetFromToTrs>() {
        from = act.from.real(addrs)?;
        to = act.to.real(addrs)?;
        amtargv = asset_param(&act.asset);
    }else {
        unreachable!()
    }

    let (fc, tc) = (from.is_contract(), to.is_contract());
    if !(fc || tc) {
        return Ok(()) // no contract address
    }

    // call from contract
    if fc {
        let param = vec![to.serialize(), amtargv.clone()].concat();
        let (_, rtv) = setup_vm_run(calldpt, ctx, absty, abstfnf as u8, from.as_bytes(), param)?;
        if rtv.is_zero() {
            return errf!("transfer from {} not allow", from.readable())
        }
    }

    // call to contract
    if tc {
        let param = vec![from.serialize(), amtargv].concat();
        let (_, rtv) = setup_vm_run(calldpt, ctx, absty, abstfnt as u8, to.as_bytes(), param)?;
        if rtv.is_zero() {
            return errf!("transfer to {} not allow", to.readable())
        }
    }

    Ok(())
}