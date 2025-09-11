
pub fn convert_and_check(cap: &SpaceCap, ctype: CodeType, codes: &[u8]) -> VmrtErr {
    use CodeType::*;
    let bytecodes = match ctype {
        IRNode =>  &runtime_irs_to_bytecodes(codes)?,
        Bytecode => codes
    };
    // check size
    if bytecodes.len() > cap.one_function_size {
        return itr_err_code!(CodeTooLong)
    }
    // verify inst
    verify_bytecodes(bytecodes)
}

pub fn convert_irs_to_bytecodes(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    let irs = parse_ir_block(bytes, &mut 0)?;
    irs.codegen()
}

pub fn runtime_irs_to_bytecodes(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    let mut codes = convert_irs_to_bytecodes(bytes)?;
    // append burn gas & end
    let cdl = ((codes.len() / 4) as u16).to_be_bytes(); // burn gas = size / 4
    let mut tail = vec![BURN as u8, cdl[0], cdl[1], END as u8];
    codes.append(&mut tail);
    Ok(codes)
}
