

pub fn convert_ir_to_bytecode(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    let irs = parse_ir_block(bytes, &mut 0)?;
    irs.codegen()
}

pub fn runtime_irs_to_bytecodes(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    let mut codes = convert_ir_to_bytecode(bytes)?;
    let mut rescodes = Vec::with_capacity(codes.len() + 4);
    // append burn gas & end
    let cdl = ((codes.len() / 8) as u16).to_be_bytes(); // burn gas = size / 8
    let mut bgas = vec![BURN as u8, cdl[0], cdl[1]];
    rescodes.append(&mut bgas);
    rescodes.append(&mut codes); // code body
    rescodes.push(END as u8);
    Ok(rescodes)
}
