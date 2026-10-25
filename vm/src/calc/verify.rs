//! Deploy-time verifier + pre-decode + basic-block cycle pricing.
//!
//! `verify_container` is run once at deployment (and re-run idempotently on
//! load); it rejects anything outside the RV64IMC whitelist, off-boundary
//! jump targets, unreachable code and malformed headers, and produces the
//! executable [`CalcModule`] used by the interpreter.

use crate::calc::container::CalcContainer;
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

/// Verified, pre-decoded calc module. `insts[i]` is at code byte
/// `addrs[i]`; branch/JAL targets are rewritten to instruction indices.
#[derive(Debug, Clone)]
pub struct CalcModule {
    pub layout: crate::calc::container::CalcLayout,
    pub insts: Vec<Inst>,
    /// Byte address of each instruction (strictly increasing).
    pub addrs: Vec<u32>,
    /// Basic blocks in address order.
    pub blocks: Vec<Block>,
    /// Block index containing each instruction.
    pub block_of_inst: Vec<u32>,
    /// Instruction index of the entry point.
    pub entry: u32,
    pub fncnf: u8,
}

impl CalcModule {
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
    let layout = c.layout();
    // Re-check layout fit even for directly constructed containers (decode
    // already enforces this; the verifier must not trust its caller).
    if layout.input_base < layout.data_base + layout.init_len {
        return Err(err("calc verify: io regions overlap code/data"));
    }
    // Pass 1: linear scan decode + boundary map.
    let code = &c.code;
    let mut insts: Vec<Inst> = Vec::new();
    let mut addrs: Vec<u32> = Vec::new();
    let mut boundaries: Vec<bool> = vec![false; code.len() + 1];
    let mut off = 0usize;
    while off < code.len() {
        boundaries[off] = true;
        let (inst, len) = decode::decode_at(code, off)?;
        addrs.push(off as u32);
        insts.push(inst);
        off += len as usize;
    }
    // Pass 2: resolve static jump targets to absolute byte addresses, then
    // to instruction indices; targets must land on instruction boundaries.
    let addr_idx: Vec<(u32, u32)> =
        addrs.iter().enumerate().map(|(i, a)| (*a, i as u32)).collect();
    let resolve = |abs: i64| -> VmrtRes<u32> {
        if abs < 0 || abs > code.len() as i64 || !boundaries[abs as usize] {
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
    // Entry must be a boundary.
    let entry_idx = match addr_idx.binary_search_by_key(&c.entry, |(a, _)| *a) {
        Ok(i) => addr_idx[i].1 as usize,
        Err(_) => return Err(err("calc verify: entry not on instruction boundary")),
    };
    // Pass 2c: resolve the compiler direct-call idiom
    // `auipc rd, hi ; jalr _, rd, lo` into a static edge. All rustc/clang
    // PC-relative calls lower to exactly this adjacent pair, so kernels
    // remain reachable; genuinely dead code is still rejected.
    let mut call_edges: Vec<Option<usize>> = vec![None; insts.len()];
    for i in 0..insts.len().saturating_sub(1) {
        if let (Inst::Auipc { rd, val }, Inst::Jalr { rs1, imm, .. }) = (insts[i], insts[i + 1]) {
            if rs1 == rd {
                let target = (addrs[i] as i64)
                    .checked_add(val)
                    .and_then(|t| t.checked_add(imm));
                let idx = match target {
                    Some(t) => resolve(t)? as usize,
                    None => return Err(err("calc verify: call target overflow")),
                };
                call_edges[i + 1] = Some(idx);
            }
        }
    }
    // Pass 3: reachability from entry (DFS over static successors). jalr
    // jumps dynamically, but its fallthrough is a call-return site and thus
    // reachable.
    let mut reach = vec![false; insts.len()];
    let mut work = vec![entry_idx];
    reach[entry_idx] = true;
    while let Some(i) = work.pop() {
        let next = i + 1;
        // Fallthrough after a plain jump (jal/jalr with rd = x0) is dead;
        // calls (rd != x0) return to the next instruction, so it stays live.
        // Dynamic jalr targets are unconstrained and checked at runtime.
        let succs: [Option<usize>; 2] = match insts[i] {
            Inst::Jal { rd: 0, target } => [Some(target as usize), None],
            Inst::Jal { target, .. } => [Some(target as usize), Some(next)],
            Inst::Branch { target, .. } => [Some(target as usize), Some(next)],
            Inst::Jalr { rd: 0, .. } => [None, None],
            Inst::Jalr { .. } => [Some(next), None],
            _ => [Some(next), None],
        };
        let push = |s: usize, reach: &mut Vec<bool>, work: &mut Vec<usize>| {
            if !reach[s] {
                reach[s] = true;
                work.push(s);
            }
        };
        for s in succs.into_iter().flatten() {
            if s < insts.len() {
                push(s, &mut reach, &mut work);
            }
        }
        if let Some(callee) = call_edges[i] {
            push(callee, &mut reach, &mut work);
        }
    }
    if reach.iter().any(|r| !r) {
        return Err(err("calc verify: unreachable instructions in code section"));
    }
    // Pass 4: split into basic blocks. Leaders: entry, branch/jal targets,
    // and any instruction following a branch/jal/jalr.
    let mut leader = vec![false; insts.len()];
    leader[entry_idx] = true;
    for (i, inst) in insts.iter().enumerate() {
        let is_term = matches!(inst, Inst::Jal { .. } | Inst::Jalr { .. } | Inst::Branch { .. });
        match inst {
            Inst::Jal { target, .. } | Inst::Branch { target, .. } => {
                leader[*target as usize] = true;
            }
            _ => {}
        }
        if let Some(callee) = call_edges[i] {
            leader[callee] = true;
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
                let cycles: u64 = insts[start..i].iter().map(decode::inst_cycles).sum();
                let bidx = blocks.len() as u32;
                for b in block_of_inst.iter_mut().take(i).skip(start) {
                    *b = bidx;
                }
                blocks.push(Block { start: start as u32, end: i as u32, cycles });
            }
            start = i;
        }
    }
    Ok(CalcModule {
        layout,
        insts,
        addrs,
        blocks,
        block_of_inst,
        entry: entry_idx as u32,
        fncnf: c.fncnf,
    })
}
