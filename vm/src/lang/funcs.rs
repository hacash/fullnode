

impl Syntax {

    pub fn item_get(&mut self, id: String) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        let Some(_) = self.locals.get(&id) else {
            return errf!("cannot find '{}' object in item get", id)
        };
        let k = self.item_must(1)?;  // over [
        let Partition(']') = self.next()? else {
            return errf!("item get statement format error")
        };
        let obj = self.link_local(&id)?;
        let nd = IRNodeDouble{hrtv: true, inst: ITEMGET, subx: obj, suby: k};
        Ok(Box::new(nd))
    }


    pub fn must_get_func_argv(&mut self, md: ArgvMode) -> Ret<(usize, Box<dyn IRNode>)> {
        // use Bytecode::*;
        let argvs = self.item_may_block()?.into_vec();
        // debug_println!("must_get_func_argv: {:?}", argvs);
        let alen = argvs.len();
        let argv = match md {
            ArgvMode::Concat => concat_func_argvs(argvs)?,
            ArgvMode::PackList => pack_func_argvs(argvs)?,
        };
        Ok((alen, argv))

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
            let (_, argvs) = self.must_get_func_argv(ArgvMode::Concat)?;
            return Ok(Box::new(IRNodeParam1Single{
                hrtv: true, inst: Bytecode::NTCALL, para: id, subx: argvs
            }))
        }

        // extend action
        if let Some((hrtv, argv, inst, para)) = pick_ext_func(&id) {
            let (num, argvres) = self.must_get_func_argv(ArgvMode::Concat)?;
            return Ok(match argv {
                false => {
                    if num > 0 {
                        return errf!("function '{}' cannot give argv", id)
                    }
                    Box::new(IRNodeParam1{hrtv, inst, para, text: s!("")})
                },
                true => {
                    Box::new(IRNodeParam1Single{hrtv, inst, para, subx: argvres})
                },
            })
        }

        // not find
        errf!("cannot find function '{}'", id)
    }

}



fn build_ir_func(inst: Bytecode, pms: usize, args: usize, rs: usize, argvs: Vec<Box<dyn IRNode>>) -> Ret<Box<dyn IRNode>> {
    use Bytecode::*;
    let mut argvs = VecDeque::from(argvs);
    let hrtv = maybe!(rs==1, true, false);
    let ttv = pms + args;
    if ttv == 0 {
        return Ok(Box::new(IRNodeLeaf::notext(hrtv, inst)))
    }
    macro_rules! avg {() => {
        argvs.pop_front().unwrap()       
    }}
    macro_rules! param { () => {{
        let mut para = -1i16;
        let e = errf!("ir func call param error");
        let ag = avg!();
        if let Some(n) = ag.as_any().downcast_ref::<IRNodeParam1>() {
            para = n.para as i16;
        } else if let Some(n) = ag.as_any().downcast_ref::<IRNodeParam2>() {
            para = i16::from_be_bytes(n.para);
        } else if let Some(n) = ag.as_any().downcast_ref::<IRNodeLeaf>() {
            para = match n.inst {
                P0 => 0,
                P1 => 1,
                _ => return e
            }
        }
        if para == -1 || para > 255{
            return e
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
            0 => Box::new(IRNodeParam1{hrtv, inst, para, text:s!("")}),
            1 => Box::new(IRNodeParam1Single{hrtv, inst, para, subx: avg!()}),
            _ => unreachable!()
        })
    }
    if pms == 2 {
        let p1 = param!();
        let p2 = param!();
        return Ok(match args {
            0 => Box::new(IRNodeParam2{hrtv, inst, para: [p1, p2]}),
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


fn concat_func_argvs(mut list: Vec<Box<dyn IRNode>>) -> Ret<Box<dyn IRNode>> {
    // list.reverse();
    let Some(mut res) = list.pop() else {
        return Ok(Syntax::push_inst(Bytecode::PNBUF)) // not pass argv
    };
    while let Some(x) = list.pop() {
        res = Box::new(IRNodeDouble{hrtv:true, inst:Bytecode::CAT, subx: x, suby: res});
    }
    Ok(res)
}


fn pack_func_argvs(mut subs: Vec<Box<dyn IRNode>>) -> Ret<Box<dyn IRNode>> {
    use Bytecode::*;
    // list.reverse();
    let argv_len = subs.len();
    Ok(match argv_len {
        0 => Box::new(IRNodeEmpty{}),// errf!("function argv length cannot be 0"),
        1 => subs.pop().unwrap(),
        2..=15 => {
            let num = Syntax::push_num(argv_len as u128);
            let pklist = Syntax::push_inst(PACKLIST);
            subs.push(num);
            subs.push(pklist);
            Box::new(IRNodeList{subs})
        },
        _ => return errf!("function argv length cannot more than 15"),
    })
    /* 
    let mut res = list.pop().unwrap();
    while let Some(x) = list.pop() {
        res = Box::new(IRNodeDouble{hrtv:true, inst:Bytecode::CAT, subx: x, suby: res});
    }
    res
    */
}



/*
    return (hav_revt, hav_argv, code, )
*/
fn pick_ext_func(id: &str) -> Option<(bool, bool, Bytecode, u8)> {
    if let Some(x) = CALL_EXTEND_ENV_DEFS.iter().find(|f|f.1==id) {
        return Some((true, false, Bytecode::EXTENV,  x.0))
    }
    if let Some(x) = CALL_EXTEND_FUNC_DEFS.iter().find(|f|f.1==id) {
        return Some((true, true,  Bytecode::EXTFUNC, x.0))
    }
    if let Some(x) = CALL_EXTEND_ACTION_DEFS.iter().find(|f|f.1==id) {
        return Some((false, true,  Bytecode::EXTACTION, x.0))
    }
    None
}






