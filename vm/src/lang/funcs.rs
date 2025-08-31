

impl Syntax {


    pub fn must_get_func_argv(&mut self) -> Ret<Box<dyn IRNode>> {
        let argvs = self.item_may_block()?.into_vec();
        let argvs = deal_may_func_argvs(argvs);
        Ok(argvs)
    }


    pub fn item_func_call(&mut self, id: String) -> Ret<Box<dyn IRNode>> {
        // ir func
        if let Some((_, inst, pms, args, rs)) = pick_ir_func(&id) {
            let argvs = self.item_may_block()?.into_vec();
            if pms + args != argvs.len() {
                return errf!("ir func call argv length must {} but got {}", 
                    pms + args, argvs.len()
                )
            }
            assert!(rs <= 1);
            return build_ir_func(inst, pms, args, rs, argvs,)
        }

        // native call
        if let Some(id) = pick_native_call(&id) {
            let argvs = self.must_get_func_argv()?;
            return Ok(Box::new(IRNodeParam1Single{
                hrtv: true, inst: Bytecode::NTCALL, para: id, subx: argvs
            }))
        }

        // extend action
        if let Some((argv, inst, para)) = pick_ext_func(&id) {
            let mut argvs = self.item_may_block()?.into_vec();
            let hrtv = true;
            return Ok(match argv {
                false => {
                    if argvs.len() > 0 {
                        return errf!("function '{}' cannot give argv", id)
                    }
                    Box::new(IRNodeParam1{hrtv,inst,para,text:s!("")})
                },
                true => {
                    let argv = match argvs.len() {
                        0 => return errf!("function '{}' must give argv", id),
                        1 => argvs.pop().unwrap(),
                        _ => concat_func_argvs(argvs) // concat all
                    };
                    Box::new(IRNodeParam1Single{hrtv,inst,para,subx: argv})
                },
            })
        }

        // not find
        errf!("cannot find function '{}'", id)
    }

}



fn build_ir_func(inst: Bytecode, pms: usize, args: usize, rs: usize, argvs: Vec<Box<dyn IRNode>>) -> Ret<Box<dyn IRNode>> {
    let mut argvs = VecDeque::from(argvs);
    let hrtv = maybe!(rs==1, true, false);
    let ttv = pms + args;
    if ttv == 0 {
        return Ok(Box::new(IRNodeLeaf{hrtv, inst}))
    }
    macro_rules! avg {() => {
        argvs.pop_front().unwrap()       
    }}
    macro_rules! param { () => {{
        let mut para = -1i16;
        if let Some(n) = avg!().as_any().downcast_ref::<IRNodeParam1>() {
            if n.inst == Bytecode::PU8 {
                para = n.para as i16;
            }
        }
        if para == -1 || para > 255{
            return errf!("ir func call param format error")
        }
        para as u8
    }}}
    if pms == 0 {
        return Ok(match args {
            1 => Box::new(IRNodeSingle{hrtv, inst, subx: avg!()}),
            2 => Box::new(IRNodeDouble{hrtv, inst, subx: avg!(), suby: avg!()}),
            3 => Box::new(IRNodeTriple{hrtv, inst, subx: avg!(), suby: avg!(), subz: avg!()}),
            _ => unreachable!()
        })
    }
    if pms == 1 {
        let para = param!();
        return Ok(match args {
            1 => Box::new(IRNodeParam1Single{hrtv, inst, para, subx: avg!()}),
            _ => unreachable!()
        })
    }
    if pms == 2 {
        let p1 = param!();
        let p2 = param!();
        return Ok(match args {
            1 => Box::new(IRNodeParam2Single{hrtv, inst, para: [p1, p2], subx: avg!()}),
            _ => unreachable!()
        })
    }

    errf!("cannot match ir call type: params({}), args({})", pms, args)
}




/****************************** */



fn pick_ir_func(id: &str) -> Option<(IrFn, Bytecode, usize, usize, usize)> {
    IrFn::from_name(id)
}


fn pick_native_call(id: &str) -> Option<u8> {
    NativeCall::from_name(id).map(|d|d.0) // only id
}

fn deal_may_func_argvs(mut argvs: Vec<Box<dyn IRNode>>) -> Box<dyn IRNode> {
    match argvs.len() {
        0 => Box::new(IRNodeLeaf{hrtv: true, inst: Bytecode::PNBUF}),
        1 => argvs.pop().unwrap(),
        _ => concat_func_argvs(argvs) // concat all
    }
}


fn concat_func_argvs(mut list: Vec<Box<dyn IRNode>>) -> Box<dyn IRNode> {
    // list.reverse();
    let mut res = list.pop().unwrap();
    while let Some(x) = list.pop() {
        res = Box::new(IRNodeDouble{hrtv:true, inst:Bytecode::CAT, subx: x, suby: res});
    }
    res
}




fn pick_ext_func(id: &str) -> Option<(bool, Bytecode, u8)> {
    if let Some(x) = CALL_EXTEND_ENV_DEFS.iter().position(|f|*f==id) {
        if x > 255 { unreachable!() }
        if x > 0 {
            return Some((false, Bytecode::EXTENV, x as u8))
        }
    }
    if let Some(x) = CALL_EXTEND_FUNC_DEFS.iter().position(|f|*f==id) {
        if x > 255 { unreachable!() }
        if x > 0 {
            return Some((true, Bytecode::EXTFUNC, x as u8))
        }
    }
    None
}






