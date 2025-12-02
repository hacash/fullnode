

/*
    return gas, val
*/
pub fn setup_vm_run(depth: isize, ctx: &mut dyn Context, ty: u8, mk: u8, cd: &[u8], pm: Value) -> Ret<(i64, Value)> {
    // check tx type
    const TY3: u8 = TransactionType3::TYPE;
    let txty = ctx.env().tx.ty;
    if txty < TY3 {
        return errf!("current transaction type {} too low to setup vm, need at least {}", txty, TY3)
    }
    // setup the vm
    if false == ctx.vm().usable() {
        let vmb = global_machine_manager().assign(ctx.env().block.height);
        ctx.vm_replace(Box::new(vmb));
    }
    // depth
    let old_depth = ctx.depth().clone();
    ctx.depth_set(CallDepth::new(depth));

    let sta = ctx.clone_mut().state();
    let vmi = ctx.clone_mut().vm();
    let ctx = ctx.clone_mut();
    let (cost, rv) = vmi.call(ctx, sta, ty, mk, cd, Box::new(pm))?;
    /*let (cost, rv) = unsafe {
        // ctx
        let ctxptr = ctx as *mut dyn Context;
        let ctxmut1: &mut dyn Context = &mut *ctxptr;
        let ctxmut2: &mut dyn Context = &mut *ctxptr;
        // sta
        let staptr = ctx.state() as *mut dyn State;
        let stamut: &mut dyn State = &mut *staptr;  
        // vmi
        let vmiptr = ctxmut1.vm() as *mut dyn VM;
        let vmimut: &mut dyn VM = &mut *vmiptr;  
        // do call
        vmimut.call(ctxmut2, stamut, ty, mk, cd, Box::new(pm))?
    }; */
    ctx.depth_set(old_depth);
    Ok((cost,  Value::bytes(rv)))
}

















/*

fn _setup_vm_run_by_fn(depth: i8, ctx: &mut dyn Context, execfn: impl FnOnce(&mut dyn Context, &mut dyn VM)->Ret<Vec<u8>>) -> Ret<Value> {
    // check tx type
    const TY3: u8 = TransactionType3::TYPE;
    let txty = ctx.env().tx.ty;
    if txty < TY3 {
        return errf!("current transaction type {} too low to setup vm, need at least {}", txty, TY3)
    }
    // init vm
    if false == ctx.vm().active() {
        let vmb = MACHINE_MANAGER.assign(ctx.env().block.height);
        ctx.vm_replace(Box::new(vmb));
    }
    // depth
    let old_depth = ctx.depth();
    ctx.depth_set(depth);
    let ctxptr = ctx as *mut dyn Context;
    let vmptr = ctx.vm() as *mut dyn VM;
    let rv = unsafe {
        let ctx: &mut dyn Context = &mut *ctxptr;
        let vm: &mut dyn VM = &mut *vmptr;
        execfn( ctx, vm )?
    };
    ctx.depth_set(old_depth);
    Ok(Value::bytes(rv))
}

*/