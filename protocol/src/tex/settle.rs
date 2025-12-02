// use crate::operate::hacd_transfer;



pub fn do_settlement(ctx: &mut dyn Context) -> Rerr {
    // check all settle result
    let t = ctx.clone_mut().tex_state();
    if t.zhu != 0 || t.sat != 0 || t.dia != 0 {
        return errf!("coin settlement check failed")
    }
    for (a, v) in t.assets.iter() {
        if *v != 0 {
            return errf!("asset <{}> settlement check failed", a.uint())
        }
    }
    // settle diamonds
    for (adr, dn) in &t.diatrs {
        let dialist = DiamondNameListMax200::from_list(t.diamonds.fetch_list(*dn)?)?;
        do_diamonds_transfer(&dialist, &SETTLEMENT_ADDR, adr, ctx.clone_mut())?;
    }
    // check
    if t.diamonds.length() > 0 {
        return errf!("diamonds settlement check failed")
    }
    Ok(())
}