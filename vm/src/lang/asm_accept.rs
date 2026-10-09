//! Acceptance suite for bytecode/ircode bidirectional assembly.

use super::asm::{
    assemble_bytecode, assemble_bytecode_as, assemble_ircode, decode_bytecode,
    disassemble_bytecode_asm, disassemble_bytecode_raw, disassemble_ircode_asm,
    disassemble_ircode_raw, format_bytecode, format_ircode, AsmFormat,
};
use crate::rt::Bytecode::*;
use crate::rt::*;

fn push_end(mut v: Vec<u8>) -> Vec<u8> {
    v.push(END as u8);
    v
}

fn roundtrip_bytecode(bytes: &[u8]) {
    let raw = disassemble_bytecode_raw(bytes).unwrap_or_else(|e| panic!("raw disasm: {e}\n{bytes:?}"));
    let asm = disassemble_bytecode_asm(bytes).unwrap_or_else(|e| panic!("asm disasm: {e}\n{bytes:?}"));
    assert_eq!(
        assemble_bytecode(&raw).expect("assemble raw"),
        bytes,
        "raw roundtrip failed\nraw:\n{raw}"
    );
    assert_eq!(
        assemble_bytecode(&asm).expect("assemble asm"),
        bytes,
        "asm roundtrip failed\nasm:\n{asm}"
    );
    let raw2 = format_bytecode(&raw, AsmFormat::Raw).expect("format raw");
    let asm2 = format_bytecode(&asm, AsmFormat::Asm).expect("format asm");
    assert_eq!(
        format_bytecode(&raw2, AsmFormat::Raw).expect("format raw 2"),
        raw2,
        "raw format not idempotent"
    );
    assert_eq!(
        format_bytecode(&asm2, AsmFormat::Asm).expect("format asm 2"),
        asm2,
        "asm format not idempotent"
    );
    assert_eq!(
        assemble_bytecode_as(&raw2, AsmFormat::Raw).expect("assemble formatted raw"),
        bytes
    );
    let via_asm = assemble_bytecode(&raw).unwrap();
    let printed_asm = disassemble_bytecode_asm(&via_asm).unwrap();
    assert_eq!(assemble_bytecode(&printed_asm).unwrap(), bytes);
}

fn roundtrip_ircode(bytes: &[u8]) {
    let raw = disassemble_ircode_raw(bytes).unwrap_or_else(|e| panic!("ircode raw: {e}"));
    let asm = disassemble_ircode_asm(bytes).unwrap_or_else(|e| panic!("ircode asm: {e}"));
    assert_eq!(
        assemble_ircode(&raw).expect("assemble ircode raw"),
        bytes,
        "ircode raw\n{raw}"
    );
    assert_eq!(
        assemble_ircode(&asm).expect("assemble ircode asm"),
        bytes,
        "ircode asm\n{asm}"
    );
    let raw2 = format_ircode(&raw, AsmFormat::Raw).expect("format ircode raw");
    let asm2 = format_ircode(&asm, AsmFormat::Asm).expect("format ircode asm");
    assert_eq!(format_ircode(&raw2, AsmFormat::Raw).unwrap(), raw2);
    assert_eq!(format_ircode(&asm2, AsmFormat::Asm).unwrap(), asm2);
}

fn assert_asm_clean(text: &str) {
    for noise in [
        "block:",
        "entry:",
        " · ",
        "--",
        "?#",
        "calluseview[",
        "push_buf[",
    ] {
        assert!(
            !text.contains(noise),
            "asm still contains noisy '{noise}':\n{text}"
        );
    }
}

#[test]
fn empty_stream_roundtrips() {
    roundtrip_bytecode(&[]);
    roundtrip_ircode(&[]);
}

#[test]
fn single_terminal_and_push_immediates() {
    for op in [
        END, ABT, NOP, P0, P1, P2, P3, PNIL, PNBUF, PTRUE, PFALSE, GET0, GET1, GET2, GET3,
    ] {
        roundtrip_bytecode(&[op as u8]);
    }
    roundtrip_bytecode(&push_end(vec![PU8 as u8, 0]));
    roundtrip_bytecode(&push_end(vec![PU8 as u8, 255]));
    roundtrip_bytecode(&push_end(vec![PU16 as u8, 0, 0]));
    roundtrip_bytecode(&push_end(vec![PU16 as u8, 1, 0]));
    roundtrip_bytecode(&push_end(vec![ALLOC as u8, 1, GET as u8, 0]));
    roundtrip_bytecode(&push_end(vec![P1 as u8, PUT as u8, 255]));
}

#[test]
fn pu16_raw_is_bytewise_not_combined_integer() {
    let bytes = push_end(vec![PU16 as u8, 1, 0]);
    let raw = disassemble_bytecode_raw(&bytes).unwrap();
    assert!(
        raw.contains("PU16 1 0"),
        "expected bytewise PU16 print, got {raw}"
    );
    assert!(
        !raw.contains("PU16 256"),
        "raw must not combine BE u16: {raw}"
    );
}

#[test]
fn pbuf_payload_not_scanned_as_opcodes() {
    let bytes = push_end(vec![PBUF as u8, 3, 0xef, 0x00, 0xff]);
    let insts = decode_bytecode(&bytes).unwrap();
    assert_eq!(insts[0].op, PBUF);
    assert_eq!(insts[0].imm, vec![3, 0xef, 0x00, 0xff]);
    roundtrip_bytecode(&bytes);

    roundtrip_bytecode(&push_end(vec![PBUF as u8, 0]));

    let mut max = vec![PBUF as u8, 255];
    max.extend(std::iter::repeat(0x61u8).take(255));
    max.push(END as u8);
    roundtrip_bytecode(&max);
}

#[test]
fn pbufl_length_big_endian() {
    let mut bytes = vec![PBUFL as u8, 1, 0];
    bytes.extend(std::iter::repeat(0x5au8).take(256));
    bytes.push(END as u8);
    let insts = decode_bytecode(&bytes).unwrap();
    assert_eq!(insts[0].op, PBUFL);
    assert_eq!(insts[0].imm.len(), 2 + 256);
    roundtrip_bytecode(&bytes);

    roundtrip_bytecode(&push_end(vec![PBUFL as u8, 0, 0]));
}

#[test]
fn truncated_streams_error_without_panic() {
    assert!(decode_bytecode(&[PU8 as u8]).is_err());
    assert!(disassemble_bytecode_raw(&[PBUF as u8, 4, 1, 2]).is_err());
    assert!(disassemble_bytecode_raw(&[PBUFL as u8, 0]).is_err());
    assert!(assemble_bytecode_as("push_u8", AsmFormat::Asm).is_err());
    assert!(decode_bytecode(&[0x01]).is_err());
    assert!(decode_bytecode(&[0x77]).is_err());
}

#[test]
fn p2sh_lockbox_roundtrip() {
    const HEX: &str = "7f0107027c00070122030bba7c32a7e5001f80221500b11f3f59601a0da2e0c471e460fb03bf156f46f33709a2ebe2001c8022150039e26861b8743434b95a1af10cb7051a429a9ce73709a2eb24ee";
    let codes = hex::decode(HEX).unwrap();
    assert_eq!(codes.len(), 79);
    let insts = decode_bytecode(&codes).unwrap();
    let names: Vec<_> = insts.iter().map(|i| format!("{:?}", i.op)).collect();
    assert_eq!(
        names,
        [
            "ALLOC", "ACTENV", "PUT", "ACTENV", "PBUF", "CU32", "GE", "BRSL", "GET0", "PBUF",
            "CTO", "EQ", "AST", "JMPSL", "GET0", "PBUF", "CTO", "EQ", "AST", "P0", "RET",
        ]
    );
    roundtrip_bytecode(&codes);
}

#[test]
fn jump_over_pbuf_payload_labels_land_on_instruction() {
    let codes = vec![PBUF as u8, 2, 0xAA, 0xBB, JMPL as u8, 0, 7, END as u8];
    let asm = disassemble_bytecode_asm(&codes).unwrap();
    assert_asm_clean(&asm);
    assert!(
        asm.contains(".L7:"),
        "expected canonical label at dest 7:\n{asm}"
    );
    assert!(!asm.contains("block:"));
    roundtrip_bytecode(&codes);
}

#[test]
fn relative_and_absolute_jumps_preserve_width() {
    roundtrip_bytecode(&[JMPS as u8, 0, END as u8]);
    roundtrip_bytecode(&[JMPL as u8, 0, 3, END as u8]);
    roundtrip_bytecode(&[JMPSL as u8, 0, 0, END as u8]);
    roundtrip_bytecode(&push_end(vec![PTRUE as u8, BRS as u8, 0]));
    roundtrip_bytecode(&push_end(vec![PTRUE as u8, BRSL as u8, 0, 0]));
    roundtrip_bytecode(&push_end(vec![PTRUE as u8, BRSLN as u8, 0, 0]));
}

#[test]
fn jmps_out_of_range_does_not_widen() {
    let mut far = String::from("  jump_offset .Lfar\n");
    for _ in 0..200 {
        far.push_str("  nop\n");
    }
    far.push_str(".Lfar:\n  end\n");
    assert!(
        assemble_bytecode(&far).is_err(),
        "jump_offset must not silently widen to jump_offset_long"
    );
}

#[test]
fn custom_label_reprints_as_canonical_offset() {
    let src = "  jump_long .loop\n.loop:\n  end\n";
    let bytes = assemble_bytecode(src).expect("custom label");
    let printed = disassemble_bytecode_asm(&bytes).unwrap();
    assert!(
        printed.contains(".L"),
        "canonical labels use .L{{offset}}\n{printed}"
    );
    assert_eq!(assemble_bytecode(&printed).unwrap(), bytes);
}

#[test]
fn undefined_and_duplicate_labels_error() {
    assert!(assemble_bytecode("  jump_long .missing\n  end").is_err());
    assert!(assemble_bytecode(".L0:\n  end\n.L0:\n  nop").is_err());
}

#[test]
fn form2_has_no_legacy_noise_and_single_call_operand() {
    let selector = [0x01u8, 0x02, 0x03, 0x04];
    let (inst, para) = encode_user_call_site(CallSpec::Invoke {
        target: CallTarget::Use(7),
        effect: EffectMode::View,
        selector,
    });
    let mut codes = vec![inst as u8];
    codes.extend_from_slice(&para);
    codes.push(END as u8);
    let asm = disassemble_bytecode_asm(&codes).unwrap();
    assert_asm_clean(&asm);
    assert!(
        asm.to_lowercase().contains("calluseview"),
        "expected calluseview mnemonic:\n{asm}"
    );
    assert!(
        asm.contains("01020304") || asm.contains("0x01020304"),
        "expected selector hex:\n{asm}"
    );
    assert!(
        !asm.contains("call view use"),
        "must not double-decode CALL:\n{asm}"
    );
    roundtrip_bytecode(&codes);
    assert!(
        asm.lines()
            .any(|l| l.starts_with(crate::rt::ASM_INDENT) && !l.starts_with("    ")),
        "form 2 indent must be two spaces:\n{asm}"
    );
}

#[test]
fn all_user_call_forms_roundtrip() {
    let selector_a = [0x01u8, 0x02, 0x03, 0x04];
    let selector_b = [0xaa, 0xbb, 0xcc, 0xdd];
    let cases = [
        CallSpec::Invoke {
            target: CallTarget::This,
            effect: EffectMode::Edit,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Self_,
            effect: EffectMode::Edit,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Super,
            effect: EffectMode::Edit,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Self_,
            effect: EffectMode::View,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Self_,
            effect: EffectMode::Pure,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Ext(7),
            effect: EffectMode::Edit,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Ext(7),
            effect: EffectMode::View,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Use(7),
            effect: EffectMode::View,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Use(7),
            effect: EffectMode::Pure,
            selector: selector_a,
        },
        CallSpec::Invoke {
            target: CallTarget::Upper,
            effect: EffectMode::View,
            selector: selector_b,
        },
        CallSpec::Splice {
            lib: 9,
            selector: selector_b,
        },
    ];
    for call in cases {
        let (inst, para) = encode_user_call_site(call);
        let mut codes = vec![inst as u8];
        codes.extend_from_slice(&para);
        codes.push(END as u8);
        roundtrip_bytecode(&codes);
    }
}

#[test]
fn push_u8_one_does_not_collapse_to_p1() {
    let bytes = push_end(vec![PU8 as u8, 1]);
    let asm = disassemble_bytecode_asm(&bytes).unwrap();
    assert_eq!(
        assemble_bytecode(&asm).unwrap(),
        bytes,
        "push_u8 1 must remain PU8\n{asm}"
    );
    let raw = disassemble_bytecode_raw(&bytes).unwrap();
    assert!(raw.contains("PU8"), "{raw}");
    assert!(
        !raw.contains("P1 "),
        "raw of PU8 1 must not print as P1: {raw}"
    );
}

#[test]
fn xop_xlg_all_ops_roundtrip() {
    for op in [LxOp::Add, LxOp::Sub, LxOp::Mul, LxOp::Div] {
        let mark = encode_local_operand_mark(op, 3).unwrap();
        roundtrip_bytecode(&push_end(vec![P1 as u8, XOP as u8, mark]));
    }
    for op in [
        LxLg::And,
        LxLg::Or,
        LxLg::Eq,
        LxLg::Ne,
        LxLg::Gt,
        LxLg::Ge,
        LxLg::Lt,
        LxLg::Le,
    ] {
        let mark = encode_local_logic_mark(op, 2).unwrap();
        roundtrip_bytecode(&push_end(vec![P1 as u8, XLG as u8, mark]));
    }
}

#[test]
fn native_and_action_names_and_ids_both_assemble() {
    roundtrip_bytecode(&push_end(vec![ACTENV as u8, 0x01]));
    roundtrip_bytecode(&push_end(vec![NTFUNC as u8, 1]));
    let numeric = assemble_bytecode("ACTENV 1 END").unwrap();
    assert_eq!(numeric, vec![ACTENV as u8, 1, END as u8]);
}

#[test]
fn mixed_debug_and_intro_mnemonics_are_rejected() {
    assert!(assemble_bytecode("push_1 PUT 0 END").is_err());
    assert!(assemble_bytecode("  push_1\n  PUT 0\n  end").is_err());
}

#[test]
fn form2_rejects_four_space_and_tab_indent() {
    assert!(
        assemble_bytecode("    push_1\n    end").is_err(),
        "4-space indent must be rejected"
    );
    assert!(
        assemble_bytecode("\tpush_1\n\tend").is_err(),
        "tab indent must be rejected"
    );
    assert!(
        assemble_bytecode("   push_1\n   end").is_err(),
        "odd indent must be rejected"
    );
    let ok = assemble_bytecode("  push_1\n  end").unwrap();
    assert_eq!(ok, vec![P1 as u8, END as u8]);
}

#[test]
fn comments_and_whitespace_ignored() {
    let compact = assemble_bytecode("P1 PUT 0 END").unwrap();
    let noisy = assemble_bytecode(
        r#"
// leading
P1  ; comment
PUT 0
END
"#,
    )
    .unwrap();
    assert_eq!(compact, noisy);
}

#[test]
fn pbuf_decimal_bytes_and_hex_payload_equal() {
    let a = assemble_bytecode("PBUF 3 97 98 99 END").unwrap();
    let b = assemble_bytecode("PBUF 3 0x616263 END").unwrap();
    assert_eq!(a, b);
    let raw = disassemble_bytecode_raw(&a).unwrap();
    assert!(
        raw.contains("0x616263"),
        "canonical raw PBUF uses hex payload: {raw}"
    );
}

#[test]
fn keyword_collision_mnemonics_are_instructions() {
    assemble_bytecode("END").unwrap_or_else(|e| panic!("END: {e}"));
    assemble_bytecode("ABT").unwrap_or_else(|e| panic!("ABT: {e}"));
    assemble_bytecode("  end").unwrap_or_else(|e| panic!("end: {e}"));
    assemble_bytecode("P1 RET").unwrap_or_else(|e| panic!("RET: {e}"));
    assemble_bytecode("P1 AST END").unwrap_or_else(|e| panic!("AST: {e}"));
}

#[test]
fn req_roundtrips_in_raw_and_asm_forms() {
    let bytes = [P0 as u8, PTRUE as u8, REQ as u8, END as u8];
    roundtrip_bytecode(&bytes);
    assert!(disassemble_bytecode_raw(&bytes).unwrap().contains("REQ"));
    assert!(disassemble_bytecode_asm(&bytes).unwrap().contains("require"));
}

#[test]
fn ircode_prefix_put_differs_from_bytecode_postfix() {
    let ir = vec![PUT as u8, 0, P1 as u8];
    roundtrip_ircode(&ir);
    let raw = disassemble_ircode_raw(&ir).unwrap();
    assert!(
        raw.contains("PUT") && raw.contains("P1"),
        "ircode raw should show prefix PUT then child P1: {raw}"
    );
    let rt = vec![P1 as u8, PUT as u8, 0, END as u8];
    roundtrip_bytecode(&rt);
    assert_ne!(ir, rt);
}

#[test]
fn ircode_nested_leaves() {
    roundtrip_ircode(&[P1 as u8]);
    roundtrip_ircode(&[ALLOC as u8, 1, PUT as u8, 0, P1 as u8]);
}

#[test]
fn ircode_rejects_absolute_jump_in_embedded_bytecode() {
    let a = assemble_ircode("IRBYTECODE 0 3 JMPL 0 0");
    let b = assemble_ircode("ir_bytecode {\n  jump_long 0\n  end\n}");
    assert!(a.is_err() || b.is_err());
}

#[cfg(feature = "execute")]
#[test]
fn fitsh_corpus_ircode_survives_asm_roundtrip() {
    let samples = [
        "return 1\nend",
        "var $0 = 1\nreturn $0\nend",
        "bytecode { PBUF 3 97 98 99 }\nend",
        "if 1 { return 2 } else { return 3 }\nend",
        "while 0 { break }\nend",
    ];
    for src in samples {
        let ir = super::lang_to_ircode(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        roundtrip_ircode(&ir);
        let rt = super::lang_to_bytecode(src).unwrap_or_else(|e| panic!("bc {src}: {e}"));
        roundtrip_bytecode(&rt);
    }
}

#[test]
fn dirty_asm_formats_to_stable_canonical() {
    let dirty = "\n\n  push_1  ; c\n  local_put   0\n  end\n\n";
    let formatted = format_bytecode(dirty, AsmFormat::Asm).unwrap();
    assert_eq!(
        format_bytecode(&formatted, AsmFormat::Asm).unwrap(),
        formatted
    );
    let bytes = assemble_bytecode(dirty).unwrap();
    roundtrip_bytecode(&bytes);
}
