

type IRNRef<'a> = &'a Box<dyn IRNode>;

// (u16::MAX/2 - jpl as u16).into();
const JMP_INST_LEN: usize = 3; //u8 + u16
const BLOCK_CODES_MAX_LEN: usize = i16::MAX as usize - JMP_INST_LEN - 64;


fn compile_block(list: &Vec<Box<dyn IRNode>>) -> VmrtRes<Vec<u8>> {
    let mut codes = vec![];
    for one in list {
        codes.append( &mut one.codegen()? );
        if one.hasretval() {
            codes.push(POP as u8);
        }
    }
    Ok(codes)
}


fn compile_double(btcd: Bytecode, x: IRNRef, y: IRNRef) -> VmrtRes<Option<Vec<u8>>> {
    Ok(match btcd {
        IRWHILE => Some(compile_while(x, y)?),
        _ => None
    })
}


fn compile_while(x: IRNRef, y: IRNRef) -> VmrtRes<Vec<u8>> {
    const JIL: usize = JMP_INST_LEN;
    const MAXL: usize = BLOCK_CODES_MAX_LEN;
    // condition
    let cond = x.codegen()?;
    let mut body = y.codegen()?;
    if y.hasretval() {
        body.push(POP as u8); // pop inst
    }
    let body_l = body.len() + JIL;
    let alls_l = body_l + cond.len() + JIL;
    // check code len
    if body_l > MAXL || alls_l > MAXL {
        return itr_err_fmt!(ComplieError, "compile ir codes too long")
    }
    // condition
    Ok(iter::empty().chain(cond)
    // if false to break
    .chain([BRSLN as u8])
    .chain((body_l as i16).to_be_bytes())
    // exec body
    .chain(body)
    // back jump to condition
    .chain([JMPSL as u8])
    .chain((-(alls_l as i16)).to_be_bytes())
    // end
    .collect())
}


/**************************************************/


fn compile_triple(btcd: Bytecode, x: IRNRef, y: IRNRef, z: IRNRef) -> VmrtRes<Option<Vec<u8>>> {
    Ok(match btcd {
        IRIF => Some(compile_if(x, y, z)?),
        _ => None
    })
}


fn compile_if(x: IRNRef, y: IRNRef, z: IRNRef) -> VmrtRes<Vec<u8>> {
    const JIL: usize = JMP_INST_LEN;
    const MAXL: usize = BLOCK_CODES_MAX_LEN;
    let cond  = x.codegen()?;
    let mut if_br = y.codegen()?;
    if y.hasretval() {
        if_br.push(POP as u8); // pop inst
    }
    let mut el_br = z.codegen()?;
    if z.hasretval() {
        el_br.push(POP as u8); // pop inst
    }
    let if_l = if_br.len();
    let el_l = el_br.len() + JIL;
    // check code len
    if if_l > MAXL || el_l > MAXL {
        return itr_err_fmt!(ComplieError, "compile ir codes too long")
    }
    // condition
    Ok(iter::empty().chain(cond)
    // check if jmp to if
    .chain([BRSL as u8])
    .chain((el_l as i16).to_be_bytes())
    // else br body
    .chain(el_br)
    // jump to end
    .chain([JMPSL as u8])
    .chain((if_l as i16).to_be_bytes())
    // if br body
    .chain(if_br)
    // end
    .collect())
}

