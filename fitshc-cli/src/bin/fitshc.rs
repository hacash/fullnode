//! fitshc — FitSH contract compiler CLI with rustc-backed rv64im calc
//! kernel builds.
//!
//! ```text
//! fitshc <file.fitsh> [fee] [nonce] [--periods N]      compile + deploy json (legacy form)
//! fitshc build <file.fitsh> [fee] [nonce] [flags]      same, explicit form
//! fitshc pack <kernel.elf> [--profile P] [flags]       ELF → verified .hcf1 container
//! fitshc run <kernel.rs|.hcf1|.elf> [--input 0x..]     build (if .rs) and execute locally
//! ```
//!
//! `function rv64im kernel(data: bytes) -> bytes` declarations without
//! `= 0x...` are resolved automatically: a sibling `kernel.rs` (or
//! `calc/kernel.rs`) is compiled through a generated wrapper crate, packed
//! with the smallest fitting memory, verified with the exact chain checks,
//! and injected into the contract. `= 0x...` stays the prebuilt-container
//! form; `{ ... }` (fitsh-written kernels) is reserved and reports
//! "not supported yet". See `vm/doc/fitshc-calc.loc.md`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use field::*;
use hacash_params::MAINNET_PARAMS;
use protocol::tx_std::StdTransaction;
use serde_json::{Value, json};
use sys::{Account, curtimes};
use vm::action::{ContractDeploy, contract_deploy_charge_bytes};
use vm::calc::SpaceCap;
use vm::contract::ContractSto;
use vm::fitshc::parse_deploy::DeployInfo;

use fitshc_cli::meta::{MemSpec, ProfileName};
use fitshc_cli::resolve::ElfDefaults;
use fitshc_cli::rustbuild::KernelBuildRequest;
use fitshc_cli::{BuildConfig, compile_fitsh_with_calcs, run_container};

fn usage() {
    println!(
        "fitshc — FitSH compiler & rv64im calc kernel builder\n\
         \n\
         USAGE:\n\
         \x20   fitshc <file.fitsh> [fee] [nonce] [--periods N] [flags]   compile contract (legacy form)\n\
         \x20   fitshc build <file.fitsh> [fee] [nonce] [flags]           same, explicit form\n\
         \x20   fitshc pack <kernel.elf> [flags]                          pack ELF → verified .hcf1\n\
         \x20   fitshc run <kernel.rs|kernel.hcf1|kernel.elf> [flags]     build + execute a kernel locally\n\
         \n\
         KERNEL RESOLUTION (function rv64im <name> without '= 0x..'):\n\
         \x20   1. --calc <name>=<path>          explicit .rs/.hcf1/.elf override\n\
         \x20   2. <name>.rs / calc/<name>.rs    rust kernel, auto-compiled (metadata in '// calc:' header)\n\
         \x20   3. <name>.hcf1 / <name>.elf      prebuilt artifacts (--no-build skips step 2)\n\
         \n\
         FLAGS:\n\
         \x20   --calc name=path   explicit kernel override (repeatable)\n\
         \x20   --no-build         never invoke rustc; only prebuilt .hcf1/.elf\n\
         \x20   --strict           rebuild core/compiler_builtins (rv64imc/b: drops prebuilt +A caveat)\n\
         \x20   --in N --out N     IO sizes for bare .elf packing (default 1024/1024)\n\
         \x20   --profile P        rv64im|rv64imb|rv64imc|rv64imcb for .elf packing (default rv64imc)\n\
         \x20   --mem auto|N       memory admission for .elf packing (default auto)\n\
         \x20   --cap l1           target chain caps (l1 only for now)\n\
         \x20   -o <path>          output path (deploy json / container)\n\
         \x20   --input 0x..       run: call data (hex)\n\
         \x20   --cycles N         run: cycle budget (default 1000000)\n\
         \x20   --verbose          print resolved kernel details\n\
         \x20   --periods N        deploy storage-fee basis (see below)"
    );
}

struct CommonArgs {
    rest: Vec<String>,
    overrides: BTreeMap<String, PathBuf>,
    no_build: bool,
    strict: bool,
    elf_defaults: ElfDefaults,
    mem: MemSpec,
    out: Option<String>,
    verbose: bool,
    periods: Option<u64>,
}

impl CommonArgs {
    fn parse(args: &[String]) -> Result<CommonArgs, String> {
        let mut me = CommonArgs {
            rest: Vec::new(),
            overrides: BTreeMap::new(),
            no_build: false,
            strict: false,
            elf_defaults: ElfDefaults::default(),
            mem: MemSpec::Auto,
            out: None,
            verbose: false,
            periods: None,
        };
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            macro_rules! val {
                ($what:expr) => {{
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| format!("missing value for {}", $what))?
                }};
            }
            match a.as_str() {
                "--calc" => {
                    let v = val!("--calc");
                    let (name, path) = v
                        .split_once('=')
                        .ok_or("--calc expects name=path (e.g. --calc kernel=./kernel.hcf1)")?;
                    me.overrides
                        .insert(name.trim().to_string(), PathBuf::from(path.trim()));
                }
                "--no-build" => me.no_build = true,
                "--strict" => me.strict = true,
                "--in" => {
                    me.elf_defaults.input_max = val!("--in").parse().map_err(|_| "--in N")?;
                }
                "--out" => {
                    me.elf_defaults.output_max = val!("--out").parse().map_err(|_| "--out N")?;
                }
                "--profile" => {
                    let v = val!("--profile");
                    me.elf_defaults.profile =
                        ProfileName::parse(&v).ok_or_else(|| format!("unknown profile {v:?}"))?;
                }
                "--mem" => {
                    let v = val!("--mem");
                    me.mem = if v == "auto" {
                        MemSpec::Auto
                    } else {
                        MemSpec::KiB(v.parse().map_err(|_| "--mem auto|N")?)
                    };
                }
                "--cap" => {
                    let v = val!("--cap");
                    if v != "l1" {
                        return Err(format!(
                            "unknown cap {v:?}: only the L1 default is wired up; sidechains pass \
                             their SpaceCap out of band"
                        ));
                    }
                }
                "--host-ecall" => me.elf_defaults.no_host_ecall = false,
                "-o" | "--output" => me.out = Some(val!("-o")),
                "--input" => return Err("--input is only valid for `fitshc run`".into()),
                "--verbose" | "-v" => me.verbose = true,
                "--periods" => {
                    me.periods = Some(val!("--periods").parse().map_err(|_| "--periods N")?);
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown flag {other:?}"));
                }
                _ => me.rest.push(a.clone()),
            }
            i += 1;
        }
        Ok(me)
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        return;
    }
    let result = match args[1].as_str() {
        "build" if args.len() > 2 => cmd_build(&args[2..]),
        "pack" if args.len() > 2 => cmd_pack(&args[2..]),
        "run" if args.len() > 2 => cmd_run(&args[2..]),
        "-h" | "--help" | "help" => {
            usage();
            Ok(())
        }
        "build" | "pack" | "run" => Err(format!("{} requires a file argument", args[1])),
        _ => cmd_build(&args[1..]),
    };
    if let Err(e) = result {
        println!("Error: {e}");
        std::process::exit(1);
    }
}

fn cmd_build(args: &[String]) -> Result<(), String> {
    let common = CommonArgs::parse(args)?;
    let file_path = common
        .rest
        .first()
        .cloned()
        .ok_or_else(|| "missing <file.fitsh> (try --help)".to_string())?;
    let source = fs::read_to_string(&file_path).map_err(|e| format!("reading {file_path}: {e}"))?;
    let cap = SpaceCap::new(0);
    let cfg = BuildConfig {
        overrides: common.overrides.clone(),
        no_build: common.no_build,
        strict: common.strict,
        elf_defaults: common.elf_defaults.clone(),
        cache_root: None,
    };

    let ((contract, deploy_opt, smaps, contract_name), artifacts, warnings) =
        compile_fitsh_with_calcs(&source, Path::new(&file_path), &cfg, &cap)?;
    println!("Compile success!");

    // Kernel side outputs were written next to the fitsh source by the
    // library call; report them.
    for a in &artifacts {
        println!(
            "Calc kernel '{}': {} (profile {:#04x}, mem {} KiB, in {}, out {}, image {} B, id {}{})",
            a.name,
            a.hcf1_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            a.profile_byte,
            a.mem_kib,
            a.input_max,
            a.output_max,
            a.image_len,
            &a.image_id[..16.min(a.image_id.len())],
            if a.cache_hit { " [cache]" } else { "" },
        );
        if common.verbose {
            println!(
                "    manifest: {}.calc.manifest.json; regions: code {} B, rodata {} B, init {} B (cap image max {} B)",
                a.name, a.code_len, a.rodata_len, a.init_len, cap.calc_image_max
            );
        }
    }
    for w in &warnings {
        println!("Warning: {w}");
    }

    let sto = contract.into_sto();
    write_contract_map(&file_path, &smaps, &contract_name, &sto);
    write_deploy_json(
        &file_path,
        &common,
        deploy_opt,
        sto,
        common.rest.get(1).cloned(),
        common.rest.get(2).cloned(),
    );
    Ok(())
}

fn write_contract_map(
    file_path: &str,
    smaps: &[(String, vm::SourceMap)],
    name: &str,
    sto: &ContractSto,
) {
    let path = Path::new(file_path);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or("output".into());
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut func_maps = Vec::new();
    for (fname, sm) in smaps {
        if let Ok(json_str) = sm.to_json() {
            if let Ok(mut v) = serde_json::from_str::<Value>(&json_str) {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("name".to_string(), json!(fname));
                }
                func_maps.push(v);
            }
        }
    }
    let inherit_addrs: Vec<String> = sto
        .inherit
        .as_list()
        .iter()
        .map(|a| a.to_readable())
        .collect();
    let lib_addrs: Vec<String> = sto
        .library
        .as_list()
        .iter()
        .map(|a| a.to_readable())
        .collect();
    let contract_map = json!({
        "contract": name,
        "inherits": inherit_addrs,
        "libs": lib_addrs,
        "funcs": func_maps
    });
    let map_file = parent.join(format!("{stem}.contractmap.json"));
    fs::write(
        &map_file,
        serde_json::to_string_pretty(&contract_map).unwrap(),
    )
    .ok();
    println!("Generated: {}", map_file.display());
}

fn write_deploy_json(
    file_path: &str,
    common: &CommonArgs,
    deploy_opt: Option<DeployInfo>,
    sto: ContractSto,
    fee_pos: Option<String>,
    nonce_pos: Option<String>,
) {
    let (d_fee, d_nonce, d_argv) = match deploy_opt {
        Some(info) => (info.protocol_cost, info.nonce, info.construct_argv),
        None => (None, None, None),
    };
    let fee_str = fee_pos.as_deref().unwrap_or("1:248");
    let txfee = Amount::from(fee_str).unwrap_or(Amount::default());
    let nonce_val = nonce_pos
        .as_deref()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    let nonce = d_nonce.unwrap_or(Uint4::from(nonce_val));
    let argv = d_argv.unwrap_or_default();
    let charge_bytes = contract_deploy_charge_bytes(&sto) as u128;

    // Storage-fee basis: --periods wins, then HACASH_STORAGE_FEE_PERIODS,
    // then full price.
    let full_periods = MAINNET_PARAMS.protocol.vm.contract_store_perm_periods;
    let period_floor = MAINNET_PARAMS.protocol.vm.contract_storage_fee.period_floor;
    let mut periods = full_periods;
    let mut periods_explicit = false;
    if let Some(p) = common.periods {
        match parse_storage_periods(&p.to_string(), period_floor, full_periods) {
            Some(v) => {
                periods = v;
                periods_explicit = true;
            }
            None => {
                println!(
                    "Error: --periods expects a multiple of {period_floor} in [{period_floor}, {full_periods}]"
                );
                return;
            }
        }
    }
    if !periods_explicit {
        if let Ok(env_p) = env::var("HACASH_STORAGE_FEE_PERIODS") {
            if let Some(v) = parse_storage_periods(&env_p, period_floor, full_periods) {
                periods = v;
            }
        }
    }

    let protocol_cost = d_fee.unwrap_or_else(|| {
        estimate_protocol_cost_auto_with_periods(
            &txfee,
            nonce,
            argv.clone(),
            &sto,
            charge_bytes,
            periods,
        )
    });

    let mut action = ContractDeploy::default();
    let protocol_cost_u232 = protocol_cost
        .to_unit_u128(base::FEE_PRICING_UNIT)
        .unwrap_or(0);
    action.protocol_cost = protocol_cost;
    action.nonce = nonce;
    action.construct_argv = argv;
    action.contract = sto;

    let action_body_bytes = action.encode();
    let deploy_json = json!({
        "action": hex::encode(&action_body_bytes),
        "storage_periods": periods,
        "storage_full_periods": full_periods,
        "fee_pricing_unit": base::FEE_PRICING_UNIT,
        "protocol_cost_u232": protocol_cost_u232.to_string(),
        "protocol_cost_basis": if periods >= full_periods { "full_price" } else { "discount" },
    });

    let path = Path::new(file_path);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or("output".into());
    let parent = path.parent().unwrap_or(Path::new("."));
    let deploy_file = match &common.out {
        Some(o) => PathBuf::from(o),
        None => parent.join(format!("{stem}.deploy.json")),
    };
    fs::write(
        &deploy_file,
        serde_json::to_string_pretty(&deploy_json).unwrap(),
    )
    .ok();
    println!("Generated: {}", deploy_file.display());
    if periods < full_periods {
        println!(
            "Deploy priced at the discounted basis ({periods} periods): the transaction is only valid while the block budget still grants that discount."
        );
    } else {
        println!(
            "Deploy priced at full price ({full_periods} periods): guaranteed inclusion. Use --periods N (current discount periods) to deploy discounted."
        );
    }
}

fn parse_storage_periods(raw: &str, floor: u64, full: u64) -> Option<u64> {
    let v = raw.parse::<u64>().ok()?;
    if floor == 0 || v < floor || v > full || v % floor != 0 {
        return None;
    }
    Some(v)
}

fn estimate_protocol_cost_auto_with_periods(
    txfee: &Amount,
    nonce: Uint4,
    argv: BytesW2,
    sto: &ContractSto,
    charge_bytes: u128,
    periods: u64,
) -> Amount {
    const SAFETY_NUM: u128 = 103; // +3% headroom
    const SAFETY_DEN: u128 = 100;
    const MAX_ITERS: usize = 6;
    if charge_bytes == 0 {
        return Amount::zero();
    }
    let mut cur = Amount::unit232(1);
    let mut best_need: u128 = 0;
    for _ in 0..MAX_ITERS {
        let mut act = ContractDeploy::default();
        act.protocol_cost = cur.clone();
        act.nonce = nonce;
        act.construct_argv = argv.clone();
        act.contract = sto.clone();

        let acc = Account::create_by_password("123456").unwrap();
        let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        let mut tx =
            StdTransaction::new_by(hacash_params::TX_TYPE_3, addr, txfee.clone(), curtimes());
        tx.push_action_in(Arc::new(act));
        tx.gas_max = Uint1::from(8);
        tx.fill_sign_account(&acc).unwrap();

        // Purity/floor in the chain pricing unit (u232 per billing byte); the
        // protocol-cost amount is ceiled to the gas settlement unit (u238).
        let fee_purity = MAINNET_PARAMS
            .protocol
            .vm
            .effective_fee_purity(0, tx.type3_fee_purity()) as u128;
        let mut need = fee_purity
            .saturating_mul(charge_bytes)
            .saturating_mul(periods as u128);
        need = need
            .saturating_mul(SAFETY_NUM)
            .saturating_add(SAFETY_DEN - 1)
            / SAFETY_DEN;
        if need > best_need {
            best_need = need;
        }
        let next = base::settlement_amount(best_need.max(1));
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn cmd_pack(args: &[String]) -> Result<(), String> {
    let common = CommonArgs::parse(args)?;
    let elf_path = common
        .rest
        .first()
        .cloned()
        .ok_or_else(|| "fitshc pack requires a <kernel.elf>".to_string())?;
    let cap = SpaceCap::new(0);
    let (c, bytes) = fitshc_cli::resolve::pack_elf_file(
        Path::new(&elf_path),
        &common.elf_defaults,
        common.mem,
        &cap,
    )?;
    let out = match common.out {
        Some(o) => PathBuf::from(o),
        None => Path::new(&elf_path).with_extension("hcf1"),
    };
    fs::write(&out, &bytes).map_err(|e| format!("write {}: {e}", out.display()))?;
    println!(
        "packed {} -> {} (profile {:#04x}, mem {} KiB, in {}, out {}, image {} B, id {})",
        elf_path,
        out.display(),
        c.profile,
        c.mem_kib,
        c.input_max,
        c.output_max,
        bytes.len(),
        hex::encode(&c.image_id()[..8.min(32)])
    );
    Ok(())
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let mut input_hex = String::new();
    let mut cycles: u64 = 1_000_000;
    let mut filtered: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" if i + 1 < args.len() => {
                input_hex = args[i + 1].clone();
                i += 2;
            }
            "--cycles" if i + 1 < args.len() => {
                cycles = args[i + 1].parse().map_err(|_| "--cycles N")?;
                i += 2;
            }
            other => {
                filtered.push(other.to_string());
                i += 1;
            }
        }
    }
    let common = CommonArgs::parse(&filtered)?;
    let kernel_path = common
        .rest
        .first()
        .cloned()
        .ok_or_else(|| "fitshc run requires a <kernel.rs|.hcf1|.elf>".to_string())?;
    let path = Path::new(&kernel_path);
    let input = if input_hex.is_empty() {
        Vec::new()
    } else {
        let h = input_hex
            .strip_prefix("0x")
            .or_else(|| input_hex.strip_prefix("0X"))
            .unwrap_or(&input_hex);
        hex::decode(h).map_err(|e| format!("--input hex: {e}"))?
    };
    let cap = SpaceCap::new(0);
    let (container, origin) = match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("kernel file has no stem")?;
            let source =
                fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
            let meta = fitshc_cli::meta::parse_kernel_meta(path, &source)?;
            let built = KernelBuildRequest {
                name,
                source_path: path,
                meta: &meta,
                has_input: true,
                cap: &cap,
                strict: common.strict,
                cache_root: None,
            }
            .build()?;
            println!(
                "built {} (profile {:#04x}, mem {} KiB, image {} B{})",
                path.display(),
                built.container.profile,
                built.mem_kib,
                built.container_bytes.len(),
                if built.cache_hit { " [cache]" } else { "" }
            );
            (built.container, "rust-source")
        }
        Some("hcf1") | Some("hcf") => {
            let (c, _) = fitshc_cli::resolve::load_container_file(path, &cap)?;
            (c, "prebuilt-container")
        }
        Some("elf") => {
            let (c, _) =
                fitshc_cli::resolve::pack_elf_file(path, &common.elf_defaults, common.mem, &cap)?;
            (c, "prebuilt-elf")
        }
        other => {
            return Err(format!(
                "unsupported kernel type {other:?}: expected .rs, .hcf1 or .elf"
            ));
        }
    };
    let (out, used, resource) = run_container(container.clone(), &input, cycles)?;
    println!(
        "kernel: {} (profile {:#04x}, mem {} KiB)",
        origin, container.profile, container.mem_kib
    );
    println!("output: 0x{}", hex::encode(&out));
    println!("cycles: {used}, resource: {resource}");
    Ok(())
}
