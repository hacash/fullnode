#![allow(unused_imports)]

use super::super::*;
use crate::calc::decode::{BrKind, ImmAluOp, Inst, LoadOp, RegAluOp, ShiftKind, StoreOp};
use crate::calc::{CalcFnObj, CalcRunLimits};
use crate::fitshc::compile_body;
use crate::lang::Tokenizer;

use super::helpers::{N, compile, hvm_per_iter_gas, lower, run_e2e, run_e2e_mode, run_input, rv_exec};


    #[test]
    fn rejects_outside_subset() {
        // task 3: `as u128` lowers now (a (lo, hi) pair); the kernel-subset
        // reject moved to the u128 faces the pair model cannot express
        // (mixed-width arithmetic — see rv64_u128_arith.rs)
        let (ir, _) = compile("return (1 as u128) as bytes");
        lower_body(&ir).unwrap();

        let (ir, _) = compile("log(1, 2)");
        let err = lower_body(&ir).unwrap_err().to_string();
        assert!(err.contains("outside the translatable kernel subset"), "{err}");

        let (ir, _) = compile("log(1, 2)");
        let err = lower_body(&ir).unwrap_err().to_string();
        assert!(err.contains("outside the translatable kernel subset"), "{err}");

        // P3-V2IMPL: in-shape list literals lower; the ESCAPE (returning
        // the list) still rejects, naming the consuming opcode.
        let (ir, _) = compile("var b = [1, 2]\nreturn b");
        let err = lower_body(&ir).unwrap_err().to_string();
        assert!(err.contains("RET"), "{err}");
        assert!(err.contains("outside the translatable kernel subset"), "{err}");
    }


    #[test]
    fn lowers_core_constructs() {
        lower("var s = 0u64\nvar i = 0u64\nwhile i < 100 { s += i\ni += 1 }\nreturn s as bytes");
        lower("var b = 0x00112233445566778899aabbccddeeff\nreturn b ++ buf_cut(b, 1, 4) ++ buf_right(4, b)");
        lower("var b = 0x01020304\nvar i = 0u64\nvar s = 0u64\nwhile i < size(b) { s = s * 256 + byte(b, i)\ni += 1 }\nreturn s as bytes");
        lower("if 3 > 2 { return 0x01 } else { return 0x02 }");
        lower("require 1001, 1 < 2\nreturn 0xff");
        lower("throw 42");
        lower("var x = min(3, 5) + max(2, 9) + clamp(11, 1, 10)\nreturn x as bytes");
        lower("var x = (2 ** 10) % 1000\nreturn x as bytes");
    }


    /* ---- end-to-end execution through the consensus engine ---- */

    #[test]
    fn e2e_arithmetic_bodies() {
        // sum of 0..=99 with compound assignment
        let out = run_e2e(
            "var s = 0u64\nvar i = 0u64\nwhile i < 100 { s += i\ni += 1 }\nreturn s as bytes",
            &[],
        );
        assert_eq!(out, 4950u64.to_be_bytes().to_vec());

        // mul-acc mod prime (score_sum shaped)
        let out = run_e2e(
            "var s = 0u64\nvar i = 0u64\nwhile i < 64 { s = (s * 31 + i * 7) % 1000000007\ni += 1 }\nreturn s as bytes",
            &[],
        );
        let mut expect = 0u64;
        for i in 0u64..64 {
            expect = (expect.wrapping_mul(31).wrapping_add(i * 7)) % 1_000_000_007;
        }
        assert_eq!(out, expect.to_be_bytes().to_vec());

        // bytes: concat + cut + right
        let out = run_e2e(
            "var b = 0x0011223344556677\nreturn b ++ buf_cut(b, 1, 2) ++ buf_right(2, b)",
            &[],
        );
        assert_eq!(out, hex::decode("001122334455667711226677").unwrap());

        // bytes → int fold (big-endian)
        let out = run_e2e(
            "var b = 0x01020304\nvar i = 0u64\nvar s = 0u64\nwhile i < size(b) { s = s * 256 + byte(b, i)\ni += 1 }\nreturn s as bytes",
            &[],
        );
        assert_eq!(out, 0x01020304u64.to_be_bytes().to_vec());

        // branches: break/continue
        let out = run_e2e(
            "var s = 0u64\nvar i = 0u64\nwhile i < 100 { i += 1\nif i % 2 == 0 { continue }\nif i > 20 { break }\ns += i }\nreturn s as bytes",
            &[],
        );
        assert_eq!(out, 100u64.to_be_bytes().to_vec()); // 1+3+...+19 = 100

        // comparison semantics (all three modes): LE/GE carried two latent
        // encoder bugs that value-coincidence hid — pin all directions with
        // operand values chosen to flip on the bug.
        for mode in [Rv64Mode::Im, Rv64Mode::Imc, Rv64Mode::ImcRemap] {
            let pairs: [(u64, u64, &str); 6] = [
                (5, 5, "eq"), (5, 9, "lt"), (9, 5, "gt"),
                (0, 1, "lt01"), (1, 0, "gt10"), (6100, 6100, "eqbig"),
            ];
            for (a, b, tag) in pairs {
                // task-1 A2: bool values have no uint width (the interpreter
                // faults 71 on bool arithmetic) — pin them through `as u64`
                // so the comparison encoders stay exercised on the u64 plane
                let body = format!(
                    "var x = {a}u64\nvar y = {b}u64\nvar lt = (x < y) as u64\nvar gt = (x > y) as u64\nvar le = (x <= y) as u64\nvar ge = (x >= y) as u64\nreturn (lt * 1 + gt * 10 + le * 100 + ge * 1000) as bytes"
                );
                let want = (a < b) as u64 * 1
                    + (a > b) as u64 * 10
                    + (a <= b) as u64 * 100
                    + (a >= b) as u64 * 1000;
                let out = run_e2e_mode(&body, &[], mode);
                assert_eq!(
                    out,
                    want.to_be_bytes().to_vec(),
                    "cmp {tag} mode {mode:?}"
                );
            }
        }

        // checked overflow traps with exit code 90
        let lo = lower("var a = 18446744073709551615u64\nreturn (a + 1) as bytes");
        let cap = SpaceCap::new(0);
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
        assert!(exec.result.is_err());
        assert_eq!(exec.exit_code, 90);

        // division by zero traps with 90
        let lo = lower("var z = 0u64\nreturn (5 / z) as bytes");
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
        assert!(exec.result.is_err());
        assert_eq!(exec.exit_code, 90);

        // require error value maps to the exit code
        let lo = lower("require 1001, false\nreturn 0x01");
        let fno = CalcFnObj::build_v1(lo.pack_segments(&cap, 64, 5120, 1280).unwrap()).unwrap();
        let exec = fno.run(&[], CalcRunLimits { cycles: 1_000_000, resource: -1, deadline: None });
        assert!(exec.result.is_err());
        assert_eq!(exec.exit_code, 1001);
    }


    /// score_sum shape: byte-weighted mod-prime accumulation and a
    /// record-wise BE u64 fold — the HacdsDeFi `score_sum` wire format.
    #[test]
    fn e2e_input_score_sum_shape() {
        // task-1 A3: `byte(data, i) * 7` is u8-width in the interpreter
        // (faults from 37·7 on) — pin the leaf through `as u64` to keep the
        // loop-shape test on the u64 plane, matching the wrapping model
        let body = "var s = 0u64\nvar i = 0u64\nvar n = size(data)\nwhile i < n { s = (s * 31 + (byte(data, i) as u64) * 7u64) % 1000000007\ni += 1 }\nreturn s as bytes";
        let input: Vec<u8> = (0u8..=39).collect(); // 40 bytes
        let out = run_input(body, &input);
        let mut want: u64 = 0;
        for &b in &input {
            want = (want.wrapping_mul(31).wrapping_add(b as u64 * 7)) % 1_000_000_007;
        }
        assert_eq!(out, want.to_be_bytes().to_vec());

        // record-wise BE u64 fold: 5 × 8-byte records summed
        let body2 = "var s = 0u64\nvar off = 0u64\nvar n = size(data)\nwhile off < n { var v = 0u64\nvar j = 0u64\nwhile j < 8 { v = v * 256 + byte(data, off + j)\nj += 1 }\ns += v\noff += 8 }\nreturn s as bytes";
        let mut input2 = Vec::new();
        let mut want2 = 0u64;
        for k in 0u64..5 {
            let r = (k * 0x0102_0304_0506_0708) % 1_000_000_007;
            want2 = want2.wrapping_add(r);
            input2.extend_from_slice(&r.to_be_bytes());
        }
        let out2 = run_input(body2, &input2);
        assert_eq!(out2, want2.to_be_bytes().to_vec());
    }


    #[test]
    fn comparison_report() {
        let bodies: Vec<(&str, &str, u64)> = vec![
            (
                "sum loop",
                "var s = 0u64\nvar i = 0u64\nwhile i < 1000 { s += i\ni += 1 }\nreturn s as bytes",
                1000,
            ),
            (
                "mul-acc mod prime (score_sum shape)",
                "var s = 0u64\nvar i = 0u64\nwhile i < 1000 { s = (s * 31 + i * 7) % 1000000007\ni += 1 }\nreturn s as bytes",
                1000,
            ),
            (
                "bytes fold (byte/size BE u64)",
                "var b = 0x0102030405060708\nvar i = 0u64\nvar s = 0u64\nwhile i < 1000 { s = (s * 31 + byte(b, i % 8)) % 1000000007\ni += 1 }\nreturn s as bytes",
                1000,
            ),
            (
                "branchy loop (continue)",
                "var s = 0u64\nvar i = 0u64\nwhile i < 1000 { i += 1\nif i % 2 == 0 { continue }\ns += i }\nreturn s as bytes",
                1000,
            ),
        ];
        println!(
            "{:<34} {:>6} {:>6} {:>8} {:>10} {:>11} {:>9} {:>8}",
            "workload", "rv_B", "hvm_B", "rv_inst", "cyc/iter", "comp/iter", "hvm_gas_i", "win"
        );
        for (name, body, iters) in &bodies {
            let (ir, hvm) = compile(body);
            let lo = lower_body(&ir).unwrap();
            let (hb, _) = hvm_bytecode_stats(&hvm).unwrap();
            let (exec, _out) = rv_exec(body);
            let per_iter = exec / *iters;
            let comp_iter = per_iter as f64 / N;
            let hvm_gas = hvm_per_iter_gas(&ir).unwrap_or(0);
            let win = hvm_gas as f64 / comp_iter;
            println!(
                "{:<34} {:>6} {:>6} {:>8} {:>10} {:>11.2} {:>9} {:>7.0}x",
                name,
                lo.stats.code_bytes,
                hb,
                lo.stats.insts,
                per_iter,
                comp_iter,
                hvm_gas,
                win
            );
            assert_eq!(lo.stats.code_bytes % 4, 0);
        }
    }
