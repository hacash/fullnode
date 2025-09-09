
pub fn try_compile_check(ctype: CodeType, codes: &[u8]) -> VmrtErr {
    match ctype {
        CodeType::IRNode => {
            let cdres = compile_irs_to_bytecodes(codes)?;
            verify_bytecodes(&cdres)?;
        },
        CodeType::Bytecode => {
            verify_bytecodes(codes)?;
        }
    };
    Ok(())
}

pub fn compile_irs_to_bytecodes(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    let irs = parse_ir_block(bytes, &mut 0)?;
    irs.codegen().map(|mut c|{ c.push(Bytecode::END as u8); c })
}
