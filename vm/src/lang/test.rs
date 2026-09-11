#[cfg(test)]
mod token_t {

    #[test]
    fn test_char_basic() {
        let tests = vec![
            ("'A'", 65u8),
            ("'Z'", 90),
            ("'a'", 97),
            ("'z'", 122),
            ("'0'", 48),
            ("'9'", 57),
            ("' '", 32),
            ("'.'", 46),
        ];

        for (input, expected) in tests {
            let tkr = super::Tokenizer::new(input.as_bytes());
            let tokens = tkr.parse().expect(&format!("Failed to parse: {}", input));
            assert_eq!(tokens.len(), 1, "Expected 1 token for {}", input);
            match &tokens[0] {
                super::Token::Character(b) => assert_eq!(*b, expected, "Wrong value for {}", input),
                _ => panic!(
                    "Expected Character token for {}, got {:?}",
                    input, tokens[0]
                ),
            }
        }
    }

    #[test]
    fn test_char_escape_sequences() {
        let tests = vec![
            ("'\\n'", 10u8),
            ("'\\t'", 9),
            ("'\\r'", 13),
            ("'\\0'", 0),
            ("'\\b'", 8),
            ("'\\f'", 12),
            ("'\\v'", 11),
            ("'\\\\'", 92),
            ("'\\''", 39),
            ("'\\\"'", 34),
            ("'\\x41'", 65),
        ];

        for (input, expected) in tests {
            let tkr = super::Tokenizer::new(input.as_bytes());
            let tokens = tkr.parse().expect(&format!("Failed to parse: {}", input));
            assert_eq!(tokens.len(), 1, "Expected 1 token for {}", input);
            match &tokens[0] {
                super::Token::Character(b) => assert_eq!(*b, expected, "Wrong value for {}", input),
                _ => panic!(
                    "Expected Character token for {}, got {:?}",
                    input, tokens[0]
                ),
            }
        }
    }

    #[test]
    fn test_char_in_expressions() {
        let scripts = vec![
            "var c = 'A'",
            "var lower = 'a'",
            "var digit = '0'",
            "var space = ' '",
            "let c = 'Z' + 1",
            "if 'A' == 'A' { return 1 }",
        ];

        for script in scripts {
            let result = super::lang_to_irnode(script);
            assert!(result.is_ok(), "Failed to parse: {}", script);
        }
    }

    #[test]
    fn test_char_error_handling() {
        let invalid_inputs = vec![
            "'",      // Incomplete
            "''",     // Empty
            "'AB'",   // Multiple chars
            "'\\x'",  // Invalid escape
            "'\\x4'", // Incomplete hex escape
            "'\\u'",  // Unsupported unicode escape
        ];

        for input in invalid_inputs {
            let tkr = super::Tokenizer::new(input.as_bytes());
            let result = tkr.parse();
            assert!(result.is_err(), "Should fail for invalid char: {}", input);
        }
    }

    #[test]
    fn test_string_escape_sequences() {
        let tests = vec![
            ("\"\\n\"", vec![10u8]),
            ("\"\\t\"", vec![9]),
            ("\"\\r\"", vec![13]),
            ("\"\\0\"", vec![0]),
            ("\"\\b\"", vec![8]),
            ("\"\\f\"", vec![12]),
            ("\"\\v\"", vec![11]),
            ("\"\\\\\"", vec![92]),
            ("\"\\\"\"", vec![34]),
            ("\"\\x41\\x42\"", vec![65, 66]),
        ];

        for (input, expected) in tests {
            let tkr = super::Tokenizer::new(input.as_bytes());
            let tokens = tkr.parse().expect(&format!("Failed to parse: {}", input));
            match &tokens[0] {
                super::Token::Bytes(bs) => assert_eq!(bs, &expected, "Wrong value for {}", input),
                _ => panic!("Expected Bytes token for {}, got {:?}", input, tokens[0]),
            }
        }
    }

    #[test]
    fn test_string_escape_error_handling() {
        let invalid_inputs = vec!["\"\\x\"", "\"\\x4\"", "\"\\u0041\""];

        for input in invalid_inputs {
            let tkr = super::Tokenizer::new(input.as_bytes());
            let result = tkr.parse();
            assert!(
                result.is_err(),
                "Should fail for invalid string escape: {}",
                input
            );
        }
    }

    #[test]
    fn test_nested_block_comments_parse() {
        let script = r#"
            /* outer
                /* inner */
            */
            return 1
        "#;
        let result = super::lang_to_irnode(script);
        assert!(result.is_ok(), "nested block comments should parse");
    }

    #[test]
    fn test_binary_literal_too_wide_fails_cleanly() {
        let input = format!("0b{}", "1".repeat(136));
        let result = super::Tokenizer::new(input.as_bytes()).parse();
        assert!(
            result.is_err(),
            "oversized binary literal must fail cleanly"
        );
    }

    #[test]
    fn test_short_binary_literals_fail_as_binary_errors() {
        for input in ["0b", "0b1", "0b101", "0b1111111"] {
            let err = super::Tokenizer::new(input.as_bytes())
                .parse()
                .unwrap_err()
                .to_string();
            assert!(err.contains("binary data"), "input={}, err={}", input, err);
        }
    }

    #[test]
    fn test_exact_byte_binary_literal_parses() {
        let tokens = super::Tokenizer::new("0b11110000".as_bytes())
            .parse()
            .expect("binary literal should parse");
        match &tokens[0] {
            super::Token::Bytes(bs) => assert_eq!(bs, &vec![0xf0]),
            _ => panic!("expected Bytes token, got {:?}", tokens[0]),
        }
    }

    #[test]
    fn test_char_vs_string_distinction() {
        let char_tkr = super::Tokenizer::new("'A'".as_bytes());
        let char_tokens = char_tkr.parse().unwrap();

        let str_tkr = super::Tokenizer::new("\"A\"".as_bytes());
        let str_tokens = str_tkr.parse().unwrap();

        assert_ne!(
            char_tokens[0], str_tokens[0],
            "Character 'A' and string \"A\" should produce different tokens"
        );

        match &char_tokens[0] {
            super::Token::Character(b) => assert_eq!(*b, 65),
            _ => panic!("'A' should be Character"),
        }

        match &str_tokens[0] {
            super::Token::Bytes(bs) => {
                assert_eq!(bs.len(), 1);
                assert_eq!(bs[0], 65);
            }
            _ => panic!("\"A\" should be Bytes"),
        }
    }

    #[test]
    fn test_char_roundtrip() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;
        let scripts = vec![
            "var c = 'A'",
            "var tab = '\\t'",
            "let newline = '\\n'",
            "if 'x' == 'y' { return 0 }",
        ];

        for script in scripts {
            let ir = lang_to_irnode(script).expect(&format!("Failed to compile: {}", script));
            let _result = irnode_to_lang(ir).expect(&format!("Failed to decompile: {}", script));
        }
    }

    #[test]
    fn test_char_compiles_to_pu8() {
        use super::lang_to_bytecode;
        use crate::rt::Bytecode;

        let bytecode = lang_to_bytecode("var c = 'A'").expect("Failed to compile");
        let pu8_byte = Bytecode::PU8 as u8;
        assert!(
            bytecode.contains(&pu8_byte),
            "Expected PU8 (0x20) in bytecode, got: {:02x?}",
            bytecode
        );
        assert!(
            bytecode.contains(&65u8),
            "Expected value 65 in bytecode, got: {:02x?}",
            bytecode
        );
    }

    #[test]
    fn test_memory_once_compiles_to_monce_and_roundtrips() {
        use super::{irnode_to_lang, lang_to_bytecode, lang_to_irnode};
        use crate::rt::Bytecode;

        let script = "memory_once(1, 1)";
        let bytecode = lang_to_bytecode(script).expect("Failed to compile memory_once");
        assert!(
            bytecode.contains(&(Bytecode::MONCE as u8)),
            "Expected MONCE (0x94) in bytecode, got: {:02x?}",
            bytecode
        );
        let back = irnode_to_lang(lang_to_irnode(script).unwrap()).unwrap();
        assert!(
            back.contains("memory_once"),
            "decompiled output lost builtin name: {}",
            back
        );

        use crate::rt::GasTable;
        let gst = GasTable::new(1);
        assert_eq!(
            gst.gas(Bytecode::MONCE as u8),
            gst.gas(Bytecode::MTAKE as u8),
            "MONCE base gas must match MPUT"
        );
        assert!(
            gst.gas(Bytecode::MONCE as u8) > 1,
            "MONCE must not fall back to the default base gas"
        );
    }

    #[test]
    fn test_char_escapes_compile_correctly() {
        use super::lang_to_bytecode;

        let bc_newline = lang_to_bytecode("var n = '\\n'").expect("Failed to compile newline");
        assert!(
            bc_newline.contains(&10u8),
            "Expected value 10, got: {:02x?}",
            bc_newline
        );

        let bc_tab = lang_to_bytecode("var t = '\\t'").expect("Failed to compile tab");
        assert!(
            bc_tab.contains(&9u8),
            "Expected value 9, got: {:02x?}",
            bc_tab
        );
    }

    #[test]
    fn test_not_operator_binds_tighter_than_binary_ops() {
        use super::lang_to_irnode;

        assert!(lang_to_irnode("return !1 == 0").is_ok());
        assert!(lang_to_irnode("return !1 && 0 == 0").is_ok());
        assert!(lang_to_irnode("return !1 is nil").is_ok());
    }

    #[test]
    fn test_codecall_double_colon_rejected() {
        use super::lang_to_irnode;
        let script = "codecall 1::0x01020304";
        let result = lang_to_irnode(script);
        assert!(result.is_err(), "codecall :: must be rejected");
    }

    #[test]
    fn test_codecall_dot_accepted() {
        use super::lang_to_irnode;
        let script = "codecall 1.0x01020304";
        let result = lang_to_irnode(script);
        assert!(result.is_ok(), "codecall . must be accepted");
    }

    #[test]
    fn test_codecall_dot_with_argument_accepted() {
        use super::lang_to_irnode;
        let script = "codecall 1.0x01020304(7)";
        let result = lang_to_irnode(script);
        assert!(result.is_ok(), "codecall . with argument must be accepted");
    }

    #[test]
    fn test_nested_call_arg_subexpression_on_binary_rhs_compiles() {
        use super::lang_to_irnode;
        let script = r#"
            return 1 + byte("abc", 3 - 1)
        "#;
        let result = lang_to_irnode(script);
        assert!(
            result.is_ok(),
            "nested call arg with subtraction on binary rhs must compile"
        );
    }

    #[test]
    fn test_nested_multi_arg_call_subexpressions_on_binary_rhs_compile() {
        use super::lang_to_irnode;
        let script = r#"
            return 1 + size(buf_cut("abcd", 1 + 1, 4 - 2 - 1))
        "#;
        let result = lang_to_irnode(script);
        assert!(
            result.is_ok(),
            "nested multi-arg call subexpressions on binary rhs must compile"
        );
    }

    #[test]
    fn test_codecall_all_required_forms_are_valid() {
        use super::lang_to_irnode;
        let scripts = [
            r#"
                lib C = 1
                codecall C.f
            "#,
            r#"
                lib C = 1
                codecall C.f()
            "#,
            r#"
                lib C = 1
                codecall C.f(nil)
            "#,
            r#"
                lib C = 1
                let a = 1
                codecall C.f (a)
            "#,
            r#"
                lib C = 1
                let a = 1
                let b = 2
                codecall C.f(a, b)
            "#,
        ];
        for script in scripts {
            let result = lang_to_irnode(script);
            assert!(
                result.is_ok(),
                "codecall form must be valid: {script} -> {:?}",
                result.err()
            );
        }
    }

    #[test]
    fn test_codecall_first_three_forms_are_equivalent() {
        use super::lang_to_bytecode;

        let s1 = r#"
            lib C = 1
            codecall C.f
        "#;
        let s2 = r#"
            lib C = 1
            codecall C.f()
        "#;
        let s3 = r#"
            lib C = 1
            codecall C.f(nil)
        "#;

        let b1 = lang_to_bytecode(s1).expect("compile codecall C.f failed");
        let b2 = lang_to_bytecode(s2).expect("compile codecall C.f() failed");
        let b3 = lang_to_bytecode(s3).expect("compile codecall C.f(nil) failed");

        assert_eq!(b1, b2, "codecall C.f and codecall C.f() must be equivalent");
        assert_eq!(
            b1, b3,
            "codecall C.f and codecall C.f(nil) must be equivalent"
        );
    }

    #[test]
    fn test_codecall_with_argument_emits_argument_push_before_opcode() {
        use super::lang_to_bytecode;
        use crate::rt::{Bytecode, verify_bytecodes};

        let codes = lang_to_bytecode("codecall 1.0x01020304(7)").expect("compile failed");
        let marks = verify_bytecodes(&codes).expect("verify failed");
        let mut codecall_idx = None;
        for (idx, mark) in marks.iter().enumerate() {
            if *mark == 0 {
                continue;
            }
            if codes[idx] == Bytecode::CODECALL as u8 {
                codecall_idx = Some(idx);
                break;
            }
        }
        let idx = codecall_idx.expect("must contain CODECALL opcode");
        assert!(idx > 0, "CODECALL must not be the first instruction");
        let prev_inst_idx = marks
            .iter()
            .enumerate()
            .take(idx)
            .filter_map(|(i, m)| (*m != 0).then_some(i))
            .last()
            .expect("CODECALL must have a previous instruction");
        assert_ne!(
            codes[prev_inst_idx],
            Bytecode::PNIL as u8,
            "codecall(expr) must not fallback to implicit nil argument"
        );
    }

    #[test]
    fn test_decompile_call_is_canonical_generic_form() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;
        let ir = lang_to_irnode("this.0x01020304()").expect("compile shortcut call failed");
        let decompiled = irnode_to_lang(ir).expect("decompile failed");
        assert!(
            decompiled.contains("call edit this.0x01020304("),
            "decompiled call must be canonical generic form, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_decompile_codecall_shows_argument_expression() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;
        let script = r#"
            lib C = 1
            codecall C.0x01020304(7)
        "#;
        let ir = lang_to_irnode(script).expect("compile codecall failed");
        let decompiled = irnode_to_lang(ir).expect("decompile failed");
        assert!(
            decompiled.contains("codecall ext(1).0x01020304(7)"),
            "decompiled codecall must include argument expression, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_generic_call_maps_to_short_opcode() {
        use super::lang_to_bytecode;
        use crate::rt::Bytecode;
        let bytecode =
            lang_to_bytecode("call edit this.0x01020304()").expect("compile generic call failed");
        assert!(
            bytecode.contains(&(Bytecode::CALLTHIS as u8)),
            "generic call should map to CALLTHIS opcode, got: {:02x?}",
            bytecode
        );
    }

    #[test]
    fn test_canonical_call_syntax_covers_all_invoke_combinations() {
        use super::lang_to_irnode;
        let effects = ["edit", "view", "pure"];
        let targets = ["this", "self", "upper", "super", "ext(1)", "use(1)"];
        for effect in effects {
            for target in targets {
                let script = format!("call {} {}.0x01020304()", effect, target);
                let result = lang_to_irnode(&script);
                assert!(
                    result.is_ok(),
                    "canonical call must compile: {} -> {:?}",
                    script,
                    result.err()
                );
            }
        }
    }

    // ==================== BUG 2: Empty Map Roundtrip Test ====================

    #[test]
    fn test_empty_map_roundtrip() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;

        let original = "map { }";
        let ir = lang_to_irnode(original).expect("Failed to compile empty map");
        let decompiled = irnode_to_lang(ir).expect("Failed to decompile");

        let ir2 = lang_to_irnode(&decompiled).expect("Failed to recompile decompiled map");
        let decompiled2 = irnode_to_lang(ir2).expect("Failed to decompile again");

        assert_eq!(
            decompiled, decompiled2,
            "Empty map roundtrip should be stable.\nOriginal: {}\nFirst: {}\nSecond: {}",
            original, decompiled, decompiled2
        );
    }

    #[test]
    fn test_empty_map_vs_nonempty_map_structure() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;
        use crate::ir::IRNode;

        let empty_ir = lang_to_irnode("map { }").expect("Failed to compile empty map");
        let empty_serialized = empty_ir.serialize();

        let nonempty_ir =
            lang_to_irnode("map { \"k\": \"v\" }").expect("Failed to compile non-empty map");

        let empty_decompiled = irnode_to_lang(empty_ir).expect("Failed to decompile empty map");
        println!("Empty map decompiled: {:?}", empty_decompiled);

        let nonempty_decompiled =
            irnode_to_lang(nonempty_ir).expect("Failed to decompile non-empty map");
        println!("Non-empty map decompiled: {:?}", nonempty_decompiled);

        let ir_after = lang_to_irnode(&empty_decompiled).expect("Failed to compile decompiled");
        let after_serialized = ir_after.serialize();

        println!("Empty map serialized: {:02x?}", empty_serialized);
        println!("After roundtrip serialized: {:02x?}", after_serialized);

        assert_eq!(
            empty_serialized, after_serialized,
            "Empty map should be stable after roundtrip"
        );
    }

    #[test]
    fn test_empty_list_vs_empty_map_comparison() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;
        use crate::ir::IRNode;

        let list_original = "list { }";
        let list_ir = lang_to_irnode(list_original).expect("Failed to compile empty list");
        let list_serialized = list_ir.serialize();
        let list_decompiled = irnode_to_lang(list_ir).expect("Failed to decompile empty list");
        let list_ir2 = lang_to_irnode(&list_decompiled).expect("Failed to recompile");
        let list_decompiled2 = irnode_to_lang(list_ir2).expect("Failed to decompile again");

        let map_original = "map { }";
        let map_ir = lang_to_irnode(map_original).expect("Failed to compile empty map");
        let map_serialized = map_ir.serialize();
        let map_decompiled = irnode_to_lang(map_ir).expect("Failed to decompile empty map");
        let map_ir2 = lang_to_irnode(&map_decompiled).expect("Failed to recompile");
        let map_decompiled2 = irnode_to_lang(map_ir2).expect("Failed to decompile again");

        assert_eq!(
            list_decompiled, list_decompiled2,
            "Empty list should be stable"
        );

        assert_eq!(
            map_decompiled, map_decompiled2,
            "Empty map should be stable"
        );

        println!("Empty list serialized: {:?}", list_serialized);
        println!("Empty map serialized: {:?}", map_serialized);
    }

    // ==================== Bug 5: Tokenizer Number Parsing Test ====================

    #[test]
    fn test_number_with_letters_should_fail() {
        // Digits mixed with letters should fail
        let result = super::Tokenizer::new("123abc".as_bytes()).parse();
        assert!(result.is_err(), "123abc should fail to parse as number");
    }

    #[test]
    fn test_underscore_in_numbers() {
        // Underscore-separated digits should parse
        let result = super::Tokenizer::new("1000_000".as_bytes()).parse();
        match result {
            Ok(tokens) => {
                println!("1000_000 tokens: {:?}", tokens);
                if let Some(super::Token::Integer(n)) = tokens.first() {
                    assert_eq!(*n, 1000000, "1000_000 should parse as 1000000");
                }
            }
            Err(e) => {
                panic!("1000_000 should parse successfully, got error: {}", e);
            }
        }
    }

    #[test]
    fn test_codecall_without_source_end_is_valid() {
        use super::lang_to_irnode;

        let script = r#"
            lib C = 0
            codecall C.probe
        "#;
        let result = lang_to_irnode(script);
        assert!(
            result.is_ok(),
            "codecall without source-level end must be valid"
        );
    }

    #[test]
    fn test_codecall_with_redundant_source_end_is_valid() {
        use super::lang_to_irnode;

        let script = r#"
            lib C = 0
            codecall C.probe
            end
        "#;
        let result = lang_to_irnode(script);
        assert!(
            result.is_ok(),
            "redundant source-level end after codecall must be valid"
        );
    }

    fn collect_user_call_opcodes(codes: &[u8]) -> Vec<u8> {
        use crate::rt::{Bytecode, verify_bytecodes};

        verify_bytecodes(codes)
            .unwrap()
            .into_iter()
            .enumerate()
            .filter_map(|(idx, mark)| {
                if mark == 0 {
                    return None;
                }
                let op = codes[idx];
                let is_call = matches!(
                    op,
                    x if x == Bytecode::CODECALL as u8
                        || x == Bytecode::CALL as u8
                        || x == Bytecode::CALLEXT as u8
                        || x == Bytecode::CALLEXTVIEW as u8
                        || x == Bytecode::CALLUSEVIEW as u8
                        || x == Bytecode::CALLUSEPURE as u8
                        || x == Bytecode::CALLTHIS as u8
                        || x == Bytecode::CALLSELF as u8
                        || x == Bytecode::CALLSUPER as u8
                        || x == Bytecode::CALLSELFVIEW as u8
                        || x == Bytecode::CALLSELFPURE as u8
                );
                is_call.then_some(op)
            })
            .collect()
    }

    #[test]
    fn test_generic_call_keyword_uses_shortcut_opcode_for_self_edit() {
        use super::lang_to_bytecode;
        use crate::rt::Bytecode;

        let codes = lang_to_bytecode("return call edit self.0x01020304(1)").unwrap();
        assert_eq!(
            collect_user_call_opcodes(&codes),
            vec![Bytecode::CALLSELF as u8]
        );
    }

    #[test]
    fn test_generic_call_keyword_uses_shortcut_opcode_for_use_view() {
        use super::lang_to_bytecode;
        use crate::rt::Bytecode;

        let codes = lang_to_bytecode("return call view use(1).0x01020304(1)").unwrap();
        assert_eq!(
            collect_user_call_opcodes(&codes),
            vec![Bytecode::CALLUSEVIEW as u8]
        );
    }

    #[test]
    fn test_generic_call_keyword_uses_shortcut_opcode_for_use_pure() {
        use super::lang_to_bytecode;
        use crate::rt::Bytecode;

        let codes = lang_to_bytecode("return call pure use(1).0x01020304(1)").unwrap();
        assert_eq!(
            collect_user_call_opcodes(&codes),
            vec![Bytecode::CALLUSEPURE as u8]
        );
    }

    #[test]
    fn test_generic_call_keyword_keeps_generic_opcode_without_shortcut() {
        use super::lang_to_bytecode;
        use crate::rt::Bytecode;

        let codes = lang_to_bytecode("return call view upper.0x01020304(1)").unwrap();
        assert_eq!(
            collect_user_call_opcodes(&codes),
            vec![Bytecode::CALL as u8]
        );
    }

    // ==================== Number Type Suffix Tests ====================

    #[test]
    fn test_number_with_type_suffix() {
        use super::lang_to_irnode;

        // Test 100u64 should parse as 100 as u64
        let scripts = vec!["100u64", "100u32", "100u16", "100u8", "100u128"];

        for script in scripts {
            let result = lang_to_irnode(script);
            match result {
                Ok(_) => println!("OK: {} parsed successfully", script),
                Err(e) => println!("FAIL: {} failed: {}", script, e),
            }
        }
    }

    // ==================== Overflow Check Tests ====================

    #[test]
    fn test_number_overflow_check() {
        use super::lang_to_irnode;

        // 300 overflows u8 (max: 255)
        let result = lang_to_irnode("300u8");
        assert!(result.is_err(), "300u8 should overflow u8");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("overflows"),
            "Error should mention overflow: {}",
            err_msg
        );

        // 70000 overflows u16 (max: 65535)
        let result = lang_to_irnode("70000u16");
        assert!(result.is_err(), "70000u16 should overflow u16");

        // 100u64 should be valid
        let result = lang_to_irnode("100u64");
        assert!(result.is_ok(), "100u64 should be valid");
    }

    #[test]
    fn test_as_cast_overflow_check_for_uint_literals() {
        use super::lang_to_irnode;

        let result = lang_to_irnode("300 as u8");
        assert!(result.is_err(), "300 as u8 should overflow u8");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("overflows u8"),
            "Error should mention u8 overflow: {}",
            err_msg
        );

        let result = lang_to_irnode("70000 as u16");
        assert!(result.is_err(), "70000 as u16 should overflow u16");

        let result = lang_to_irnode("70000 as u32");
        assert!(result.is_ok(), "70000 as u32 should be valid");
    }

    #[test]
    fn test_as_cast_address_literal_compile_time_check() {
        use super::lang_to_irnode;

        let result = lang_to_irnode("0xABCD as address");
        assert!(
            result.is_err(),
            "invalid bytes literal cast to address should fail at compile time"
        );

        let result = lang_to_irnode("emqjNS9PscqdBpMtnC3Jfuc4mvZUPYTPS as address");
        assert!(result.is_ok(), "address literal as address should be valid");
    }

    #[test]
    fn test_number_with_suffix_allows_underscore() {
        use super::lang_to_irnode;

        assert!(lang_to_irnode("1000_u64").is_ok());
        assert!(lang_to_irnode("1_000u64").is_ok());
    }

    #[test]
    fn test_suffix_requires_tokenizer_recognition_without_whitespace() {
        use super::lang_to_irnode;

        assert!(lang_to_irnode("100u64").is_ok());
        assert!(lang_to_irnode("1000_u64").is_ok());
        assert!(lang_to_irnode("100 u64").is_err());
        assert!(lang_to_irnode("100\nu64").is_err());
    }

    // ==================== Simplify Numeric As Suffix Test ====================

    #[test]
    fn test_simplify_numeric_as_suffix_decompilation() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;

        // Test that 100u64 decompiles to "100u64" instead of "100 as u64"
        let script = "var x = 100u64";
        let ir = lang_to_irnode(script).expect("Failed to compile");
        let decompiled = irnode_to_lang(ir).expect("Failed to decompile");

        // Should contain "100u64" not "100 as u64"
        assert!(
            decompiled.contains("100u64"),
            "Decompiled should contain '100u64', got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains(" as u64"),
            "Decompiled should NOT contain ' as u64', got: {}",
            decompiled
        );
    }

    #[test]
    fn test_all_numeric_suffixes_decompilation() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;

        let test_cases = [
            ("100u8", "100u8"),
            ("100u16", "100u16"),
            ("100u32", "100u32"),
            ("100u64", "100u64"),
            ("100u128", "100u128"),
            ("70000u64", "70000u64"),
        ];

        for (input, expected) in test_cases {
            let script = format!("var x = {}", input);
            let ir = lang_to_irnode(&script).expect(&format!("Failed to compile: {}", input));
            let decompiled = irnode_to_lang(ir).expect(&format!("Failed to decompile: {}", input));

            assert!(
                decompiled.contains(expected),
                "Expected '{}' in decompiled output, got: {}",
                expected,
                decompiled
            );
        }
    }

    #[test]
    fn test_variable_type_cast_not_suffix() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;

        // Test that variable as u8 is NOT converted to numu8
        let script = r#"
            var num = 100
            var x = num as u8
        "#;
        let ir = lang_to_irnode(script).expect("Failed to compile");
        let decompiled = irnode_to_lang(ir).expect("Failed to decompile");

        // Should contain "num as u8" NOT "numu8"
        assert!(
            decompiled.contains(" as u8"),
            "Variable cast should use 'as u8', got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains("numu8"),
            "Should NOT contain 'numu8', got: {}",
            decompiled
        );
    }

    #[test]
    fn test_bool_cast_keeps_as_form_not_suffix() {
        use super::irnode_to_lang;
        use super::lang_to_irnode;

        let script = "var x = true as u8";
        let ir = lang_to_irnode(script).expect("Failed to compile");
        let decompiled = irnode_to_lang(ir).expect("Failed to decompile");

        assert!(
            decompiled.contains("true as u8"),
            "Bool cast should keep 'as u8', got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains("trueu8"),
            "Bool cast must not use numeric suffix, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_simplify_numeric_as_suffix_option_off_uses_as_cast() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let script = "var x = 100u64";
        let ir = lang_to_irnode(script).expect("Failed to compile");

        let mut opt = PrintOption::new("  ", 0);
        opt.recover_literals = true;
        opt.simplify_numeric_as_suffix = false;
        let decompiled = Formater::new(&opt).print(&ir);

        assert!(
            decompiled.contains("100 as u64"),
            "Expected '100 as u64' when simplify_numeric_as_suffix=false, got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains("100u64"),
            "Should NOT contain '100u64' when simplify_numeric_as_suffix=false, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_hide_default_call_argv_keeps_explicit_empty_bytes_for_ntfunc() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return sha2(\"\")").expect("Failed to compile");
        let mut opt = PrintOption::new("  ", 0);
        opt.hide_default_call_argv = true;
        let decompiled = Formater::new(&opt).print(&ir);
        assert!(
            decompiled.contains("sha2(\"\")"),
            "NTFUNC explicit empty bytes arg must be preserved, got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains("sha2()"),
            "NTFUNC arg must not be hidden as zero-arg, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_keccak256_roundtrips_as_single_arg_ntfunc() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return keccak256(\"abc\")").expect("Failed to compile");
        let decompiled = Formater::new(&PrintOption::new("  ", 0)).print(&ir);
        assert!(
            decompiled.contains("keccak256(") && decompiled.contains("0x616263"),
            "expected keccak256 call to survive decompile with canonical bytes, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_blake2s256_roundtrips_as_single_arg_ntfunc() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return blake2s256(\"abc\")").expect("Failed to compile");
        let decompiled = Formater::new(&PrintOption::new("  ", 0)).print(&ir);
        assert!(
            decompiled.contains("blake2s256(") && decompiled.contains("0x616263"),
            "expected blake2s256 call to survive decompile with canonical bytes, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_blake2b256_roundtrips_as_single_arg_ntfunc() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return blake2b256(\"abc\")").expect("Failed to compile");
        let decompiled = Formater::new(&PrintOption::new("  ", 0)).print(&ir);
        assert!(
            decompiled.contains("blake2b256(") && decompiled.contains("0x616263"),
            "expected blake2b256 call to survive decompile with canonical bytes, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_hide_default_call_argv_applies_to_ntenv() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return context_address()").expect("Failed to compile");
        let plain = Formater::new(&PrintOption::new("  ", 0)).print(&ir);
        assert!(
            plain.contains("context_address()"),
            "NTENV should stay zero-arg without placeholder, got: {}",
            plain
        );
        let mut opt = PrintOption::new("  ", 0);
        opt.hide_default_call_argv = true;
        let decompiled = Formater::new(&opt).print(&ir);
        assert!(
            decompiled.contains("context_address()"),
            "NTENV should stay zero-arg when hide option enabled, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_hide_default_call_argv_applies_to_ntreg() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("intent_pop()\nreturn 0").expect("Failed to compile");
        let plain = Formater::new(&PrintOption::new("  ", 0)).print(&ir);
        assert!(
            plain.contains("intent_pop()"),
            "NTREG should stay zero-arg without placeholder, got: {}",
            plain
        );
        let mut opt = PrintOption::new("  ", 0);
        opt.hide_default_call_argv = true;
        let decompiled = Formater::new(&opt).print(&ir);
        assert!(
            decompiled.contains("intent_pop()"),
            "NTREG should stay zero-arg when hide option enabled, got: {}",
            decompiled
        );
    }

    #[test]
    fn test_syscall_single_arg_cat_not_split() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return sha2(\"a\" ++ \"b\")").expect("Failed to compile");
        let mut opt = PrintOption::new("  ", 0);
        opt.recover_literals = true;
        opt.flatten_syscall_cat = true;
        let decompiled = Formater::new(&opt).print(&ir);
        assert!(
            decompiled.contains("sha2(") && decompiled.contains("++"),
            "single-arg syscall CAT expression must stay single arg, got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains("sha2(\"a\", \"b\")"),
            "single-arg syscall must not be split into multi args, got: {}",
            decompiled
        );
    }

    #[test]
    fn pack_asset_compiles_as_packed_tuple_without_uint_coercion() {
        use super::lang_to_bytecode;
        use crate::rt::Bytecode;
        use crate::rt::NativeArgvPack;
        use crate::rt::NativeFunc;

        assert_eq!(NativeFunc::pack_asset as u8, 82);
        assert_eq!(
            NativeFunc::argv_pack(NativeFunc::pack_asset as u8).unwrap(),
            NativeArgvPack::Packed
        );
        lang_to_bytecode("return pack_asset(1, 2)").expect("compile");

        let codes = lang_to_bytecode("return pack_asset(1, 100)").expect("compile");
        let cu64_count = codes.iter().filter(|&&b| b == Bytecode::CU64 as u8).count();
        assert_eq!(cu64_count, 0, "Packed uints stay native width, got CU64");
        assert!(
            codes.contains(&(Bytecode::PACKTUPLE as u8)),
            "arity-2 Packed must emit PACKTUPLE"
        );
        assert!(
            !codes.contains(&(Bytecode::CAT as u8)),
            "Packed pack_asset must not CAT"
        );
    }

    #[test]
    fn ascii_u128_dec_unit_compiles_as_packed_three_args() {
        use super::lang_to_bytecode;
        use super::lang_to_irnode;
        use crate::rt::Bytecode;

        lang_to_bytecode(r#"return ascii_u128_dec_unit(2, 0, "21k")"#).expect("compile");
        assert!(
            lang_to_irnode(r#"return ascii_u128_dec_unit(2, "21k")"#).is_err(),
            "old 2-arg Concat form must not compile"
        );
        let codes = lang_to_bytecode(r#"return ascii_u128_dec_unit(2, 0, "21k")"#).unwrap();
        assert!(
            codes.contains(&(Bytecode::PACKTUPLE as u8)),
            "arity-3 Packed must emit PACKTUPLE"
        );
        assert!(
            !codes.contains(&(Bytecode::CAT as u8)),
            "Packed ascii_u128_dec_unit must not CAT"
        );
    }

    #[test]
    fn pack_asset_variable_and_wide_literal_still_compile() {
        use super::lang_to_bytecode;
        use super::lang_to_irnode;

        lang_to_bytecode("var s = 1\nreturn pack_asset(s, 100)").expect("compile");
        lang_to_irnode(&format!("return pack_asset({}, 1)", u64::MAX as u128 + 1))
            .expect("wide literal compiles; range is checked at runtime");
    }

    #[test]
    fn test_syscall_multi_arg_packed_decompiles_as_two_args() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return pack_asset(1, 2)").expect("Failed to compile");

        let mut opt_false = PrintOption::new("  ", 0);
        opt_false.flatten_syscall_cat = false;
        let decompiled_false = Formater::new(&opt_false).print(&ir);

        let mut opt_true = PrintOption::new("  ", 0);
        opt_true.flatten_syscall_cat = true;
        let decompiled_true = Formater::new(&opt_true).print(&ir);

        for out in [&decompiled_false, &decompiled_true] {
            assert!(
                out.contains("pack_asset(") && out.contains(","),
                "multi-arg syscall must decompile as multi args, got: {}",
                out
            );
            assert!(
                !out.contains("++"),
                "multi-arg syscall must not stay CAT expression, got: {}",
                out
            );
        }
    }

    #[test]
    fn direct_ir_func_params_accept_only_immediate_u8_literals() {
        use super::lang_to_ircode;

        for src in [
            r#"return buf_left(1, "abcdef")"#,
            r#"return buf_left((1), "abcdef")"#,
            r#"const N = 1
               return buf_left(N, "abcdef")"#,
            r#"return heap_read_uint_long(1, 2)"#,
        ] {
            lang_to_ircode(src).expect(src);
        }

        for src in [
            r#"return buf_left(context_address(), "abcdef")"#,
            r#"var n $5 = 2
               return buf_left($5, "abcdef")"#,
            r#"return heap_read_uint_long(258, 0)"#,
        ] {
            let err = lang_to_ircode(src).unwrap_err().to_string();
            assert!(
                err.contains("immediate u8 literal"),
                "src={}, err={}",
                src,
                err
            );
        }
    }

    #[test]
    fn is_expression_decompile_recompile_preserves_ircode() {
        use super::ircode_to_lang;
        use super::lang_to_ircode;

        for src in [
            "return (1 == 1) is bool",
            "return (1 == 1) is not bool",
            "return (1 + 2) is u64",
            "return (! true) is bool",
            "return (1 | 2) is u64",
            "return (true && false) is nil",
        ] {
            let expect = lang_to_ircode(src).expect(src);
            let text = ircode_to_lang(&expect).expect(src);
            let reparsed = lang_to_ircode(&text)
                .map_err(|e| {
                    format!(
                        "{}\n---- decompiled ----\n{}\n--------------------",
                        e, text
                    )
                })
                .unwrap();
            assert_eq!(expect, reparsed, "src={}\ndecompiled={}", src, text);
        }
    }

    #[test]
    fn postfix_lhs_decompile_recompile_preserves_not_operands() {
        use super::ircode_to_lang;
        use super::lang_to_ircode;

        for src in ["return (! true) as u8", "return (! true)[0]"] {
            let expect = lang_to_ircode(src).expect(src);
            let text = ircode_to_lang(&expect).expect(src);
            let reparsed = lang_to_ircode(&text)
                .map_err(|e| {
                    format!(
                        "{}\n---- decompiled ----\n{}\n--------------------",
                        e, text
                    )
                })
                .unwrap();
            assert_eq!(expect, reparsed, "src={}\ndecompiled={}", src, text);
        }
    }

    #[test]
    fn bytecode_pbuf_decompile_recompile_preserves_ircode() {
        use super::ircode_to_lang;
        use super::lang_to_ircode;

        let src = "bytecode { PBUF 3 97 98 99 }\nend";
        let expect = lang_to_ircode(src).expect(src);
        let text = ircode_to_lang(&expect).expect(src);
        let reparsed = lang_to_ircode(&text)
            .map_err(|e| {
                format!(
                    "{}\n---- decompiled ----\n{}\n--------------------",
                    e, text
                )
            })
            .unwrap();
        assert_eq!(expect, reparsed, "decompiled={}", text);
    }

    #[test]
    fn test_all_print_options_disabled_preserve_ircode_semantics() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_ircode;
        use super::lang_to_irnode_with_sourcemap;

        let script = r#"
            param { amt }
            lib C = 1
            print 1 as u64
            ext(1).0xabcdef01()
            print [1, 2]
            if true { print 3 } else { print 4 }
            while false { print 5 }
            codecall C.probe
        "#;

        let expect = lang_to_ircode(script).expect("compile baseline");
        let (block, smap) = lang_to_irnode_with_sourcemap(script).expect("compile with sourcemap");

        for map_enabled in [false, true] {
            let mut opt = PrintOption::new("  ", 0);
            if map_enabled {
                opt.map = Some(&smap);
            }
            opt.emit_lib_prelude = false;
            opt.trim_root_block = false;
            opt.trim_head_alloc = false;
            opt.trim_param_unpack = false;
            opt.hide_default_call_argv = false;
            opt.call_short_syntax = false;
            opt.flatten_call_list = false;
            opt.flatten_array_list = false;
            opt.flatten_syscall_cat = false;
            opt.recover_literals = false;
            opt.simplify_numeric_as_suffix = false;

            let text = Formater::new(&opt).print(&block);
            let reparsed = lang_to_ircode(&text)
                .map_err(|e| format!("{}\n---- all-off printed (map_enabled={}) ----\n{}\n---------------------\n", e, map_enabled, text))
                .unwrap();
            assert_eq!(
                expect, reparsed,
                "all-off roundtrip mismatch (map_enabled={})\n{}",
                map_enabled, text
            );
        }
    }

    #[test]
    fn patches_list_literal_roundtrips_without_flattening_concat_args() {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode("return patches([0, 1, 1 as u8])").expect("compile patches");
        let mut opt = PrintOption::new("  ", 0);
        opt.recover_literals = true;
        opt.flatten_array_list = true;
        opt.simplify_numeric_as_suffix = true;
        let decompiled = Formater::new(&opt).print(&ir);
        assert!(
            decompiled.contains("patches(["),
            "Packed patches must print the list as one argument, got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains("patches(0,"),
            "Packed list must not flatten into Concat CAT args, got: {}",
            decompiled
        );
        let ir2 = lang_to_irnode(&decompiled).expect("recompile decompiled patches");
        let decompiled2 = Formater::new(&opt).print(&ir2);
        assert!(
            decompiled2.contains("patches(["),
            "roundtrip must keep packed list argv, got: {}",
            decompiled2
        );
    }

    fn ir_contains_cat(node: &dyn super::IRNode) -> bool {
        use crate::ir::{
            IRNodeArray, IRNodeDouble, IRNodeParam1Single, IRNodeSingle, IRNodeWrapOne,
        };
        use crate::rt::Bytecode;
        if node.bytecode() == Bytecode::CAT as u8 {
            return true;
        }
        if let Some(arr) = node.as_any().downcast_ref::<IRNodeArray>() {
            return arr.subs.iter().any(|n| ir_contains_cat(n.as_ref()));
        }
        if let Some(d) = node.as_any().downcast_ref::<IRNodeDouble>() {
            return ir_contains_cat(d.subx.as_ref()) || ir_contains_cat(d.suby.as_ref());
        }
        if let Some(s) = node.as_any().downcast_ref::<IRNodeSingle>() {
            return ir_contains_cat(s.subx.as_ref());
        }
        if let Some(p) = node.as_any().downcast_ref::<IRNodeParam1Single>() {
            return ir_contains_cat(p.subx.as_ref());
        }
        if let Some(w) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return ir_contains_cat(w.node.as_ref());
        }
        false
    }

    fn print_sigset_call(src: &str) -> String {
        use super::Formater;
        use super::PrintOption;
        use super::lang_to_irnode;

        let ir = lang_to_irnode(src).expect(src);
        let mut opt = PrintOption::new("  ", 0);
        opt.recover_literals = true;
        opt.flatten_array_list = true;
        opt.flatten_syscall_cat = true;
        opt.simplify_numeric_as_suffix = true;
        Formater::new(&opt).print(&ir)
    }

    #[test]
    fn sigset_at_least_two_arg_compiles_with_cat_and_decompiles_as_two_args() {
        use super::irnode_to_lang;
        use super::lang_to_bytecode;
        use super::lang_to_irnode;
        use crate::rt::Bytecode;

        let src = r#"return sigset_at_least("ab", 1)"#;
        let ir = lang_to_irnode(src).expect("2-arg sigset_at_least must compile");
        let codes = lang_to_bytecode(src).expect("2-arg sigset_at_least bytecode");
        assert!(
            ir_contains_cat(&ir) && codes.contains(&(Bytecode::CAT as u8)),
            "2-arg sigset_at_least must emit CAT"
        );

        let decompiled = print_sigset_call(src);
        assert!(
            decompiled.contains(r#"sigset_at_least("ab", 1)"#)
                || (decompiled.contains("sigset_at_least(")
                    && decompiled.contains(',')
                    && !decompiled.contains("sigset_at_least([")),
            "2-arg decompile must stay two args, got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains(r#"sigset_at_least(["ab", 1])"#),
            "2-arg decompile must not collapse into a list, got: {}",
            decompiled
        );

        let via_helper = irnode_to_lang(lang_to_irnode(src).unwrap()).unwrap();
        assert!(
            via_helper.contains("sigset_at_least(") && via_helper.contains(','),
            "irnode_to_lang 2-arg must stay two args, got: {}",
            via_helper
        );
    }

    #[test]
    fn sigset_at_least_one_arg_list_compiles_without_cat_and_decompiles_as_list() {
        use super::irnode_to_lang;
        use super::lang_to_bytecode;
        use super::lang_to_irnode;
        use crate::rt::Bytecode;

        let src = r#"return sigset_at_least(["ab", 1])"#;
        let ir = lang_to_irnode(src).expect("1-arg sigset_at_least must compile");
        let codes = lang_to_bytecode(src).expect("1-arg sigset_at_least bytecode");
        assert!(
            !ir_contains_cat(&ir) && !codes.contains(&(Bytecode::CAT as u8)),
            "1-arg raw body must not emit CAT"
        );

        let decompiled = print_sigset_call(src);
        assert!(
            decompiled.contains("sigset_at_least([") || decompiled.contains("sigset_at_least(list"),
            "1-arg decompile must be a single list arg, got: {}",
            decompiled
        );
        assert!(
            !decompiled.contains(r#"sigset_at_least("ab", 1)"#),
            "1-arg list must not flatten into two CAT args, got: {}",
            decompiled
        );

        let via_helper = irnode_to_lang(lang_to_irnode(src).unwrap()).unwrap();
        assert!(
            !via_helper.contains(r#"sigset_at_least("ab", 1)"#),
            "irnode_to_lang 1-arg must not flatten, got: {}",
            via_helper
        );
    }

    #[test]
    fn sigset_count_list_compiles_without_cat() {
        use super::lang_to_bytecode;
        use super::lang_to_irnode;
        use crate::rt::Bytecode;

        let src = r#"return sigset_count(["a", "b"])"#;
        let ir = lang_to_irnode(src).expect("sigset_count list must compile");
        let codes = lang_to_bytecode(src).expect("sigset_count list bytecode");
        assert!(
            !ir_contains_cat(&ir) && !codes.contains(&(Bytecode::CAT as u8)),
            "sigset_count([c0, c1]) must not emit CAT"
        );
    }

    #[test]
    fn sigset_at_least_arity_zero_and_three_still_fail() {
        use super::lang_to_irnode;

        assert!(
            lang_to_irnode("return sigset_at_least()").is_err(),
            "zero-arg sigset_at_least must fail"
        );
        assert!(
            lang_to_irnode(r#"return sigset_at_least("a", "b", "c")"#).is_err(),
            "three-arg sigset_at_least must fail"
        );
    }
}
