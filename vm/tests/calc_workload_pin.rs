//! Pins interpreter and AOT on the financial workload ELFs.
//!
//! Kernels that fit the L1 image and memory caps are packed with
//! `SpaceCap::new(0)` and run through `CalcFnObj::run`, which executes the
//! interpreter and then AOT and rejects the call on any disagreement.
//! secp256k1 does not fit the `u16` wire ceiling or the 64 KiB window; that
//! case loads the ELF with `load_elf_direct` (testkit) and runs the same
//! lockstep without a contract deploy.
//!
//! Rebuild the ELFs with `vm/tests/fixtures/workloads/build.sh`.

#[cfg(feature = "execute")]
mod workloads {
    use std::collections::HashMap;

    use vm::calc::{
        CALC_PROFILE_RV64IM, CalcAbort2, CalcFnObj, CalcHostConfig, CalcRunLimits, SpaceCap,
        load_elf_direct, pack_elf, pack_elf_min_mem,
    };

    struct Pin {
        name: &'static str,
        elf: &'static str,
        mem: u32,
        in_max: u16,
        out_max: u16,
        input: Vec<u8>,
        cycles: u64,
        resource: i64,
        output: Vec<u8>,
    }

    fn dir() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/workloads").to_string()
    }

    fn unhex(s: &str) -> Vec<u8> {
        let s = s.trim();
        assert!(s.len() % 2 == 0, "odd hex length");
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    fn pins() -> Vec<Pin> {
        let text = std::fs::read_to_string(format!("{}/pins.txt", dir())).expect("pins.txt");
        text.lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                assert_eq!(f.len(), 9, "pin line {l}");
                Pin {
                    name: f[0].to_string().leak(),
                    elf: f[1].to_string().leak(),
                    mem: f[2].parse().unwrap(),
                    in_max: f[3].parse().unwrap(),
                    out_max: f[4].parse().unwrap(),
                    input: unhex(f[5]),
                    cycles: f[6].parse().unwrap(),
                    resource: f[7].parse().unwrap(),
                    output: unhex(f[8]),
                }
            })
            .collect()
    }

    fn limits(cycles: u64) -> CalcRunLimits {
        CalcRunLimits {
            cycles,
            resource: -1,
            deadline: None,
        }
    }

    fn build_l1(elf: &[u8], mem: u32, in_max: u16, out_max: u16) -> CalcFnObj {
        let cap = SpaceCap::new(0);
        let c = if mem == 0 {
            pack_elf_min_mem(&cap, elf, CALC_PROFILE_RV64IM, in_max, out_max)
        } else {
            pack_elf(&cap, elf, CALC_PROFILE_RV64IM, mem, in_max, out_max)
        }
        .expect("L1 pack");
        CalcFnObj::build_with_aot(c).expect("AOT mount")
    }

    #[test]
    fn workload_outputs_and_cycles_match_both_engines() {
        let mut built: HashMap<(&str, u32, u16, u16), CalcFnObj> = HashMap::new();
        for pin in pins() {
            let key = (pin.elf, pin.mem, pin.in_max, pin.out_max);
            let obj = built.entry(key).or_insert_with(|| {
                let elf = std::fs::read(format!("{}/elf/{}.elf", dir(), pin.elf)).expect("elf");
                build_l1(&elf, pin.mem, pin.in_max, pin.out_max)
            });
            #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
            assert!(obj.aot_mounted(), "{}: AOT image was not mounted", pin.name);

            let exec = obj.run(&pin.input, limits(pin.cycles));
            let got = exec
                .result
                .unwrap_or_else(|e| panic!("{}: {:?} cycles={}", pin.name, e, exec.cycles_used));
            assert_eq!(got, pin.output, "{}", pin.name);
            assert_eq!(exec.cycles_used, pin.cycles, "{}", pin.name);
            assert_eq!(exec.resource, pin.resource, "{}", pin.name);
            assert_eq!(exec.exit_code, 0, "{}", pin.name);

            let under = obj.run(&pin.input, limits(pin.cycles - 1));
            assert!(
                matches!(under.result, Err(CalcAbort2::OutOfGas)),
                "{}: budget-1 must be OutOfGas, got {:?} cycles={}",
                pin.name,
                under.result,
                under.cycles_used
            );
        }
    }

    /// Image is above the u16 wire ceiling and the 64 KiB window, so it is
    /// not packed as a contract. The testkit loader verifies it under a
    /// raised memory cap and runs interpreter/AOT lockstep directly.
    #[test]
    fn secp256k1_direct_lockstep_outside_deploy_cap() {
        let elf = std::fs::read(format!("{}/elf/secp256k1.elf", dir())).expect("secp elf");
        let mut wide = SpaceCap::new(0);
        wide.calc_mem_kib_max = 256;
        wide.calc_image_max = 1_000_000;
        assert!(
            pack_elf_min_mem(&wide, &elf, CALC_PROFILE_RV64IM, 8, 8).is_err(),
            "wire u16 ceiling must still reject secp256k1"
        );
        // Minimum window leaves a 2 KiB stack. k256's frames store through
        // that into the RX image. 256 KiB is above the L1 cap and leaves
        // room for the frame; it is still a testkit memory, not a deploy.
        let c = load_elf_direct(&wide, &elf, CALC_PROFILE_RV64IM, Some(256), 8, 8)
            .expect("direct verify");
        assert!(c.mem_kib > 64, "secp256k1 must sit above the L1 memory cap");
        assert!(
            c.encode().expect("encode").len() > 65535,
            "secp256k1 must sit above the wire ceiling"
        );
        let obj =
            CalcFnObj::build(c, &wide, CalcHostConfig::new().with_aot_mount(true)).expect("build");
        #[cfg(all(feature = "calc-aot", target_arch = "x86_64"))]
        assert!(obj.aot_mounted());
        let exec = obj.run(&[0x00], limits(500_000_000));
        let got = exec
            .result
            .unwrap_or_else(|e| panic!("secp256k1: {:?} cycles={}", e, exec.cycles_used));
        assert_eq!(got, vec![1], "secp256k1 must accept the pinned signature");
        assert_eq!(exec.cycles_used, 3_208_116, "secp256k1 cycles");
        assert_eq!(exec.resource, 514, "secp256k1 resource");
        assert_eq!(exec.exit_code, 0);
        let under = obj.run(&[0x00], limits(3_208_116 - 1));
        assert!(
            matches!(under.result, Err(CalcAbort2::OutOfGas)),
            "secp256k1 budget-1 must be OutOfGas, got {:?}",
            under.result
        );
    }
}
