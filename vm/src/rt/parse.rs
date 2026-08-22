/// Bytecode disassembly to human-readable assembly text (`bytecode_print`),
/// ported from the fullnodedev VM. `desc = true` renders annotated assembly
/// with jump-dest blocks and decoded operands (action/native names, user call
/// sites, pbuf payloads); `desc = false` renders the dense one-line form used
/// by IR node printing.
pub trait BytecodePrint {
    fn bytecode_print(&self, desc: bool) -> VmrtRes<String>;
}

impl BytecodePrint for Vec<u8> {
    fn bytecode_print(&self, desc: bool) -> VmrtRes<String> {
        let mut res = String::new();
        let mut jpdests = vec![];
        if desc {
            jpdests = scan_jump_dests(self);
            res.push_str(&format!("block: {:?}\nentry:\n", jpdests));
        }
        let max = self.len();
        let mut i = 0;
        macro_rules! pu16 {
            ($i:expr) => {
                u16::from_be_bytes(self[$i..$i + 2].try_into().unwrap())
            };
        }
        while i < max {
            let byte = self[i];
            let inst: Bytecode = std_mem_transmute!(byte);
            let meta = inst.metadata();
            if desc {
                let mut line = if jpdests.contains(&i) {
                    format!("#{}:\n    ", i)
                } else {
                    s!("    ")
                };
                match inst {
                    P0 => line += &format!("{} · ", 0),
                    P1 => line += &format!("{} · ", 1),
                    P2 => line += &format!("{} · ", 2),
                    P3 => line += &format!("{} · ", 3),
                    PTRUE => line += "true · ",
                    PFALSE => line += "false · ",
                    PU8 => line += &format!("{} · ", self[i + 1]),
                    PU16 => line += &format!("{} · ", pu16!(i + 1)),
                    RET | END | ERR | ABT => line += "--",
                    JMPL | JMPS | JMPSL => line += "# ",
                    BRL | BRS | BRSL | BRSLN => line += "?# ",
                    _ => {}
                }
                res.push_str(&format!("{}{}", line, meta.intro));
            } else {
                res.push_str(&format!("{:?} ", inst));
            }
            if !meta.valid {
                return itr_err_fmt!(InstInvalid, "bytecode_print failed for inst {}", byte);
            }
            i += 1;
            if meta.param > 0 {
                // Bounds guard: malformed/truncated code must degrade to an
                // error, never index out of bounds (the SDK boundary feeds
                // arbitrary hex into this path).
                if i + meta.param as usize > max {
                    return itr_err_fmt!(
                        InstInvalid,
                        "bytecode_print failed: inst {} params truncated at {}..{}",
                        byte,
                        i,
                        i + meta.param as usize
                    );
                }
                let mut pms = vec![];
                let mut nmpm = || {
                    for k in 0..meta.param {
                        pms.push(format!("{}", self[i + k as usize]));
                    }
                };
                if desc {
                    res.push_str("[");
                    if let JMPS | BRS = inst {
                        let s = self[i];
                        pms.push(format!(" -#{}- ", s as isize + 1));
                    } else if let JMPL | BRL = inst {
                        // JMPL/BRL use absolute u16 program counter target.
                        let t = pu16!(i);
                        pms.push(format!(" #{} ", t));
                    } else if let JMPSL | BRSL | BRSLN = inst {
                        let s = pu16!(i) as i16;
                        pms.push(format!(" -#{}- ", i as isize + s as isize + 2));
                    } else if let ACTENV = inst {
                        let ary = ACTION_ENV_DEFS;
                        let f = search_act_name_by_id(self[i], &ary);
                        pms.push(format!(" {}() ", f));
                    } else if let ACTVIEW = inst {
                        let ary = ACTION_VIEW_DEFS;
                        let f = search_act_name_by_id(self[i], &ary);
                        pms.push(format!(" {}(..) ", f));
                    } else if let XOP = inst {
                        let (opt, idx) = local_operand_param_parse(self[i]);
                        pms.push(format!("{}, {}", idx, opt));
                    } else if let XLG = inst {
                        let (opt, idx) = local_logic_param_parse(self[i]);
                        pms.push(format!("{}, {}", idx, opt));
                    } else if is_user_call_inst(inst) {
                        let body = &self[i..i + meta.param as usize];
                        match decode_user_call_site(inst, body) {
                            Ok(CallSpec::Invoke {
                                target,
                                effect,
                                selector,
                            }) => {
                                let eff = match effect {
                                    EffectMode::Edit => "edit",
                                    EffectMode::View => "view",
                                    EffectMode::Pure => "pure",
                                };
                                let tgt = match target {
                                    CallTarget::This => "this".to_string(),
                                    CallTarget::Self_ => "self".to_string(),
                                    CallTarget::Upper => "upper".to_string(),
                                    CallTarget::Super => "super".to_string(),
                                    CallTarget::Ext(n) => format!("ext({})", n),
                                    CallTarget::Use(n) => format!("use({})", n),
                                };
                                pms.push(format!("call {} {}.<{}>", eff, tgt, hex::encode(&selector)));
                            }
                            Ok(CallSpec::Splice { lib, selector }) => {
                                pms.push(format!("codecall {}.<{}>", lib, hex::encode(&selector)));
                            }
                            Err(_) => nmpm(),
                        }
                    } else {
                        nmpm();
                    }
                } else {
                    nmpm();
                }
                if let PBUF = inst {
                    let n = self[i] as usize;
                    i += 1;
                    let r = i + n;
                    if r > max {
                        return itr_err_fmt!(
                            InstInvalid,
                            "bytecode_print failed: PBUF payload truncated at {}..{}",
                            i,
                            r
                        );
                    }
                    pms.push(format!("0x{}", hex::encode(&self[i..r])));
                    i = r - 1;
                } else if let PBUFL = inst {
                    if i + 2 > max {
                        return itr_err_fmt!(
                            InstInvalid,
                            "bytecode_print failed: PBUFL length truncated"
                        );
                    }
                    let n = u16::from_be_bytes(self[i..i + 2].try_into().unwrap()) as usize;
                    i += 2;
                    let r = i + n;
                    if r > max {
                        return itr_err_fmt!(
                            InstInvalid,
                            "bytecode_print failed: PBUFL payload truncated at {}..{}",
                            i,
                            r
                        );
                    }
                    pms.push(format!("0x{}", hex::encode(&self[i..r])));
                    i = r - 1;
                }
                if desc {
                    res.push_str(&pms.join(","));
                    res.push_str("]");
                } else {
                    res.push_str(&pms.join(" "));
                    res.push_str(" ");
                }
            }
            i += meta.param as usize;
            if desc {
                res.push_str("\n");
            }
        }
        Ok(res)
    }
}

/* return block mark */
fn scan_jump_dests(codes: &[u8]) -> Vec<usize> {
    let mut dests = vec![];
    let cdl = codes.len();
    let mut i = 0;
    macro_rules! adddest {
        ($jt:expr) => {{
            dests.push($jt as usize)
        }};
    }
    macro_rules! pu8 {
        () => {{
            codes[i as usize]
        }};
    }
    macro_rules! pi8 {
        () => {
            pu8!() as i8
        };
    }
    macro_rules! pu16 {
        () => {{
            let r = i + 2;
            u16::from_be_bytes(codes[i as usize..r as usize].try_into().unwrap())
        }};
    }
    macro_rules! pi16 {
        () => {
            pu16!() as i16
        };
    }
    while i < cdl {
        let inst: Bytecode = std_mem_transmute!(codes[i]);
        let meta = inst.metadata();
        i += 1;
        // Bounds guard for truncated streams (the SDK boundary feeds arbitrary
        // hex); a partial jump/prefix operand is treated as a non-jump.
        if i + meta.param as usize > cdl {
            break;
        }
        match inst {
            // Payload only; length prefix is covered by meta.param (same as verify.rs).
            PBUF => i += pu8!() as usize,
            PBUFL => i += pu16!() as usize,
            JMPL | BRL => adddest!(pu16!()),
            JMPS | BRS => adddest!(i as isize + pi8!() as isize + 1),
            JMPSL | BRSL | BRSLN => adddest!(i as isize + pi16!() as isize + 2),
            _ => {}
        };
        i += meta.param as usize;
    }

    dests
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn scan_jump_dests_skips_pbuf_payload_before_relative_jump() {
        let codes = vec![
            Bytecode::PU8 as u8,
            1,
            Bytecode::PBUF as u8,
            4,
            0,
            0,
            0,
            0,
            Bytecode::JMPS as u8,
            0,
            Bytecode::P1 as u8,
            Bytecode::END as u8,
        ];
        // JMPS at 8: param at 9, dest = 9 + 0 + 1 = 10 (END)
        assert_eq!(scan_jump_dests(&codes), vec![10usize]);
    }

    #[test]
    fn scan_jump_dests_skips_pbufl_payload_before_absolute_jump() {
        let codes = vec![
            Bytecode::PBUFL as u8,
            0,
            2,
            0xAA,
            0xBB,
            Bytecode::JMPL as u8,
            0,
            10,
            Bytecode::P0 as u8,
            Bytecode::P1 as u8,
            Bytecode::END as u8,
        ];
        assert_eq!(scan_jump_dests(&codes), vec![10usize]);
    }

    #[test]
    fn bytecode_print_desc_marks_jump_after_pbuf() {
        let codes = vec![
            Bytecode::PBUF as u8,
            2,
            0xAA,
            0xBB,
            Bytecode::JMPL as u8,
            0,
            7,
            Bytecode::END as u8,
        ];
        let printed = codes.bytecode_print(true).unwrap();
        assert!(
            printed.contains("block: [7]"),
            "unexpected print:\n{}",
            printed
        );
        assert!(printed.contains("#7:"), "unexpected print:\n{}", printed);
    }

    #[test]
    fn scan_jump_dests_long_jump_uses_absolute_target() {
        let codes = vec![
            Bytecode::P0 as u8,
            Bytecode::JMPL as u8,
            0,
            5,
            Bytecode::P1 as u8,
            Bytecode::END as u8,
        ];
        assert_eq!(scan_jump_dests(&codes), vec![5usize]);
    }

    #[test]
    fn bytecode_print_desc_reports_absolute_long_jump_block() {
        let codes = vec![Bytecode::JMPL as u8, 0, 3, Bytecode::END as u8];
        let printed = codes.bytecode_print(true).unwrap();
        assert!(
            printed.contains("block: [3]"),
            "unexpected print: {}",
            printed
        );
    }

    #[test]
    fn bytecode_print_desc_covers_all_user_call_forms() {
        let selector_a = [0x01, 0x02, 0x03, 0x04];
        let selector_b = [0xaa, 0xbb, 0xcc, 0xdd];
        let cases = [
            (
                CallSpec::Invoke {
                    target: CallTarget::This,
                    effect: EffectMode::Edit,
                    selector: selector_a,
                },
                "call edit this.<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Self_,
                    effect: EffectMode::Edit,
                    selector: selector_a,
                },
                "call edit self.<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Super,
                    effect: EffectMode::Edit,
                    selector: selector_a,
                },
                "call edit super.<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Self_,
                    effect: EffectMode::View,
                    selector: selector_a,
                },
                "call view self.<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Self_,
                    effect: EffectMode::Pure,
                    selector: selector_a,
                },
                "call pure self.<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Ext(7),
                    effect: EffectMode::Edit,
                    selector: selector_a,
                },
                "call edit ext(7).<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Ext(7),
                    effect: EffectMode::View,
                    selector: selector_a,
                },
                "call view ext(7).<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Use(7),
                    effect: EffectMode::View,
                    selector: selector_a,
                },
                "call view use(7).<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Use(7),
                    effect: EffectMode::Pure,
                    selector: selector_a,
                },
                "call pure use(7).<01020304>",
            ),
            (
                CallSpec::Invoke {
                    target: CallTarget::Upper,
                    effect: EffectMode::View,
                    selector: selector_b,
                },
                "call view upper.<aabbccdd>",
            ),
            (
                CallSpec::Splice {
                    lib: 9,
                    selector: selector_b,
                },
                "codecall 9.<aabbccdd>",
            ),
        ];

        for (call, needle) in cases {
            let (inst, para) = encode_user_call_site(call);
            let mut codes = vec![inst as u8];
            codes.extend_from_slice(&para);
            codes.push(Bytecode::END as u8);
            let printed = codes.bytecode_print(true).unwrap();
            assert!(
                printed.contains(needle),
                "missing '{}' in print:\n{}",
                needle,
                printed
            );
        }
    }
}
