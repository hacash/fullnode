//! Deploy-time verifier + pre-decode + basic-block cycle pricing.
//!
//! `verify_container` is run once at deployment (and re-run idempotently on
//! load); it rejects anything outside the profile whitelist, off-boundary
//! jump targets and malformed headers, and produces the executable
//! [`CalcModule`] used by the interpreter.
//!
//! There is deliberately no reachability requirement (CKB-VM likewise
//! executes whatever decodes): LLVM/rustc output routinely contains code
//! only reachable through rodata jump tables (dense `match`), inter-function
//! alignment padding and function-pointer-only targets, and runtime safety
//! never depends on reachability — every dynamic jump target is
//! boundary-checked by `inst_at_addr` and every memory access is
//! bounds-checked in the interpreter. Unreachable bytes still have to pass
//! the whitelist decode; they are simply never executed and never charged.

use crate::calc::container::{CalcContainer, CODE_BASE};
use crate::calc::decode::{self, Inst};
use crate::rt::{ItrErr, ItrErrCode, VmrtRes};

fn err(t: &str) -> ItrErr {
    ItrErr::new(ItrErrCode::ContractError, t)
}

/// One basic block: contiguous instruction-index range plus its static
/// cycle cost, charged in full when the block is entered.
#[derive(Debug, Clone)]
pub struct Block {
    /// inclusive start instruction index
    pub start: u32,
    /// exclusive end instruction index
    pub end: u32,
    pub cycles: u64,
}

/// Verified, pre-decoded calc module. `insts[i]` is at absolute guest
/// address `addrs[i]` (`CODE_BASE + code-offset`); branch/JAL targets are
/// rewritten to instruction indices.
#[derive(Debug, Clone)]
pub struct CalcModule {
    #[allow(dead_code)]
    pub(crate) profile: u8,
    pub(crate) layout: crate::calc::container::CalcLayout,
    pub(crate) insts: Vec<Inst>,
    /// Byte address of each instruction (strictly increasing).
    pub(crate) addrs: Vec<u32>,
    /// Basic blocks in address order.
    pub(crate) blocks: Vec<Block>,
    /// Block index containing each instruction.
    pub(crate) block_of_inst: Vec<u32>,
    /// Cycle cost from instruction `i` to the end of its containing block.
    /// Charged at block entry; for the first instruction of a block this
    /// equals the block's static cost, while a dynamic `jalr` into the middle
    /// of a block pays only for the remaining instructions.
    pub(crate) suffix_cycles: Vec<u64>,
    /// Instruction index of the entry point.
    pub(crate) entry: u32,
    /// Captured from [`crate::calc::CalcConfig`] at verify time.
    pub(crate) hash_builtins: bool,
    pub(crate) rodata_readonly: bool,
    pub(crate) stack_guard: bool,
}

impl CalcModule {
    /// Test-oracle view of the deterministic memory layout.
    #[doc(hidden)]
    pub fn layout_snapshot(&self) -> crate::calc::container::CalcLayout {
        self.layout
    }

    /// Resolve a runtime jump target (byte address) to an instruction index;
    /// only exact instruction boundaries are accepted.
    pub fn inst_at_addr(&self, addr: u64) -> Option<u32> {
        if addr > u32::MAX as u64 {
            return None;
        }
        self.addrs.binary_search(&(addr as u32)).ok().map(|i| i as u32)
    }
}

pub fn verify_container(c: &CalcContainer) -> VmrtRes<CalcModule> {
    verify_container_with(c, &crate::calc::CALC_CONFIG_V1)
}

pub fn verify_container_with(
    c: &CalcContainer,
    cfg: &crate::calc::CalcConfig,
) -> VmrtRes<CalcModule> {
    // Header checks must run here too: a directly constructed container did
    // not pass through `decode`.
    c.validate_header(cfg)?;
    verify_decoded(c, cfg, c.layout())
}

fn verify_decoded(
    c: &CalcContainer,
    cfg: &crate::calc::CalcConfig,
    layout: crate::calc::container::CalcLayout,
) -> VmrtRes<CalcModule> {
    if crate::calc::container::profile_has_bitmanip(c.profile) && !cfg.profile1_bitmanip {
        return Err(err("calc verify: profile not enabled"));
    }
    if crate::calc::container::profile_has_compressed(c.profile) && !cfg.compressed {
        return Err(err("calc verify: profile not enabled"));
    }
    // Header, layout overlap and entry alignment already ran in
    // `validate_header` → `validate_invariants`.
    let entry_u = c.entry as u64;
    if !cfg.free_entry && entry_u != CODE_BASE {
        return Err(err("calc verify: entry must be CODE_BASE"));
    }
    // Pass 1: linear scan decode + boundary map. Every byte of the code
    // section must decode to a whitelisted instruction — including bytes
    // that will never execute (padding, cold paths).
    let code = &c.code;
    let mut insts: Vec<Inst> = Vec::new();
    let mut addrs: Vec<u32> = Vec::new();
    let mut boundaries: Vec<bool> = vec![false; code.len() + 1];
    let mut off = 0usize;
    while off < code.len() {
        boundaries[off] = true;
        let (inst, len) = decode::decode_at(code, off, c.profile)?;
        let ok_len = if crate::calc::container::profile_has_compressed(c.profile) {
            len == 2 || len == 4
        } else {
            len == 4
        };
        if !ok_len {
            return Err(err("calc verify: instruction length not in profile"));
        }
        addrs.push((CODE_BASE + off as u64) as u32);
        insts.push(inst);
        off += len as usize;
    }
    // Pass 2: resolve static jump targets to absolute guest addresses, then
    // to instruction indices; targets must land on instruction boundaries.
    let addr_idx: Vec<(u32, u32)> =
        addrs.iter().enumerate().map(|(i, a)| (*a, i as u32)).collect();
    let resolve = |abs: i64| -> VmrtRes<u32> {
        if abs < CODE_BASE as i64
            || abs > (CODE_BASE + code.len() as u64) as i64
        {
            return Err(err("calc verify: jump target not on instruction boundary"));
        }
        let rel = (abs as u64 - CODE_BASE) as usize;
        if rel > code.len() || !boundaries[rel] {
            return Err(err("calc verify: jump target not on instruction boundary"));
        }
        match addr_idx.binary_search_by_key(&(abs as u32), |(a, _)| *a) {
            Ok(i) => Ok(addr_idx[i].1),
            Err(_) => Err(err("calc verify: jump target not on instruction boundary")),
        }
    };
    for (i, inst) in insts.iter_mut().enumerate() {
        match inst {
            Inst::Jal { target, .. } | Inst::Branch { target, .. } => {
                let abs = (addrs[i] as i64)
                    .checked_add(*target)
                    .ok_or_else(|| err("calc verify: jump target overflow"))?;
                *target = resolve(abs)? as i64;
            }
            _ => {}
        }
    }
    // Entry must be a boundary (absolute PC).
    let entry_idx = match addr_idx.binary_search_by_key(&c.entry, |(a, _)| *a) {
        Ok(i) => addr_idx[i].1 as usize,
        Err(_) => return Err(err("calc verify: entry not on instruction boundary")),
    };
    // Pass 3: split into basic blocks. Leaders: entry, branch/jal targets,
    // and any instruction following a branch/jal/jalr/ecall.
    let mut leader = vec![false; insts.len()];
    leader[entry_idx] = true;
    for (i, inst) in insts.iter().enumerate() {
        let is_term = matches!(
            inst,
            Inst::Jal { .. }
            | Inst::Jalr { .. }
            | Inst::Branch { .. }
            | Inst::Ecall
            | Inst::Trap
        );
        match inst {
            Inst::Jal { target, .. } | Inst::Branch { target, .. } => {
                leader[*target as usize] = true;
            }
            _ => {}
        }
        if is_term && i + 1 < insts.len() {
            leader[i + 1] = true;
        }
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut block_of_inst = vec![0u32; insts.len()];
    let mut start = 0usize;
    for i in 0..=insts.len() {
        let brk = i == insts.len() || (i > 0 && leader[i]);
        if brk {
            if i > start {
                let cycles: u64 = insts[start..i]
                    .iter()
                    .map(|inst| decode::inst_cycles(inst, c.profile))
                    .sum();
                let bidx = blocks.len() as u32;
                for b in block_of_inst.iter_mut().take(i).skip(start) {
                    *b = bidx;
                }
                blocks.push(Block { start: start as u32, end: i as u32, cycles });
            }
            start = i;
        }
    }
    // Per-instruction suffix cost within its block, used when a dynamic edge
    // enters a block at an instruction other than its leader.
    let mut suffix_cycles = vec![0u64; insts.len()];
    for blk in &blocks {
        let mut acc = 0u64;
        for i in (blk.start as usize..blk.end as usize).rev() {
            acc += decode::inst_cycles(&insts[i], c.profile);
            suffix_cycles[i] = acc;
        }
    }
    Ok(CalcModule {
        profile: c.profile,
        layout,
        insts,
        addrs,
        blocks,
        block_of_inst,
        suffix_cycles,
        entry: entry_idx as u32,
        hash_builtins: cfg.hash_builtins,
        rodata_readonly: cfg.rodata_readonly,
        stack_guard: cfg.stack_guard,
    })
}
