//! Full HAC supply audit over a (copy of a) state database.
//!
//! Walks every KV entry of the state DB and reconciles the total HAC against
//! the issuance identity used by the `/query/supply` API:
//!
//!   circulation(H) = cumulative_block_reward(H) + channel_interest
//!                    - burned_fee - blackhole_burn
//!
//! where burned_fee = legacy tx extra9 burn + diamond-inscription burn + VM gas
//! burn + asset issue burn + contract protocol cost burn. The "held" side sums
//! every address balance (state key 0x0b + Address) plus every live payment
//! channel deposit (0x0c + ChannelId, status 0/1). Any other state namespace
//! that custody HAC would show up as a non-zero delta or an unexpected prefix.
//!
//! Known reconciling item: the one-time genesis seeding by
//! `mint::consensus::initialize` (0.109 HAC to five dev addresses) is NOT part
//! of the issuance formula, so a healthy ledger shows
//! delta = +0.109 HAC - rounding dust. Anything beyond that is a real bug.
//!
//! Usage:
//!   supply_audit <state_db_dir> [--dump out.tsv] [--top N]
//!   supply_audit --snapshot <live_state_dir> [--dump out.tsv] [--top N]
//!
//! `--snapshot` copies a live node's state DB (which holds the leveldb LOCK)
//! to a temp dir with retries, audits the copy, then deletes the copy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base::{BaseTotal, DiskDB};
use db::DiskKV;
use field::{Address, Balance, ChannelSto, Decode};
use mint::GENESIS_INIT_TOTAL_238;
use mint_core::state::MintTotal;

const UNIT_238: u128 = 100_0000_0000;

const KEY_BASE_TOTAL: u8 = 0x01;
const KEY_BALANCE: u8 = 0x0b;
const KEY_CHANNEL: u8 = 0x0c;

/// A full-file copy of a live leveldb can catch a compaction mid-flight; the
/// open/iteration then fails and a fresh copy usually succeeds.
const SNAPSHOT_ATTEMPTS: usize = 3;

fn fmt_hac(v238: u128) -> String {
    let int = v238 / UNIT_238;
    let frac = v238 % UNIT_238;
    format!("{}.{:010}", int, frac)
}

fn signed_hac(delta: i128) -> String {
    if delta < 0 {
        format!("-{}", fmt_hac((-delta) as u128))
    } else {
        fmt_hac(delta as u128)
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copy a live state DB into a fresh temp dir, retrying on torn copies.
/// Returns the copy path; caller removes it after the audit.
fn snapshot_live(src: &Path) -> PathBuf {
    let dst = std::env::temp_dir().join(format!(
        "hacash_state_audit_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    for attempt in 1..=SNAPSHOT_ATTEMPTS {
        let _ = std::fs::remove_dir_all(&dst);
        match copy_dir_recursive(src, &dst) {
            Ok(()) => {
                eprintln!("[audit] snapshot copy ok (attempt {}): {}", attempt, dst.display());
                return dst;
            }
            Err(e) => eprintln!("[audit] snapshot copy attempt {} failed: {}", attempt, e),
        }
    }
    panic!("snapshot copy failed after {} attempts", SNAPSHOT_ATTEMPTS);
}

struct BalanceRow {
    addr: Address,
    hac238: u128,
    balance: Balance,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dump_path: Option<String> = None;
    let mut top_n: usize = 20;
    let mut snapshot_src: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => {
                dump_path = Some(args.get(i + 1).cloned().expect("--dump needs a path"));
                args.drain(i..=i + 1);
            }
            "--top" => {
                top_n = args
                    .get(i + 1)
                    .cloned()
                    .expect("--top needs N")
                    .parse()
                    .expect("N");
                args.drain(i..=i + 1);
            }
            "--snapshot" => {
                snapshot_src = Some(args.get(i + 1).cloned().expect("--snapshot needs a dir"));
                args.drain(i..=i + 1);
            }
            _ => i += 1,
        }
    }

    let temp_copy: Option<PathBuf> = snapshot_src.map(|src| {
        let p = Path::new(&src).to_path_buf();
        snapshot_live(&p)
    });
    let dir = temp_copy
        .clone()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            args.first()
                .expect(
                    "usage: supply_audit <state_db_dir> [--snapshot live_dir] [--dump out.tsv] [--top N]",
                )
                .clone()
        });

    let audit_result = run_audit(&dir, dump_path, top_n);

    if let Some(tmp) = temp_copy {
        let _ = std::fs::remove_dir_all(&tmp);
        eprintln!("[audit] temp snapshot removed: {}", tmp.display());
    }

    audit_result.expect("audit failed");
}

fn run_audit(dir: &str, dump_path: Option<String>, top_n: usize) -> sys::Ret<()> {
    let kv = DiskKV::open(Path::new(dir))?;
    eprintln!("[audit] opened {}", dir);

    let mut root_height: u64 = 0;
    let mut root_hash_hex: Option<String> = None;
    let mut base_total_raw: Option<Vec<u8>> = None;
    let mut mint_total_raw: Option<Vec<u8>> = None;
    let mut text_keys: Vec<String> = Vec::new();

    let mut balances: Vec<BalanceRow> = Vec::new();
    let mut balance_decode_errors: usize = 0;
    let mut zero_hac_addresses: usize = 0;

    let mut channels: u64 = 0;
    let mut channel_by_status: BTreeMap<u64, u64> = BTreeMap::new();
    let mut channel_deposit_by_status: BTreeMap<u64, u128> = BTreeMap::new();
    let mut channel_decode_errors: usize = 0;

    let mut prefix_count: BTreeMap<u32, u64> = BTreeMap::new(); // first byte; 0x100 = text keys

    kv.for_each(&mut |k: &[u8], v: &[u8]| {
        if k.is_empty() {
            *prefix_count.entry(0xFFFF).or_insert(0) += 1;
            return;
        }
        match k[0] {
            0x5f => {
                *prefix_count.entry(0x100).or_insert(0) += 1;
                let key = String::from_utf8_lossy(k).to_string();
                match key.as_str() {
                    "_chain.root_height" => {
                        if v.len() == 8 {
                            root_height = u64::from_be_bytes(v.try_into().unwrap());
                        }
                    }
                    "_chain.root_hash" => {
                        root_hash_hex = Some(
                            field::Hash::decode(v)
                                .map(|(h, _)| hex::encode(h.as_bytes()))
                                .unwrap_or_else(|_| "<undecodable>".into()),
                        );
                    }
                    "_mint.total" => mint_total_raw = Some(v.to_vec()),
                    _ => {}
                }
                text_keys.push(key);
            }
            KEY_BASE_TOTAL if k.len() == 1 => {
                *prefix_count.entry(KEY_BASE_TOTAL as u32).or_insert(0) += 1;
                base_total_raw = Some(v.to_vec());
            }
            KEY_BALANCE => {
                *prefix_count.entry(KEY_BALANCE as u32).or_insert(0) += 1;
                let (addr, used) = match Address::decode(&k[1..]) {
                    Ok(x) => x,
                    Err(_) => {
                        balance_decode_errors += 1;
                        return;
                    }
                };
                if used != k.len() - 1 {
                    balance_decode_errors += 1;
                    return;
                }
                match Balance::decode(v) {
                    Ok((bal, _)) => match bal.hacash.to_238_u128() {
                        Ok(hac) => {
                            if hac == 0 {
                                zero_hac_addresses += 1;
                            }
                            balances.push(BalanceRow { addr, hac238: hac, balance: bal });
                        }
                        Err(_) => balance_decode_errors += 1,
                    },
                    Err(_) => balance_decode_errors += 1,
                }
            }
            KEY_CHANNEL => {
                *prefix_count.entry(KEY_CHANNEL as u32).or_insert(0) += 1;
                match ChannelSto::decode(v) {
                    Ok((sto, _)) => {
                        channels += 1;
                        let st = sto.status.uint() as u64;
                        *channel_by_status.entry(st).or_insert(0) += 1;
                        let dep = sto
                            .left_bill
                            .balance
                            .hacash
                            .to_238_u128()
                            .unwrap_or(0)
                            .saturating_add(sto.right_bill.balance.hacash.to_238_u128().unwrap_or(0));
                        *channel_deposit_by_status.entry(st).or_insert(0) += dep;
                    }
                    Err(_) => channel_decode_errors += 1,
                }
            }
            first => {
                *prefix_count.entry(first as u32).or_insert(0) += 1;
            }
        }
    })
    ?;

    text_keys.sort();
    text_keys.dedup();

    let base_total: BaseTotal = match base_total_raw {
        Some(raw) => BaseTotal::decode(&raw).expect("BaseTotal decode").0,
        None => Default::default(),
    };
    let mint_total: MintTotal = match mint_total_raw {
        Some(raw) => MintTotal::decode(&raw).expect("MintTotal decode").0,
        None => Default::default(),
    };

    balances.sort_by(|a, b| {
        b.hac238
            .cmp(&a.hac238)
            .then_with(|| a.addr.to_readable().cmp(&b.addr.to_readable()))
    });
    let sum_balances: u128 = balances.iter().map(|b| b.hac238).sum();
    let deposit_open = channel_deposit_by_status.get(&0).copied().unwrap_or(0);
    let deposit_challenging = channel_deposit_by_status.get(&1).copied().unwrap_or(0);
    let deposit_closed: u128 = channel_deposit_by_status
        .iter()
        .filter(|(st, _)| **st >= 2)
        .map(|(_, v)| *v)
        .sum();

    // ---- issuance identity (mirrors mint/src/api/util/supply.rs) ----
    let height = root_height;
    let block_reward_238 = mint::minter::cumulative_block_reward(height) as u128 * UNIT_238;
    let burned_fee: u128 = base_total.tx_fee_burn90_238.uint() as u128
        + base_total.ast_vm_gas_burn_238.uint() as u128
        + base_total.contract_protocol_cost_burn_238.uint() as u128
        + mint_total.asset_issue_burn_238.uint() as u128
        + mint_total.diamond_insc_burn_238.uint() as u128;
    let blackhole: u128 = base_total.blackhole_hac_burn_238.uint() as u128;
    let interest: u128 = mint_total.channel_interest_238.uint() as u128;
    let circulation = block_reward_238 + interest - burned_fee - blackhole;
    let held = sum_balances + deposit_open + deposit_challenging;
    let delta = held as i128 - circulation as i128;

    println!("================ HAC SUPPLY AUDIT ================");
    println!(
        "state root height: {}  root_hash: {}",
        height,
        root_hash_hex.as_deref().unwrap_or("<missing>")
    );
    println!("text keys present: {:?}", text_keys);
    println!();
    println!("--- issuance side (1 HAC = 1e10 in 238 units) ---");
    println!(
        "cumulative_block_reward({}) = {} HAC",
        height,
        fmt_hac(block_reward_238)
    );
    println!("channel_interest           = {} HAC", fmt_hac(interest));
    println!("burned_fee                 = {} HAC", fmt_hac(burned_fee));
    println!(
        "  tx_fee_burn90            = {} HAC",
        fmt_hac(base_total.tx_fee_burn90_238.uint() as u128)
    );
    println!(
        "  ast_vm_gas               = {} HAC",
        fmt_hac(base_total.ast_vm_gas_burn_238.uint() as u128)
    );
    println!(
        "  contract_protocol_cost   = {} HAC",
        fmt_hac(base_total.contract_protocol_cost_burn_238.uint() as u128)
    );
    println!(
        "  asset_issue              = {} HAC",
        fmt_hac(mint_total.asset_issue_burn_238.uint() as u128)
    );
    println!(
        "  diamond_insc             = {} HAC",
        fmt_hac(mint_total.diamond_insc_burn_238.uint() as u128)
    );
    println!("blackhole_burn             = {} HAC", fmt_hac(blackhole));
    println!("=> current_circulation     = {} HAC", fmt_hac(circulation));
    println!();
    println!("--- held side ---");
    println!("addresses with balance entry : {}", balances.len());
    println!("  zero-HAC entries           : {}", zero_hac_addresses);
    println!("SUM(address balances)        = {} HAC", fmt_hac(sum_balances));
    println!("channels total               : {}", channels);
    for (st, n) in &channel_by_status {
        println!(
            "  status {}: {} channels, deposits {} HAC",
            st,
            n,
            fmt_hac(channel_deposit_by_status.get(st).copied().unwrap_or(0))
        );
    }
    println!("SUM(open channel deposits)   = {} HAC", fmt_hac(deposit_open));
    if deposit_challenging != 0 {
        println!(
            "SUM(challenging channel dep) = {} HAC",
            fmt_hac(deposit_challenging)
        );
    }
    if deposit_closed != 0 {
        println!(
            "SUM(closed channel residual) = {} HAC  (excluded from HELD)",
            fmt_hac(deposit_closed)
        );
    }
    println!("=> HELD TOTAL                = {} HAC", fmt_hac(held));
    println!();
    println!("DELTA (held - circulation)   = {} HAC", signed_hac(delta));
    println!(
        "cross-check MintTotal.channel_deposit_238 = {} HAC",
        fmt_hac(mint_total.channel_deposit_238.uint() as u128)
    );
    println!(
        "decode errors: balance={} channel={}",
        balance_decode_errors, channel_decode_errors
    );
    println!();
    println!("--- reconciliation verdict ---");
    println!(
        "known genesis init allocation = {} HAC (mint::consensus::initialize, not in issuance formula)",
        fmt_hac(GENESIS_INIT_TOTAL_238)
    );
    let unexplained = delta - GENESIS_INIT_TOTAL_238 as i128;
    println!(
        "unexplained (delta - init)    = {} HAC",
        signed_hac(unexplained)
    );
    // dust threshold: 0.0001 HAC covers historical unit-conversion rounding
    // (interest/fee conversions truncate at 10^-10); anything beyond it is a bug.
    const DUST_LIMIT_238: i128 = 1_000_000;
    if unexplained.abs() <= DUST_LIMIT_238 {
        println!("RESULT: LEDGER BALANCED - no inflation beyond the known genesis allocation.");
    } else if unexplained > 0 {
        println!(
            "RESULT: UNEXPLAINED EXTRA HAC {} - possible minting bug, investigate!",
            signed_hac(unexplained)
        );
    } else {
        println!(
            "RESULT: HAC MISSING {} - over-burn or lost custody, investigate.",
            signed_hac(-unexplained)
        );
    }
    println!();
    println!("--- state key prefix histogram (numeric first byte, 0x100 = text keys) ---");
    for (p, n) in &prefix_count {
        if *p == 0x100 {
            println!("  TEXT(_) : {}", n);
        } else {
            println!("  0x{:02x}    : {}", p, n);
        }
    }
    println!();
    println!("--- top {} holders ---", top_n);
    for b in balances.iter().take(top_n) {
        println!("  {}  {} HAC", b.addr.to_readable(), fmt_hac(b.hac238));
    }

    if let Some(path) = dump_path {
        let mut out = String::from("# address\thac_238\thac\t satoshi\t diamond\t assets\n");
        for b in &balances {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                b.addr.to_readable(),
                b.hac238,
                fmt_hac(b.hac238),
                b.balance.satoshi.uint(),
                b.balance.diamond.uint(),
                b.balance.assets.length(),
            ));
        }
        std::fs::write(&path, out).expect("write dump");
        eprintln!("[audit] wrote {} balance rows to {}", balances.len(), path);
    }

    eprintln!("[audit] done");
    Ok(())
}
