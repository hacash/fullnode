pub fn convert_ir_to_bytecode(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    // Parse as raw block content (without IRBLOCK header) Input format: [node1][node2]... (no opcode/length prefix)
    let block = parse_ir_block(bytes, &mut 0)?;
    block.codegen()
}

pub fn verify_ir_runtime_safe_bytecodes(codes: &[u8]) -> VmrtErr {
    verify_ir_bytecode_stream(codes, /*require_terminal=*/ true)
}


#[cfg(feature = "execute")]
pub fn convert_ir_to_runtime_bytecode(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    let codes = convert_ir_to_bytecode(bytes)?;
    verify_ir_runtime_safe_bytecodes(&codes)?;
    verify_bytecodes(&codes)?;
    Ok(codes)
}

#[cfg(feature = "execute")]
pub fn runtime_irs_to_bytecodes(bytes: &[u8], gas_extra: &GasExtra) -> VmrtRes<Vec<u8>> {
    runtime_irs_to_exec_bytecodes(bytes, gas_extra)
}

#[cfg(feature = "execute")]
pub fn runtime_irs_to_exec_bytecodes(bytes: &[u8], _gas_extra: &GasExtra) -> VmrtRes<Vec<u8>> {
    let codes = convert_ir_to_runtime_bytecode(bytes)?;
    // Runtime executable stream is the compiled code only. IR-format gas is charged
    // at frame entry from raw IR length, so cached bytecode stays independent from gas policy.
    Ok(codes)
}

