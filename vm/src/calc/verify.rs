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

use crate::calc::container::{
    CalcContainer, CALC_PROFILE_RV64IM, HALT_STUB, HALT_STUB_BYTES, SEG_X,
};
use crate::calc::decode::{self, Inst};
use crate::rt::{ItrErr, ItrErrCode, SpaceCap, VmrtRes};

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
/// address `addrs[i]`; branch/JAL targets are rewritten to instruction
/// indices. Index 0..HALT_STUB_INSTS is the injected trampoline.
#[derive(Debug, Clone)]
pub struct CalcModule {
    /// ISA profile byte (bitmanip / RVC / hash).
    pub(crate) profile: u8,
    pub(crate) layout: crate::calc::container::CalcLayout,
    pub(crate) segs: Vec<(u32, u32, u8)>,
    pub(crate) insts: Vec<Inst>,
    /// Byte address of each instruction (strictly increasing).
    pub(crate) addrs: Vec<u32>,
    /// Encoded length of each instruction (2 or 4). Required because
    /// executable segments need not be adjacent.
    pub(crate) lens: Vec<u32>,
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
}

impl CalcModule {
    /// Sequential fetch from `idx` lands on `idx+1` iff that instruction
    /// begins at `pc + encoded_len`. A hole between RX segments is
    /// unmapped: falling through must trap, not execute the next index.
    pub fn sequential_next(&self, idx: usize) -> Option<usize> {
        let next = idx + 1;
        if next >= self.insts.len() {
            return None;
        }
        let expect = self.addrs[idx].wrapping_add(self.lens[idx]);
        if self.addrs[next] == expect {
            Some(next)
        } else {
            None
        }
    }

    /// ISA profile byte of the verified container. Test-oracle view.
    #[doc(hidden)]
    pub fn profile(&self) -> u8 {
        self.profile
    }

    /// Encoded byte length of instruction `idx` (2 or 4).
    pub fn inst_len(&self, idx: usize) -> u32 {
        self.lens[idx]
    }

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

/// Verify under the default chain caps (`SpaceCap::new(0)`). Tests and
/// tools only; production deploy/load pass the live cap.
pub fn verify_container(c: &CalcContainer) -> VmrtRes<CalcModule> {
    verify_container_with_cap(c, &SpaceCap::new(0))
}

/// Verify a container under the chain's current `SpaceCap`. The memory
/// admission cap (`calc_mem_kib_max`) is the only chain input; it only
/// rises, so a container admitted once stays admissible.
pub fn verify_container_with_cap(c: &CalcContainer, cap: &SpaceCap) -> VmrtRes<CalcModule> {
    // Header checks must run here too: a directly constructed container did
    // not pass through `decode`.
    c.validate_header(cap)?;
    verify_decoded(c, c.layout())
}

fn push_stream(
    insts: &mut Vec<Inst>,
    addrs: &mut Vec<u32>,
    lens: &mut Vec<u32>,
    base: u64,
    bytes: &[u8],
    profile: u8,
) -> VmrtRes<()> {
    let mut off = 0usize;
    while off < bytes.len() {
        let (inst, len) = decode::decode_at(bytes, off, profile)?;
        let ok_len = if crate::calc::container::profile_has_compressed(profile) {
            len == 2 || len == 4
        } else {
            len == 4
        };
        if !ok_len {
            return Err(err("calc verify: instruction length not in profile"));
        }
        addrs.push((base + off as u64) as u32);
        insts.push(inst);
        lens.push(len);
        off += len as usize;
    }
    if off != bytes.len() {
        return Err(err("calc verify: truncated executable segment"));
    }
    Ok(())
}

fn verify_decoded(
    c: &CalcContainer,
    layout: crate::calc::container::CalcLayout,
) -> VmrtRes<CalcModule> {
    let mut insts: Vec<Inst> = Vec::new();
    let mut addrs: Vec<u32> = Vec::new();
    let mut lens: Vec<u32> = Vec::new();
    // Platform HALT trampoline: always uncompressed RV64I, always present.
    push_stream(
        &mut insts,
        &mut addrs,
        &mut lens,
        HALT_STUB,
        &HALT_STUB_BYTES,
        CALC_PROFILE_RV64IM,
    )?;
    for s in &c.segs {
        if s.flags & SEG_X == 0 {
            continue;
        }
        push_stream(
            &mut insts,
            &mut addrs,
            &mut lens,
            s.vaddr as u64,
            &s.data,
            c.profile,
        )?;
    }
    let addr_idx: Vec<(u32, u32)> =
        addrs.iter().enumerate().map(|(i, a)| (*a, i as u32)).collect();
    let resolve = |abs: i64| -> VmrtRes<u32> {
        if abs < 0 || abs > u32::MAX as i64 {
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
    leader[0] = true; // HALT trampoline is always a block of its own
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
    // RX segments need not be adjacent. A new segment (or the guest after
    // the HALT stub, when it is not at CODE_BASE) is a fetch hole: start a
    // block so fall-through is a trap instead of executing the next index.
    for i in 1..addrs.len() {
        if addrs[i] != addrs[i - 1] + lens[i - 1] {
            leader[i] = true;
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
                    .map(|inst| decode::inst_cycles(inst))
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
            acc += decode::inst_cycles(&insts[i]);
            suffix_cycles[i] = acc;
        }
    }
    Ok(CalcModule {
        profile: c.profile,
        layout,
        segs: c.seg_recs(),
        insts,
        addrs,
        lens,
        blocks,
        block_of_inst,
        suffix_cycles,
        entry: entry_idx as u32,
    })
}
