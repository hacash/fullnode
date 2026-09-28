use sys::*;

pub use crate::rt::{
    assemble_bytecode, assemble_bytecode_as, decode_bytecode, disassemble_bytecode_asm,
    disassemble_bytecode_raw, format_bytecode, AsmFormat, DecodedInst,
};

pub fn disassemble_ircode_raw(codes: &[u8]) -> Ret<String> {
    crate::rt::disassemble_bytecode_raw(codes)
}

pub fn disassemble_ircode_asm(codes: &[u8]) -> Ret<String> {
    ircode::disassemble_ircode_asm(codes)
}

pub fn assemble_ircode(text: &str) -> Ret<Vec<u8>> {
    if crate::rt::looks_like_asm_text(text) {
        ircode::assemble_ircode_asm(text)
    } else {
        assemble_ircode_raw(text)
    }
}

pub fn assemble_ircode_as(text: &str, format: AsmFormat) -> Ret<Vec<u8>> {
    match format {
        AsmFormat::Raw => assemble_ircode_raw(text),
        AsmFormat::Asm => ircode::assemble_ircode_asm(text),
    }
}

pub fn format_ircode(text: &str, format: AsmFormat) -> Ret<String> {
    let bytes = assemble_ircode(text)?;
    match format {
        AsmFormat::Raw => disassemble_ircode_raw(&bytes),
        AsmFormat::Asm => disassemble_ircode_asm(&bytes),
    }
}

fn assemble_ircode_raw(text: &str) -> Ret<Vec<u8>> {
    let bytes = crate::rt::assemble_bytecode_raw(text)?;
    validate_ircode_body(&bytes)?;
    Ok(bytes)
}

fn validate_ircode_body(bytes: &[u8]) -> Ret<()> {
    let mut seek = 0usize;
    crate::ir::parse_ir_block(bytes, &mut seek).map_err(|e| Error::normal(e.to_string()))?;
    if seek != bytes.len() {
        return errf!(
            "ircode body trailing bytes at {} (len {})",
            seek,
            bytes.len()
        );
    }
    Ok(())
}

mod ircode;
