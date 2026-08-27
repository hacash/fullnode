//! fitshc — FitSH source compiler CLI (ported from fullnodedev/src/bin/fitshc.rs).
//! Usage: `fitshc <file.fitsh> [fee] [nonce]`
//! Compiles a `.fitsh` contract, writes `<name>.contractmap.json` (functions +
//! source maps) and `<name>.deploy.json` (Type-3 `ContractDeploy` action body,
//! hex-encoded) for submission to a local node RPC.

use field::*;
use hacash_params::MAINNET_PARAMS;
use protocol::tx_std::StdTransaction;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use sys::{Account, curtimes};
use vm::action::ContractDeploy;
use vm::contract::ContractSto;
use vm::fitshc::compiler::compile;

fn estimate_protocol_cost_auto(
    txfee: &Amount,
    nonce: Uint4,
    argv: BytesW2,
    sto: &ContractSto,
) -> Amount {
    estimate_protocol_cost_auto_with_periods(
        txfee,
        nonce,
        argv,
        sto,
        sto.size() as u128,
        MAINNET_PARAMS.protocol.vm.contract_store_perm_periods,
    )
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
    let mut cur = Amount::unit238(1);
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
        let next = Amount::coin_u128(best_need.max(1), UNIT_238);
        if next == cur {
            break;
        }
        cur = next;
    }
    cur
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Usage: fitshc <file.fitsh> [fee] [nonce]");
        return;
    }
    let file_path = &args[1];
    let source = match fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(e) => {
            println!("Error reading file: {}", e);
            return;
        }
    };

    let (contract, deploy_opt, smaps, contract_name) = match compile(&source) {
        Ok(res) => res,
        Err(e) => {
            println!("Compile error: {:?}", e);
            return;
        }
    };

    println!("Compile success!");

    let path = Path::new(file_path);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or("output".into());
    let parent = path.parent().unwrap_or(Path::new("."));

    let sto = contract.into_sto();

    // Map
    let mut func_maps = Vec::new();
    for (name, sm) in smaps {
        if let Ok(json_str) = sm.to_json() {
            if let Ok(mut v) = serde_json::from_str::<Value>(&json_str) {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("name".to_string(), json!(name));
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
        "contract": contract_name,
        "inherits": inherit_addrs,
        "libs": lib_addrs,
        "funcs": func_maps
    });

    let map_file = parent.join(format!("{}.contractmap.json", stem));
    fs::write(
        &map_file,
        serde_json::to_string_pretty(&contract_map).unwrap(),
    )
    .ok();
    println!("Generated: {}", map_file.display());

    // Deploy
    let (d_fee, d_nonce, d_argv) = if let Some(info) = deploy_opt {
        (info.protocol_cost, info.nonce, info.construct_argv)
    } else {
        (None, None, None)
    };

    let fee_str = args.get(2).map(|s| s.as_str()).unwrap_or("1:248");
    let txfee = Amount::from(fee_str).unwrap_or(Amount::default());

    let nonce_val = args.get(3).and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
    let nonce = d_nonce.unwrap_or(Uint4::from(nonce_val));

    let argv = d_argv.unwrap_or_default();
    let protocol_cost =
        d_fee.unwrap_or_else(|| estimate_protocol_cost_auto(&txfee, nonce, argv.clone(), &sto));

    let mut action = ContractDeploy::default();
    action.protocol_cost = protocol_cost;
    action.nonce = nonce;
    action.construct_argv = argv;
    action.contract = sto;

    let action_body_bytes = action.encode();
    let deploy_json = json!({
        "action": hex::encode(&action_body_bytes)
    });

    let deploy_file = parent.join(format!("{}.deploy.json", stem));
    fs::write(
        &deploy_file,
        serde_json::to_string_pretty(&deploy_json).unwrap(),
    )
    .ok();
    println!("Generated: {}", deploy_file.display());
}
