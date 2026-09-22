//! P2SH-era audit over a local block store.
//!
//! For every block in `[start..=end]` it walks transactions and actions and reports:
//!   * every `P2SHScriptProve` (action kind 46) lock script, with its derived `SCRIPTMH` address;
//!   * every `SCRIPTMH` address that money was sent *to* (declared transfer intents);
//!   * the set of `SCRIPTMH` addresses that received value but were never revealed by a
//!     kind-46 action — i.e. funded P2SH addresses whose lock script is still unknown.
//!
//! Usage: `p2sh_scan <block_dir> <start_height> <end_height> [lockboxes.tsv]`

use std::collections::BTreeMap;

use base::{BinaryCodecs, DiskDB, TransferAsset};
use field::Address;
use sys::ToHex;

const KEY_PREFIX_BLOCK: u8 = 0x01;
const KEY_PREFIX_INDEX: u8 = 0x02;
const KEY_CURSOR: &[u8] = b"_block.cursor";

fn block_key(hash: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(33);
    v.push(KEY_PREFIX_BLOCK);
    v.extend_from_slice(hash);
    v
}

fn height_key(height: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(9);
    v.push(KEY_PREFIX_INDEX);
    v.extend_from_slice(&height.to_be_bytes());
    v
}

/// Collect every base58check token in `json` that decodes to a SCRIPTMH address.
/// Safety net for action kinds that move value without a declared `transfer_intent`
/// (diamond bid payouts, channels, TEX settlement, ...): any such address must
/// still appear somewhere in the action payload.
fn json_scriptmh_addrs(json: &str, out: &mut Vec<Address>) {
    let is_b58 = |c: char| c.is_ascii_alphanumeric() && !matches!(c, '0' | 'O' | 'I' | 'l');
    let chars: Vec<char> = json.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if !is_b58(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_b58(chars[i]) {
            i += 1;
        }
        let run: String = chars[start..i].iter().collect();
        let rl = run.chars().count();
        let mut hit = false;
        if rl >= 20 {
            if let Ok(a) = Address::from_readable(&run) {
                if a.is_scriptmh() {
                    out.push(a);
                    hit = true;
                }
            }
        }
        // a 21-byte base58check payload renders as ~34 chars; sliding windows also
        // catch a token glued to neighbouring base58 characters in one JSON string
        if !hit {
            for len in 30usize..=36 {
                if len > rl {
                    break;
                }
                for off in 0..=(rl - len) {
                    let win: String = run.chars().skip(off).take(len).collect();
                    if let Ok(a) = Address::from_readable(&win) {
                        if a.is_scriptmh() {
                            out.push(a);
                        }
                    }
                }
            }
        }
    }
}

/// Flatten an action together with its nested (AST / guard branch) children.
fn flatten_action<'a>(act: &'a dyn base::Action, out: &mut Vec<&'a dyn base::Action>) {
    out.push(act);
    if let Some(nested) = act.nested_actions() {
        for child in nested.flatten() {
            flatten_action(child, out);
        }
    }
}

/// Per-address tally of value declared as flowing *into* a SCRIPTMH address.
#[derive(Clone, Default)]
struct Recv {
    first_h: u64,
    last_h: u64,
    events: usize,
    hac_fin: Vec<String>,
    sat_total: u128,
    hacd_names: usize,
    asset_events: usize,
    detail: Vec<String>,
}

impl Recv {
    fn add(&mut self, height: u64, action_kind: u16, asset: &TransferAsset) {
        if self.events == 0 {
            self.first_h = height;
        }
        self.last_h = height;
        self.events += 1;
        let desc = match asset {
            TransferAsset::Hac(a) => {
                let s = a.to_unit_string("HAC");
                self.hac_fin.push(s.clone());
                format!("HAC {}", s)
            }
            TransferAsset::Sat(s) => {
                self.sat_total += s.uint() as u128;
                format!("SAT {}", s.uint())
            }
            TransferAsset::Diamond(list) => {
                self.hacd_names += list.length();
                format!("HACD x{}", list.length())
            }
            TransferAsset::Asset(a) => {
                self.asset_events += 1;
                format!("ASSET serial={} amount={}", a.serial.uint(), a.amount.uint())
            }
        };
        if self.detail.len() < 16 {
            self.detail
                .push(format!("h={} kind={} {}", height, action_kind, desc));
        }
    }

    fn summary(&self) -> String {
        let mut parts = Vec::new();
        if !self.hac_fin.is_empty() {
            parts.push(format!("HAC x{} [{}]", self.hac_fin.len(), self.hac_fin.join(" | ")));
        }
        if self.sat_total > 0 {
            parts.push(format!("SAT total {}", self.sat_total));
        }
        if self.hacd_names > 0 {
            parts.push(format!("HACD names {}", self.hacd_names));
        }
        if self.asset_events > 0 {
            parts.push(format!("ASSET events {}", self.asset_events));
        }
        parts.join("  ")
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .unwrap_or_else(|| "/tmp/p2sh_scan/block".to_string());
    let start: u64 = args
        .next()
        .map(|s| s.parse().expect("start height"))
        .unwrap_or(0);
    let end: Option<u64> = args.next().map(|s| s.parse().expect("end height"));
    let out_path = args.next();

    let kv = db::DiskKV::open(std::path::Path::new(&dir)).expect("open block store");
    let tip = kv
        .read(KEY_CURSOR)
        .expect("cursor read")
        .map(|b| u64::from_be_bytes(b[..8].try_into().expect("cursor bytes")))
        .unwrap_or(0);
    eprintln!("[scan] block store tip cursor = {}", tip);

    let registry = app::standard_registry().expect("standard registry");
    let end = end.unwrap_or(tip).min(tip);

    let mut per_height: Vec<(u64, u64, String, usize)> = Vec::new();
    let mut lines = Vec::new();
    let mut kind_hist: BTreeMap<u16, usize> = BTreeMap::new();

    // SCRIPTMH addresses revealed by a kind-46 action: addr -> (first height, count)
    let mut revealed: BTreeMap<Address, (u64, usize)> = BTreeMap::new();
    // SCRIPTMH addresses value was sent to: addr -> tally
    let mut received: BTreeMap<Address, Recv> = BTreeMap::new();
    // SCRIPTMH addresses that paid out (should always be accompanied by a reveal in the same tx)
    let mut paid_out: BTreeMap<Address, (u64, usize)> = BTreeMap::new();
    // SCRIPTMH addresses appearing as tx main / addrlist entries
    let mut tx_addr_seen: BTreeMap<Address, (u64, usize)> = BTreeMap::new();
    // SCRIPTMH addresses found only by the payload sweep: addr -> (first h, action kind, count)
    let mut json_seen: BTreeMap<Address, (u64, u16, usize)> = BTreeMap::new();
    let mut reveal_errors = 0usize;
    let mut scanned = 0u64;
    // sweep self-test counters (see the positive control in the action loop)
    let mut control_total = 0usize;
    let mut control_hit = 0usize;
    let mut control_miss = 0usize;

    for height in start..=end {
        let Some(raw_hash) = kv
            .read(&height_key(height))
            .unwrap_or_else(|e| panic!("height index {height}: {e}"))
        else {
            eprintln!("[scan] no index entry at {height}, stopping");
            break;
        };
        let data = kv
            .read(&block_key(&raw_hash))
            .unwrap_or_else(|e| panic!("block {height}: {e}"))
            .unwrap_or_else(|| panic!("block body missing at {height}"));
        let (block, _used) = registry
            .decode_block(&data)
            .unwrap_or_else(|e| panic!("decode block {height}: {e}"));
        scanned += 1;
        let mut hits = 0usize;

        for tx in block.transactions() {
            let main = tx.main();
            let addrs = tx.addrs();
            for a in std::iter::once(main).chain(addrs.iter().copied()) {
                if a.is_scriptmh() {
                    let e = tx_addr_seen.entry(a).or_insert((height, 0));
                    e.0 = e.0.min(height);
                    e.1 += 1;
                }
            }
            let mut flat: Vec<&dyn base::Action> = Vec::with_capacity(tx.actions().len());
            for act in tx.actions() {
                flatten_action(act.as_ref(), &mut flat);
            }
            for act in flat {
                *kind_hist.entry(act.kind()).or_insert(0) += 1;

                let intent = act.transfer_intent();
                if let Some(intent) = intent {
                    if let Ok((from, to)) = intent.resolve_with(main, |p| p.real(&addrs)) {
                        if to.is_scriptmh() {
                            received
                                .entry(to)
                                .or_default()
                                .add(height, act.kind(), &intent.asset);
                        }
                        if from.is_scriptmh() {
                            let e = paid_out.entry(from).or_insert((height, 0));
                            e.0 = e.0.min(height);
                            e.1 += 1;
                        }
                        // Positive control: the payload sweep must be able to see a SCRIPTMH
                        // address we already know is inside this action's JSON, otherwise a
                        // "0 found" sweep result would only mean the detector is broken.
                        if to.is_scriptmh() || from.is_scriptmh() {
                            control_total += 1;
                            let mut found = Vec::new();
                            json_scriptmh_addrs(&act.to_json(), &mut found);
                            if found.contains(&to) || found.contains(&from) {
                                control_hit += 1;
                            } else {
                                control_miss += 1;
                            }
                        }
                    }
                } else if act.kind() != vm::action::P2SHScriptProve::KIND {
                    // Non-transfer action: sweep its payload for SCRIPTMH addresses so
                    // protocol payouts (diamond bid / channel / TEX) are not missed.
                    let mut found = Vec::new();
                    json_scriptmh_addrs(&act.to_json(), &mut found);
                    for a in found {
                        let e = json_seen.entry(a).or_insert((height, act.kind(), 0));
                        e.0 = e.0.min(height);
                        e.2 += 1;
                    }
                }

                if act.kind() != vm::action::P2SHScriptProve::KIND {
                    continue;
                }
                let Some(p) = act.as_any().downcast_ref::<vm::action::P2SHScriptProve>() else {
                    continue;
                };
                hits += 1;
                let lockbox = p.lockbox.as_vec();
                lines.push(format!(
                    "{}\t{}\t{}\t{}\t{}",
                    height,
                    hex::encode(tx.hash().as_bytes()),
                    p.codeconf.uint(),
                    lockbox.len(),
                    lockbox.to_hex(),
                ));
                let addr: Result<Address, String> =
                    match vm::CodeConf::parse(p.codeconf.uint()) {
                        Ok(conf) => vm::action::P2SHScriptProve::calc_scriptmh_from_lockbox(
                            &p.adrlibs,
                            conf,
                            &p.lockbox,
                            &p.merkels,
                        )
                        .map(|calc| calc.address)
                        .map_err(|e| e.to_string()),
                        Err(e) => Err(e.to_string()),
                    };
                match addr {
                    Ok(a) => {
                        let e = revealed.entry(a).or_insert((height, 0));
                        e.0 = e.0.min(height);
                        e.1 += 1;
                    }
                    Err(e) => {
                        reveal_errors += 1;
                        eprintln!("[scan]   h={} scriptmh derive failed: {}", height, e);
                    }
                }
            }
        }
        if hits > 0 {
            per_height.push((
                height,
                block.timestamp(),
                hex::encode(block.hash().as_bytes()),
                hits,
            ));
        }
    }

    eprintln!("[scan] scanned {} blocks [{}..={}]", scanned, start, end);
    let total: usize = per_height.iter().map(|r| r.3).sum();
    eprintln!("[scan] p2sh actions = {}  (derive failures: {})", total, reveal_errors);
    for (h, ts, hash, n) in per_height.iter().take(20) {
        eprintln!("[scan]   h={} ts={} n={} {}", h, ts, n, hash);
    }
    eprintln!("[scan] action kind histogram (kind -> count):");
    for (k, n) in kind_hist.iter() {
        eprintln!("[scan]   kind {:>4} (0x{:x}) -> {}", k, k, n);
    }

    // ---- the deliverable: funded but never revealed SCRIPTMH addresses ----
    println!("# SCRIPTMH addresses observed in [{}..={}]", start, end);
    println!(
        "# revealed by kind-46: {} distinct / {} actions",
        revealed.len(),
        revealed.values().map(|v| v.1).sum::<usize>()
    );
    println!(
        "# received value (transfer destinations): {} distinct",
        received.len()
    );
    println!(
        "# paid out from a SCRIPTMH: {} distinct   tx main/addrlist holds a SCRIPTMH: {} distinct",
        paid_out.len(),
        tx_addr_seen.len()
    );
    println!(
        "# sweep positive control: {} of {} known SCRIPTMH mentions re-found (missed {})",
        control_hit, control_total, control_miss
    );
    println!(
        "# SCRIPTMH found by payload sweep of non-transfer actions (channel/diamond/TEX): {} distinct",
        json_seen.len()
    );
    for (a, (h, kind, n)) in json_seen.iter() {
        println!(
            "PAYLOAD_SEEN\t{}\tfirst_h={}\taction_kind={}\tcount={}\talso_received={}\talso_revealed={}",
            a.to_readable(),
            h,
            kind,
            n,
            received.contains_key(a),
            revealed.contains_key(a),
        );
    }

    let orphan: Vec<(&Address, &Recv)> = received
        .iter()
        .filter(|(a, _)| !revealed.contains_key(*a))
        .collect();
    println!(
        "# >>> funded but NEVER revealed (lock script still unknown): {} distinct",
        orphan.len()
    );
    for (a, r) in orphan.iter() {
        println!("ORPHAN\t{}\tfirst_h={}\tlast_h={}\tevents={}\t{}", a.to_readable(), r.first_h, r.last_h, r.events, r.summary());
        for d in r.detail.iter() {
            println!("ORPHAN_DETAIL\t{}\t{}", a.to_readable(), d);
        }
    }

    println!("# --- all SCRIPTMH addresses: funded (transfer intent) vs revealed (kind-46) ---");
    for (a, r) in received.iter() {
        let rev = revealed.get(a);
        println!(
            "PAIR\t{}\tfunded_first_h={}\tfunded_events={}\trevealed={}\treveal_first_h={}\treveal_count={}\t{}",
            a.to_readable(),
            r.first_h,
            r.events,
            rev.is_some(),
            rev.map(|v| v.0).unwrap_or(0),
            rev.map(|v| v.1).unwrap_or(0),
            r.summary(),
        );
        for d in r.detail.iter() {
            println!("PAIR_DETAIL\t{}\t{}", a.to_readable(), d);
        }
    }
    for a in revealed.keys().filter(|a| !received.contains_key(*a)) {
        let (h, n) = revealed[a];
        println!(
            "PAIR\t{}\tfunded_first_h=NEVER\tfunded_events=0\trevealed=true\treveal_first_h={}\treveal_count={}",
            a.to_readable(), h, n
        );
    }

    let revealed_only: Vec<&Address> = revealed
        .keys()
        .filter(|a| !received.contains_key(*a))
        .collect();
    println!(
        "# revealed but never a transfer destination: {} distinct",
        revealed_only.len()
    );
    for a in revealed_only {
        let (h, n) = revealed[a];
        println!("REVEAL_ONLY\t{}\tfirst_h={}\tcount={}", a.to_readable(), h, n);
    }

    if let Some(path) = out_path {
        std::fs::write(&path, lines.join("\n") + "\n").expect("write out");
        eprintln!("[scan] wrote {} lockbox rows to {}", lines.len(), path);
    }
}
