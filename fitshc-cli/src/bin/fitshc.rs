//! fitshc — FitSH contract compiler CLI with rustc-backed rv64im calc
//! kernel builds.
//!
//! ```text
//! fitshc <file.fitsh> [fee] [nonce] [--periods N]      compile + deploy json (legacy form)
//! fitshc build <file.fitsh> [fee] [nonce] [flags]      same, explicit form
//! fitshc pack <kernel.elf> [--profile P] [flags]       ELF → verified .hcf1 container
//! fitshc image <kernel.rs> [-o file] [--hex]           .rs → function rv64im container bytes
//! fitshc run <kernel.rs|.hcf1|.elf> [--input 0x..]     build (if .rs) and execute locally
//! ```
//!
//! `function rv64im kernel(data: bytes) -> bytes` declarations without
//! `= 0x...` are resolved automatically: a sibling `kernel.rs` (or
//! `calc/kernel.rs`) is compiled through a generated wrapper crate, packed
//! with the smallest fitting memory, verified with the exact chain checks,
//! and injected into the contract. `= 0x...` stays the prebuilt-container
//! form; `{ ... }` (fitsh-written kernels) is reserved and reports
//! "not supported yet". See `vm/doc/fitshc-calc.loc.md` and the crate
//! README.

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

use fitshc_cli::meta::{MemSpec, ProfileName, infer_kernel_has_input, parse_kernel_meta};
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
         \x20   fitshc image <kernel.rs> [-o file] [--hex]                .rs → function rv64im container bytes\n\
         \x20   fitshc run <kernel.rs|kernel.hcf1|kernel.elf> [flags]     build + execute a kernel locally\n\
         \n\
         KERNEL RESOLUTION (function rv64im <name> without '= 0x..'):\n\
         \x20   1. --calc <name>=<path>          explicit .rs/.hcf1/.elf override\n\
         \x20   2. <name>.rs / calc/<name>.rs    rust kernel, auto-compiled (metadata in '// calc:' header)\n\
         \x20   3. <name>.hcf1 / <name>.elf      prebuilt artifacts (--no-build skips step 2)\n\
         \n\
         FLAGS:\n\
         \x20   --calc name=path   explicit kernel override (repeatable; build only)\n\
         \x20   --no-build         never invoke rustc; only prebuilt .hcf1/.elf\n\
         \x20   --strict           rebuild core/compiler_builtins (rv64imc/b: drops prebuilt +A caveat)\n\
         \x20   --in N --out N     IO sizes for bare .elf packing (default 1024/1024)\n\
         \x20   --profile P        rv64im (v1 admits only this; default rv64im)\n\
         \x20   --mem auto|N       memory admission for .elf packing (default auto)\n\
         \x20   --no-cache         never read/write the shared build cache; temp build dir is removed\n\
         \x20   --no-artifacts     do not write <name>.hcf1/.calc.manifest.json next to the fitsh\n\
         \x20   --artifacts        force artifact writing (overrides the --no-build default)\n\
         \x20   --out-dir <dir>    write kernel artifacts and contractmap/deploy json into <dir>\n\
         \x20   --no-input         image/run: the .rs kernel takes only `out` (zero-arg form)\n\
         \x20   --hex              image: also print the container as 0x… on stdout\n\
         \x20   --input 0x..       run: call data (hex)\n\
         \x20   --cycles N         run: cycle budget (default 1000000)\n\
         \x20   -o <path>          output path (image/pack: container file; build: deploy json)\n\
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
    no_cache: bool,
    /// `--artifacts` / `--no-artifacts`; `None` = subcommand default.
    artifacts: Option<bool>,
    out_dir: Option<PathBuf>,
    out: Option<String>,
    verbose: bool,
    /// `image`/`run`: kernel takes only `out`.
    no_input: bool,
    /// `image`: print `0x` + hex of the container on stdout.
    hex: bool,
    /// Validated storage periods (multiple of floor within [floor, full]).
    periods: Option<u64>,
}

impl CommonArgs {
    fn parse(args: &[String], sub: &str) -> Result<CommonArgs, String> {
        let mut me = CommonArgs {
            rest: Vec::new(),
            overrides: BTreeMap::new(),
            no_build: false,
            strict: false,
            elf_defaults: ElfDefaults::default(),
            no_cache: false,
            artifacts: None,
            out_dir: None,
            out: None,
            verbose: false,
            no_input: false,
            hex: false,
            periods: None,
        };
        let full_periods = MAINNET_PARAMS.protocol.vm.contract_store_perm_periods;
        let period_floor = MAINNET_PARAMS.protocol.vm.contract_storage_fee.period_floor;
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
                    if sub != "build" {
                        return Err(format!(
                            "--calc is not valid for `{sub}` (it overrides kernels of a .fitsh build)"
                        ));
                    }
                    let v = val!("--calc");
                    let (name, path) = v
                        .split_once('=')
                        .ok_or("--calc expects name=path (e.g. --calc kernel=./kernel.hcf1)")?;
                    me.overrides
                        .insert(name.trim().to_string(), PathBuf::from(path.trim()));
                }
                "--no-build" => {
                    if sub == "image" {
                        return Err(
                            "--no-build is not valid for `image` (it compiles the .rs)".into()
                        );
                    }
                    me.no_build = true;
                }
                "--strict" => me.strict = true,
                "--in" => {
                    if sub == "image" {
                        return Err(
                            "--in is not valid for `image`; set `// calc: input_max` in the .rs"
                                .into(),
                        );
                    }
                    me.elf_defaults.input_max = val!("--in").parse().map_err(|_| "--in N")?;
                }
                "--out" => {
                    if sub == "image" {
                        return Err(
                            "--out is not valid for `image`; set `// calc: output_max` in the .rs"
                                .into(),
                        );
                    }
                    me.elf_defaults.output_max = val!("--out").parse().map_err(|_| "--out N")?;
                }
                "--profile" => {
                    if sub == "image" {
                        return Err(
                            "--profile is not valid for `image`; set `// calc: profile` in the .rs"
                                .into(),
                        );
                    }
                    let v = val!("--profile");
                    me.elf_defaults.profile =
                        ProfileName::parse(&v).ok_or_else(|| format!("unknown profile {v:?}"))?;
                    // rv64im / rv64imb / rv64imc / rv64imcb are all admitted
                    // by the chain; hash stays rejected (see --hash).
                }
                "--mem" => {
                    if sub == "image" {
                        return Err(
                            "--mem is not valid for `image`; set `// calc: mem_kib` in the .rs"
                                .into(),
                        );
                    }
                    let v = val!("--mem");
                    me.elf_defaults.mem = if v == "auto" {
                        MemSpec::Auto
                    } else {
                        MemSpec::KiB(v.parse().map_err(|_| "--mem auto|N")?)
                    };
                }
                "--hash" => {
                    return Err("hash ecalls are not admitted in v1".into());
                }
                "--no-cache" => me.no_cache = true,
                "--no-input" => {
                    if sub != "image" && sub != "run" {
                        return Err(format!("--no-input is not valid for `{sub}`"));
                    }
                    me.no_input = true;
                }
                "--hex" => {
                    if sub != "image" {
                        return Err(format!(
                            "--hex is not valid for `{sub}` (it prints an image container)"
                        ));
                    }
                    me.hex = true;
                }
                "--artifacts" => {
                    if sub == "image" {
                        return Err(
                            "--artifacts is not valid for `image` (the container file is the output)"
                                .into(),
                        );
                    }
                    me.artifacts = Some(true);
                }
                "--no-artifacts" => {
                    if sub == "image" {
                        return Err("--no-artifacts is not valid for `image`".into());
                    }
                    me.artifacts = Some(false);
                }
                "--out-dir" => {
                    if sub == "image" {
                        return Err("--out-dir is not valid for `image`; pass -o <file>".into());
                    }
                    me.out_dir = Some(PathBuf::from(val!("--out-dir")));
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
                "-o" | "--output" => me.out = Some(val!("-o")),
                "--verbose" | "-v" => me.verbose = true,
                "--periods" => {
                    if sub == "image" {
                        return Err("--periods is not valid for `image`".into());
                    }
                    let v = val!("--periods");
                    let n: u64 = v
                        .parse()
                        .map_err(|_| format!("--periods expects a number, got {v:?}"))?;
                    if period_floor == 0
                        || n < period_floor
                        || n > full_periods
                        || n % period_floor != 0
                    {
                        return Err(format!(
                            "--periods expects a multiple of {period_floor} in [{period_floor}, {full_periods}], got {n}"
                        ));
                    }
                    me.periods = Some(n);
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

/// Absolute path with `.` and `..` removed, without requiring the file to exist.
fn lex_abs(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir().unwrap_or_default().join(path)
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
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
        "image" if args.len() > 2 => cmd_image(&args[2..]),
        "run" if args.len() > 2 => cmd_run(&args[2..]),
        "-h" | "--help" | "help" => {
            usage();
            Ok(())
        }
        "build" | "pack" | "image" | "run" => Err(format!("{} requires a file argument", args[1])),
        _ => cmd_build(&args[1..]),
    };
    if let Err(e) = result {
        println!("Error: {e}");
        std::process::exit(1);
    }
}

fn cmd_build(args: &[String]) -> Result<(), String> {
    let common = CommonArgs::parse(args, "build")?;
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
        no_cache: common.no_cache,
        // --no-build flips the default off (CI / read-only checkouts);
        // --artifacts / --no-artifacts override either way.
        emit_artifacts: common.artifacts.unwrap_or(!common.no_build),
        artifact_dir: common.out_dir.clone(),
    };

    let ((contract, deploy_opt, smaps, contract_name), artifacts, warnings) =
        compile_fitsh_with_calcs(&source, Path::new(&file_path), &cfg, &cap)?;
    println!("Compile success!");

    for a in &artifacts {
        match &a.hcf1_path {
            Some(hcf1) => println!(
                "Calc kernel '{}': {} (profile {:#04x}, mem {} KiB, in {}, out {}, image {} B, id {}{})",
                a.name,
                hcf1.display(),
                a.profile_byte,
                a.mem_kib,
                a.input_max,
                a.output_max,
                a.image_len,
                &a.image_id[..16.min(a.image_id.len())],
                if a.cache_hit { " [cache]" } else { "" },
            ),
            None => println!(
                "Calc kernel '{}': built from {} (profile {:#04x}, mem {} KiB, in {}, out {}, image {} B)",
                a.name,
                a.source
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
                a.profile_byte,
                a.mem_kib,
                a.input_max,
                a.output_max,
                a.image_len,
            ),
        }
        if common.verbose {
            println!(
                "    manifest: {}.calc.manifest.json (code {} B, rodata {} B, init {} B, cap image max {} B)",
                a.name, a.code_len, a.rodata_len, a.init_len, cap.calc_image_max
            );
        }
    }
    for w in &warnings {
        println!("Warning: {w}");
    }

    let sto = contract.into_sto();
    if let Some(dir) = &common.out_dir {
        fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    // The contract map is a debugging aid, the deploy JSON is the essential
    // output: a read-only fitsh dir must not block deployment (route both to
    // --out-dir, or -o for the deploy JSON, in CI).
    if let Err(e) = write_contract_map(
        &file_path,
        &smaps,
        &contract_name,
        &sto,
        common.out_dir.as_deref(),
    ) {
        println!("Warning: contract map not written: {e}");
    }
    write_deploy_json(&file_path, &common, deploy_opt, sto)?;
    Ok(())
}

fn write_contract_map(
    file_path: &str,
    smaps: &[(String, vm::SourceMap)],
    name: &str,
    sto: &ContractSto,
    out_dir: Option<&Path>,
) -> Result<(), String> {
    let path = Path::new(file_path);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or("output".into());
    let parent = out_dir.unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")));
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
    .map_err(|e| format!("write {}: {e}", map_file.display()))?;
    println!("Generated: {}", map_file.display());
    Ok(())
}

fn write_deploy_json(
    file_path: &str,
    common: &CommonArgs,
    deploy_opt: Option<DeployInfo>,
    sto: ContractSto,
) -> Result<(), String> {
    let (d_fee, d_nonce, d_argv) = match deploy_opt {
        Some(info) => (info.protocol_cost, info.nonce, info.construct_argv),
        None => (None, None, None),
    };
    let fee_str = common
        .rest
        .get(1)
        .cloned()
        .unwrap_or_else(|| "1:248".to_string());
    let txfee = Amount::from(&fee_str).map_err(|e| format!("invalid fee {fee_str:?}: {e:?}"))?;
    let nonce_val = common
        .rest
        .get(2)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);
    let nonce = d_nonce.unwrap_or(Uint4::from(nonce_val));
    let argv = d_argv.unwrap_or_default();
    let charge_bytes = contract_deploy_charge_bytes(&sto) as u128;

    // Storage-fee basis: --periods (validated at parse) wins, then
    // HACASH_STORAGE_FEE_PERIODS, then full price.
    let full_periods = MAINNET_PARAMS.protocol.vm.contract_store_perm_periods;
    let period_floor = MAINNET_PARAMS.protocol.vm.contract_storage_fee.period_floor;
    let mut periods = full_periods;
    if let Some(p) = common.periods {
        periods = p;
    } else if let Ok(env_p) = env::var("HACASH_STORAGE_FEE_PERIODS") {
        periods = parse_storage_periods(&env_p, period_floor, full_periods).ok_or_else(|| {
            format!(
                "HACASH_STORAGE_FEE_PERIODS expects a multiple of {period_floor} in \
                 [{period_floor}, {full_periods}]"
            )
        })?;
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
    let parent = common
        .out_dir
        .as_deref()
        .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")));
    let deploy_file = match &common.out {
        Some(o) => PathBuf::from(o),
        None => parent.join(format!("{stem}.deploy.json")),
    };
    fs::write(
        &deploy_file,
        serde_json::to_string_pretty(&deploy_json).unwrap(),
    )
    .map_err(|e| format!("write {}: {e}", deploy_file.display()))?;
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
    Ok(())
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

/// Compile one `.rs` kernel and write the bytes `function rv64im = 0x` stores:
/// `CalcContainer::encode()`, admitted by `Func::rv64im_container_bytes`.
fn cmd_image(args: &[String]) -> Result<(), String> {
    let common = CommonArgs::parse(args, "image")?;
    if common.rest.len() != 1 {
        return Err("fitshc image requires exactly one <kernel.rs>".into());
    }
    let rs_path = PathBuf::from(&common.rest[0]);
    if rs_path.extension().and_then(|e| e.to_str()) != Some("rs") {
        return Err(format!(
            "fitshc image expects a .rs kernel, got {}",
            rs_path.display()
        ));
    }
    let name = rs_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("kernel file has no stem")?;
    let out = match &common.out {
        Some(o) => PathBuf::from(o),
        None => rs_path.with_extension("hcf1"),
    };
    if lex_abs(&out) == lex_abs(&rs_path) {
        return Err(format!(
            "output path {} would overwrite the input source; pass -o with a different path",
            out.display()
        ));
    }
    let source =
        fs::read_to_string(&rs_path).map_err(|e| format!("read {}: {e}", rs_path.display()))?;
    let meta = parse_kernel_meta(&rs_path, &source)?;
    let has_input = if common.no_input {
        false
    } else {
        infer_kernel_has_input(&source, name)?
    };
    let cap = SpaceCap::new(0);
    let built = KernelBuildRequest {
        name,
        source_path: &rs_path,
        meta: &meta,
        has_input,
        cap: &cap,
        strict: common.strict,
        cache_root: None,
        no_cache: common.no_cache,
    }
    .build()?;
    // Same gate as a FitSH `= 0x` body. Fail before any output file exists.
    vm::contract::Func::new(name)
        .and_then(|f| f.rv64im_container_bytes_with_cap(built.container_bytes.clone(), &cap))
        .map_err(|e| e.to_string())?;
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
    }
    fs::write(&out, &built.container_bytes).map_err(|e| format!("write {}: {e}", out.display()))?;
    let summary = format!(
        "image {} -> {} (profile {:#04x}, mem {} KiB, in {}, out {}, {} B)",
        rs_path.display(),
        out.display(),
        built.container.profile,
        built.mem_kib,
        built.container.input_max,
        built.container.output_max,
        built.container_bytes.len()
    );
    if common.hex {
        println!("0x{}", hex::encode(&built.container_bytes));
        eprintln!("{summary}");
    } else {
        println!("{summary}");
    }
    Ok(())
}

fn cmd_pack(args: &[String]) -> Result<(), String> {
    let common = CommonArgs::parse(args, "pack")?;
    if common.strict {
        return Err("--strict is not valid for `pack` (it compiles nothing)".into());
    }
    if common.no_build {
        return Err("--no-build is not valid for `pack` (it never builds anything)".into());
    }
    let elf_path = common
        .rest
        .first()
        .cloned()
        .ok_or_else(|| "fitshc pack requires a <kernel.elf>".to_string())?;
    let out = match common.out {
        Some(o) => PathBuf::from(o),
        None => Path::new(&elf_path).with_extension("hcf1"),
    };
    if out == Path::new(&elf_path) {
        return Err(format!(
            "output path {} would overwrite the input ELF; pass -o with a different path",
            out.display()
        ));
    }
    let cap = SpaceCap::new(0);
    let (c, bytes) =
        fitshc_cli::resolve::pack_elf_file(Path::new(&elf_path), &common.elf_defaults, &cap)?;
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
    let mut no_input_flag = false;
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
            "--no-input" => {
                no_input_flag = true;
                i += 1;
            }
            other => {
                filtered.push(other.to_string());
                i += 1;
            }
        }
    }
    let common = CommonArgs::parse(&filtered, "run")?;
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
            if common.no_build {
                return Err(
                    "--no-build cannot be combined with a .rs kernel (nothing to run)".into(),
                );
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("kernel file has no stem")?;
            let source =
                fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
            let meta = parse_kernel_meta(path, &source)?;
            // Without a fitsh declaration, infer the signature shape unless
            // the user pinned the zero-arg form with --no-input.
            let has_input = if no_input_flag {
                false
            } else {
                infer_kernel_has_input(&source, name)?
            };
            let built = KernelBuildRequest {
                name,
                source_path: path,
                meta: &meta,
                has_input,
                cap: &cap,
                strict: common.strict,
                cache_root: None,
                no_cache: common.no_cache,
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
            let (c, _) = fitshc_cli::resolve::pack_elf_file(path, &common.elf_defaults, &cap)?;
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
