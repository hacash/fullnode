pub fn convert_ir_to_bytecode(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    // Parse as raw block content (without IRBLOCK header) Input format: [node1][node2]... (no opcode/length prefix)
    let block = parse_ir_block(bytes, &mut 0)?;
    block.codegen()
}

pub fn verify_ir_runtime_safe_bytecodes(codes: &[u8]) -> VmrtErr {
    verify_ir_bytecode_stream(codes, /*require_terminal=*/ true)
}

/// Verify an IR bytecode sub-stream (the payload of an `IRNodeBytecodes`).
///
/// Unlike `verify_ir_runtime_safe_bytecodes`, this allows the stream to be
/// non-terminal (it is a fragment that will be composed with siblings) and
/// allows it to be empty (the parser already rejects out-of-range payload
/// lengths). Everything else — valid opcodes, no IR-only opcodes, no absolute
/// jumps, parameter byte alignment — must still hold so that downstream
/// scanners (rewriter, runtime verifier) cannot be derailed.
pub fn verify_ir_bytecode_stream_fragment(codes: &[u8]) -> VmrtErr {
    verify_ir_bytecode_stream(codes, /*require_terminal=*/ false)
}

fn verify_ir_bytecode_stream(codes: &[u8], require_terminal: bool) -> VmrtErr {
    let mut i = 0usize;
    let mut last = None;
    while i < codes.len() {
        let inst = Bytecode::try_from_u8(codes[i])?;
        last = Some(inst);
        let meta = inst.metadata();
        if !meta.valid {
            return itr_err_fmt!(InstInvalid, "bytecode {} not found", inst as u8);
        }
        // IR-only opcodes must have been lowered by codegen. Catching any
        // residual occurrence here is the first line of defense — without it,
        // a stray IRBREAK / IRCONTINUE / IRBLOCK / ... slips through with
        // `meta.param=0` scanning and shifts the rest of the stream.
        if matches!(
            inst,
            IRBYTECODE | IRLIST | IRBLOCK | IRBLOCKR | IRIF | IRIFR | IRWHILE | IRBREAK
                | IRCONTINUE
        ) {
            return itr_err_fmt!(
                InstInvalid,
                "IR bytecode {:?} leaked into runtime stream",
                inst
            );
        }
        if matches!(inst, JMPL | BRL) {
            return itr_err_fmt!(
                InstInvalid,
                "absolute jumps are not allowed in IRNode code; use relative jumps"
            );
        }
        i += 1;
        let end = match inst {
            PBUF => {
                if i >= codes.len() {
                    return itr_err_code!(InstParamsErr);
                }
                i + 1 + codes[i] as usize
            }
            PBUFL => {
                if i + 2 > codes.len() {
                    return itr_err_code!(InstParamsErr);
                }
                let len = u16::from_be_bytes(codes[i..i + 2].try_into().unwrap()) as usize;
                i + 2 + len
            }
            _ => i + meta.param as usize,
        };
        if end > codes.len() {
            return itr_err_code!(InstParamsErr);
        }
        i = end;
    }
    if require_terminal {
        let Some(last) = last else {
            return itr_err_code!(CodeEmpty);
        };
        ensure_terminal_instruction(last)?;
    }
    Ok(())
}

pub fn convert_ir_to_runtime_bytecode(bytes: &[u8]) -> VmrtRes<Vec<u8>> {
    let codes = convert_ir_to_bytecode(bytes)?;
    verify_ir_runtime_safe_bytecodes(&codes)?;
    verify_bytecodes(&codes)?;
    Ok(codes)
}

pub fn runtime_irs_to_bytecodes(bytes: &[u8], gas_extra: &GasExtra) -> VmrtRes<Vec<u8>> {
    runtime_irs_to_exec_bytecodes(bytes, gas_extra)
}

pub fn runtime_irs_to_exec_bytecodes(bytes: &[u8], _gas_extra: &GasExtra) -> VmrtRes<Vec<u8>> {
    let codes = convert_ir_to_runtime_bytecode(bytes)?;
    // Runtime executable stream is the compiled code only. IR-format gas is charged
    // at frame entry from raw IR length, so cached bytecode stays independent from gas policy.
    Ok(codes)
}

