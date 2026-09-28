/// Bytecode assemble/disassemble dispatch. Codec-safe.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsmFormat {
    Raw,
    Asm,
}

pub fn assemble_bytecode(text: &str) -> Ret<Vec<u8>> {
    if looks_like_asm_text(text) {
        assemble_bytecode_as(text, AsmFormat::Asm)
    } else {
        assemble_bytecode_as(text, AsmFormat::Raw)
    }
}

pub fn assemble_bytecode_as(text: &str, format: AsmFormat) -> Ret<Vec<u8>> {
    let bytes = match format {
        AsmFormat::Raw => assemble_bytecode_raw(text)?,
        AsmFormat::Asm => assemble_bytecode_asm(text)?,
    };
    decode_bytecode(&bytes)?;
    Ok(bytes)
}

pub fn format_bytecode(text: &str, format: AsmFormat) -> Ret<String> {
    let bytes = assemble_bytecode(text)?;
    match format {
        AsmFormat::Raw => disassemble_bytecode_raw(&bytes),
        AsmFormat::Asm => disassemble_bytecode_asm(&bytes),
    }
}
