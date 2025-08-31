
pub fn try_action_hook(kid: u16, action: &dyn Any, ctx: &mut dyn Context, _gas: &mut u32) -> Rerr {

    // hac transfer
    if [1,13,14].contains(&kid) {

        let addrs = &ctx.env().tx.addrs;
        let mut from = ctx.env().tx.main;
        let mut to = from.clone();
        let amt;
        if let Some(act) = action.downcast_ref::<HacToTrs>() {
            to = act.to.real(addrs)?;
            amt = act.hacash.clone();
        }else if let Some(act) = action.downcast_ref::<HacFromTrs>() {
            from = act.from.real(addrs)?;
            amt = act.hacash.clone();
        }else if let Some(act) = action.downcast_ref::<HacFromToTrs>() {
            from = act.from.real(addrs)?;
            to = act.to.real(addrs)?;
            amt = act.hacash.clone();
        }else {
            return errf!("action kind {} hook call error", kid)
        }
        let isctl = from.is_contract() || to.is_contract();
        if ! isctl {
            return Ok(()) // no contract address
        }
        // do call
        let syscty = CallTy::Abst as u8;
        let amtbts = amt.serialize();
        if from.is_contract() {
            let kid = AbstCall::PermitHAC as u8;
            let param = vec![to.serialize(), amtbts.clone()].concat();
            let depth = 1; // abst call depth start from 1
            let vm_res = setup_vm_run(depth, ctx, syscty, kid, from.as_bytes(), param)?;
            if vm_res.is_zero() {
                return errf!("hac transfer from {} not allow", from.readable())
            }
        }
        if to.is_contract() {
            let kid = AbstCall::PayableHAC as u8;
            let param = vec![from.serialize(), amtbts].concat();
            let depth = 1; // abst call depth start from 1
            let vm_res = setup_vm_run(depth, ctx, syscty, kid, to.as_bytes(), param)?;
            if vm_res.is_zero() {
                return errf!("hac transfer to {} not allow", to.readable())
            }
        }
        return Ok(())
    }

    Ok(())
}