use super::*;


/* ------------------------------------------------------------------ */
/* Public result types                                                 */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy)]
pub struct Rv64Stats {
    pub insts: usize,
    pub code_bytes: usize,
    /// Sum of consensus `inst_cycles` over the re-decoded RX stream.
    pub cycles: u64,
    /// Local slots in the frame.
    pub locals: usize,
}

#[derive(Debug)]
pub struct Rv64Lowered {
    /// RX instruction stream (entry at offset 0; 4-byte words in `Im`, a
    /// 2-byte-aligned mix of compressed and full words in `Imc*`).
    pub code: Vec<u8>,
    /// Read-only literal pool (empty when the body has no bytes literals).
    pub pool: Vec<u8>,
    /// RW arena size declared by `pack_segments` (ARENA_SIZE or BIG).
    pub arena_size: u64,
    pub stats: Rv64Stats,
}

impl Rv64Lowered {
    /// Pack into an admitted calc container: RX code + R pool + RW arena bss.
    /// The container profile follows the lowering mode (0 for `Im`, 2 for `Imc*`).
    pub fn pack_segments(
        &self,
        cap: &SpaceCap,
        mem_kib: u32,
        input_max: u16,
        output_max: u16,
    ) -> VmrtRes<container::CalcContainer> {
        self.pack_segments_profile(cap, mem_kib, input_max, output_max, container::CALC_PROFILE_RV64IM)
    }

    pub fn pack_segments_profile(
        &self,
        cap: &SpaceCap,
        mem_kib: u32,
        input_max: u16,
        output_max: u16,
        profile: u8,
    ) -> VmrtRes<container::CalcContainer> {
        let mut segs = vec![CalcSegment::rx(CODE_BASE as u32, self.code.clone())];
        let pool_vaddr = CODE_BASE as u32 + self.code.len() as u32;
        if !self.pool.is_empty() {
            segs.push(CalcSegment::ro(pool_vaddr, self.pool.clone()));
        }
        segs.push(CalcSegment::rw(ARENA_BASE as u32, Vec::new(), self.arena_size as u32));
        crate::calc::pack_container(
            cap,
            profile,
            mem_kib,
            CODE_BASE as u32,
            input_max,
            output_max,
            segs,
        )
    }
}

/// HVM-channel stats for the same IR: bytecode size and the per-opcode base
/// compute gas (static sum; the HVM side's dynamic byte-metered extras are
/// NOT included, so the HVM compute number is a lower bound — the comparison
/// is conservative in the HVM channel's favor).
pub fn hvm_bytecode_stats(codes: &[u8]) -> Ret<(usize, i64)> {
    let gst = GasTable::new(0);
    let mut pc = 0usize;
    let mut gas = 0i64;
    while pc < codes.len() {
        let op = Bytecode::try_from_u8(codes[pc])?;
        pc += 1;
        pc += match op {
            Bytecode::PBUF => 1 + codes[pc] as usize,
            Bytecode::PBUFL => 2 + u16::from_be_bytes([codes[pc], codes[pc + 1]]) as usize,
            _ => op.metadata().param as usize,
        };
        gas += gst.gas(op as u8);
    }
    Ok((codes.len(), gas))
}

/// Decode `code` with the consensus decoder for `profile` and sum cycle weights.
pub(crate) fn rv_code_stats(code: &[u8], profile: u8, locals: usize) -> Ret<Rv64Stats> {
    let mut off = 0usize;
    let mut insts = 0usize;
    let mut cycles = 0u64;
    while off < code.len() {
        let (inst, len) = decode_at(code, off, profile)?;
        cycles += inst_cycles(&inst);
        insts += 1;
        off += len as usize;
    }
    if off != code.len() {
        return errf!("rv64 lower: code stream does not end on an instruction boundary");
    }
    Ok(Rv64Stats { insts, code_bytes: code.len(), cycles, locals })
}

/* ------------------------------------------------------------------ */
/* A-19 batch 2: dominated constant-mul overflow-check merging          */
/*                                                                     */
/* `x * k` overflow guards on the SAME base register form a domination  */
/* lattice: for k1 < k2 (unsigned, both >= 2),                         */
/*   k1*x > 2^64-1  ==>  x >= ceil(2^64/k1) >= ceil(2^64/k2)           */
/*                     ==>  k2*x >= 2^64                               */
/* so the trap-input set of the k1 check is a subset of the k2 check's */
/* (mechanical arithmetic fact). When the k2 check executes later in   */
/* the same straight-line region, deleting the k1 check only moves the */
/* trap site forward — same iteration, same exit code 90 — which is    */
/* observably identical PROVIDED nothing consensus-observable happens  */
/* between the sites. The three release proofs (main-agent ruling      */
/* 2026-11-14) are built in here, not prose:                           */
/*   (a) constant domination table — k_dominated < k_retained on the   */
/*       same base register (the integer argument above);              */
/*   (b) same-base def-use — no instruction between the sites writes   */
/*       the base register (the copies feeding both checks read the    */
/*       same value);                                                   */
/*   (c) interval scan — every instruction between the sites is pure   */
/*       compute, a frame sd (non-observable by the red-line           */
/*       definition), or an arith-thunk guard branch (exit 90). Arena  */
/*       or out stores, ecall, and any other control flow veto the     */
/*       merge, as do reads of the deleted check's scratch register    */
/*       (its stale value must never be consumed before the retained   */
/*       check redefines the scratch discipline).                      */
/* Deleting bytes mid-buffer invalidates backward branches that hop    */
/* over the cut (their displacement was baked at emission); those are  */
/* re-patched with the shrunk displacement, which always still fits    */
/* (removing bytes between target and site can only narrow it).        */
/* The whole pass is a deterministic function of the emitted stream — */
/* no iteration-order dependence, re-derived per relaxation pass.      */
/* ------------------------------------------------------------------ */

/// Classified instruction kind between a dominated and its dominating
/// check (the interval-scan artifact pinned by the `#[cfg(test)]`
/// mechanical-proof tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MulMergeIvKind {
    /// pure compute or a load (loads have no guest-visible side effect)
    Pure,
    /// `sd` off the locals frame base (frame sd/ld are non-observable)
    FrameSd,
    /// a branch to the arith fault thunk — a retained exit-90 guard
    GuardArith,
}

/// One applied dominated-check deletion. Offsets are pre-deletion
/// positions in the pass's emission stream (diagnostics only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MulMergeRecord {
    /// base register both checks' value copies read (same value by (b))
    pub base_reg: u8,
    /// constant of the DELETED (dominated) check
    pub k_dominated: u64,
    /// constant of the RETAINED (dominating) check
    pub k_retained: u64,
    /// pre-deletion offset of the dominated check's first instruction
    pub dom_at: usize,
    /// pre-deletion offset of the retained check's first instruction
    pub ret_at: usize,
    /// classified instructions strictly between the two checks (proof (c))
    pub interval: Vec<MulMergeIvKind>,
}

/// Register an instruction writes, if any (the def-use scanner).
pub(crate) fn inst_rd(i: &Inst) -> Option<u8> {
    use crate::calc::decode::Inst::*;
    Some(match i {
        Lui { rd, .. }
        | Auipc { rd, .. }
        | Jal { rd, .. }
        | Jalr { rd, .. }
        | Load { rd, .. }
        | ImmAlu { rd, .. }
        | ShiftImm { rd, .. }
        | RegAlu { rd, .. }
        | UnAlu { rd, .. }
        | BitImm { rd, .. }
        | Slliuw { rd, .. } => *rd,
        Store { .. } | Branch { .. } | Ecall | Fence | Trap => return None,
    })
}

/// Registers an instruction reads (at most two; the veto scanners).
pub(crate) fn inst_rs(i: &Inst) -> (Option<u8>, Option<u8>) {
    use crate::calc::decode::Inst::*;
    match i {
        Branch { rs1, rs2, .. } | RegAlu { rs1, rs2, .. } => (Some(*rs1), Some(*rs2)),
        Store { rs1, rs2, .. } => (Some(*rs1), Some(*rs2)),
        Load { rs1, .. }
        | ImmAlu { rs1, .. }
        | ShiftImm { rs1, .. }
        | UnAlu { rs1, .. }
        | BitImm { rs1, .. }
        | Slliuw { rs1, .. }
        | Jalr { rs1, .. } => (Some(*rs1), None),
        _ => (None, None),
    }
}

/// One recognized constant-mul overflow check in the emitted stream.
pub(crate) struct MulCheckSite {
    pub(crate) base: u8,
    pub(crate) k: u64,
    /// scratch holding the check value (dead after the branch; reads of it
    /// in a merge interval veto the merge)
    pub(crate) sc: u8,
    /// byte span of the deletable pair (mulhu|srli ..= bne), pre-deletion
    pub(crate) span: (usize, usize),
    /// index of the first check instruction, then of the branch
    pub(crate) first_ix: usize,
    pub(crate) bne_ix: usize,
    pub(crate) deleted: bool,
}

/// Backward def-use recognizer for one check site: starting just before
/// `from`, the last writer of `d` must be `mv d, r` (r != 0, distinct from
/// d and t) and — when `t` is given — the last writer of `t` a plain
/// constant load >= 2. Anything else written into d/t first, or any write
/// of r between its copy and the check, vetoes (None).
///
/// The copy→check region is rescanned FORWARD before accepting: the
/// backward walk visits that region while `base` is still None (it only
/// resolves at the copy, which sits at a lower index), so an in-region
/// base rewrite would otherwise escape the veto and break the same-base
/// premise of the merge (found in main-agent acceptance review — the
/// review's counter-example is pinned by `a19_mulchk_base_write_between_
/// copy_and_check_vetoes`).
pub(crate) fn mul_check_base_const(insts: &[Inst], from: usize, d: u8, t: Option<u8>) -> Option<(u8, u64)> {
    use crate::calc::decode::{ImmAluOp, RegAluOp};
    const WINDOW: usize = 12;
    let mut base: Option<u8> = None;
    let mut k: Option<u64> = None;
    let mut copy_ix: usize = 0;
    let mut j = from;
    let mut steps = 0;
    while j > 0 && steps < WINDOW {
        j -= 1;
        steps += 1;
        let cur = &insts[j];
        let Some(rd) = inst_rd(cur) else { continue };
        if rd == d {
            let crate::calc::decode::Inst::RegAlu { op: RegAluOp::Add, rs1: 0, rs2: r, .. } = cur
            else {
                return None;
            };
            if *r == 0 || *r == d || t == Some(*r) {
                return None;
            }
            base = Some(*r);
            copy_ix = j;
        } else if t.is_some() && rd == t.unwrap() {
            match cur {
                crate::calc::decode::Inst::ImmAlu {
                    op: ImmAluOp::Addi, rs1: 0, imm, ..
                } if *imm >= 2 => k = Some(*imm as u64),
                crate::calc::decode::Inst::Lui { val, .. } if *val >= 2 => k = Some(*val as u64),
                _ => return None,
            }
        }
        if base.is_some() && (t.is_none() || k.is_some()) {
            // copy→check forward rescan: a base write in (copy_ix, from)
            // redefines the value after the copy captured it — the check
            // tests the OLD value while any later same-base site reads the
            // NEW one, so the trap-set inclusion premise breaks. Veto.
            for m in &insts[copy_ix + 1..from] {
                if inst_rd(m) == base {
                    return None;
                }
            }
            return Some((base.unwrap(), k.unwrap_or(0)));
        }
    }
    None
}

/// Decode a whole stream (positions, instructions, lengths).
pub(crate) fn decode_stream(buf: &[u8], profile: u8) -> Option<(Vec<usize>, Vec<Inst>, Vec<usize>)> {
    let mut offs = Vec::with_capacity(buf.len() / 2);
    let mut insts = Vec::with_capacity(buf.len() / 2);
    let mut lens = Vec::with_capacity(buf.len() / 2);
    let mut p = 0usize;
    while p < buf.len() {
        let (inst, len) = decode_at(buf, p, profile).ok()?;
        offs.push(p);
        insts.push(inst);
        lens.push(len as usize);
        p += len as usize;
    }
    Some((offs, insts, lens))
}

/// funct3 of a decoded branch kind (the `branch_word` encoding table).
pub(crate) fn brkind_f3(kind: crate::calc::decode::BrKind) -> u32 {
    use crate::calc::decode::BrKind::*;
    match kind {
        Eq => 0,
        Ne => 1,
        Lt => 4,
        Ge => 5,
        Ltu => 6,
        Geu => 7,
    }
}

/// Re-patch one backward branch/jump whose baked displacement hops a cut
/// `[s, e)`: the site sits above the cut, its target below, so draining
/// `cut` bytes narrows the displacement by exactly `cut` — which always
/// stays inside the same encoding window (only 4-byte jal can be baked
/// backward for far targets; c.j/bentries only shrink toward zero).
pub(crate) fn repatch_hopped_backward(asm: &mut Asm, profile: u8, s: usize, e: usize) {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else { return };
    let cut = e - s;
    for i in 0..insts.len() {
        let pos = offs[i];
        if pos < e {
            continue; // below (or inside — impossible) the cut: unaffected
        }
        // fixup-backed branches are shifted by the caller; only baked
        // (backward-resolved) ones need re-patching here
        if asm.fixups.iter().any(|f| f.at == pos) {
            continue;
        }
        let (new_rel, abs) = match &insts[i] {
            Inst::Branch { target, .. } | Inst::Jal { target, .. } => {
                (*target + cut as i64, pos as i64 + *target)
            }
            _ => continue,
        };
        if abs >= s as i64 {
            continue; // forward (+8 trampoline skip) or not hopping the cut
        }
        match &insts[i] {
            Inst::Branch { kind, rs1, rs2, .. } => {
                if lens[i] == 2 {
                    let cf3 = if matches!(kind, crate::calc::decode::BrKind::Eq) { 6 } else { 7 };
                    let patched = Asm::c_bz(cf3, *rs1, new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::branch_word(brkind_f3(*kind), *rs1, *rs2, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            Inst::Jal { rd, .. } => {
                if lens[i] == 2 {
                    let patched = Asm::c_j(new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::jal_word(*rd, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            _ => unreachable!("matched above"),
        }
    }
}

/// A-19 batch 2 driver: scan the emitted body stream, prove and apply the
/// dominated constant-mul check deletions. Returns the proof records
/// (empty when nothing merged). Deterministic: decisions depend only on
/// the instruction stream, never on iteration order.
pub(crate) fn merge_dominated_mul_checks(
    asm: &mut Asm,
    profile: u8,
    fault_arith: usize,
) -> Vec<MulMergeRecord> {
    use crate::calc::decode::{BrKind, Inst, RegAluOp, ShiftKind, StoreOp};
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else { return Vec::new() };
    // Guard branches to the arith thunk are forward refs at this point
    // (thunks are placed after the body), so the direct form carries a
    // fixup. Far guards (> ±4 KiB, big kernels) use the trampoline form:
    // an inverted branch baked to skip the next `jal`, which itself holds
    // the fault_arith fixup. Both forms are recognized here.
    let has_arith_fixup = |pos: usize| asm.fixups.iter().any(|f| f.at == pos && f.label == fault_arith);
    let trampoline_jal_end = |i: usize| -> Option<usize> {
        match &insts[i] {
            Inst::Branch { target, .. }
                if *target == 8
                    && i + 1 < insts.len()
                    && matches!(insts[i + 1], Inst::Jal { .. })
                    && has_arith_fixup(offs[i + 1]) =>
            {
                Some(offs[i + 1] + lens[i + 1])
            }
            _ => None,
        }
    };
    // ---- recognize check sites (proof input collection) ----
    let mut sites: Vec<MulCheckSite> = Vec::new();
    for i in 1..insts.len() {
        // the check guard is `bne sc, x0, →arith90`: direct (Ne + fixup)
        // or trampoline (inverted Eq skipping the jal)
        let Inst::Branch { rs1: sc, rs2: 0, .. } = insts[i] else { continue };
        let span_end = match &insts[i] {
            Inst::Branch { kind: BrKind::Ne, .. } if has_arith_fixup(offs[i]) => {
                Some(offs[i] + lens[i])
            }
            Inst::Branch { kind: BrKind::Eq, .. } => trampoline_jal_end(i),
            _ => None,
        };
        let Some(span_end) = span_end else { continue };
        let (first_ix, d, k, t_reg) = match insts[i - 1] {
            Inst::RegAlu { op: RegAluOp::Mulhu, rd, rs1, rs2 } if rd == sc => {
                (i - 1, rs1, None::<u64>, Some(rs2))
            }
            Inst::ShiftImm { kind: ShiftKind::Srl, rd, rs1, shamt, .. }
                if rd == sc && shamt >= 1 && shamt <= 63 =>
            {
                (i - 1, rs1, Some(1u64 << (64 - shamt as u32)), None)
            }
            _ => continue,
        };
        // value register: last writer must be a plain copy from a base
        // register; const operand (register form) must be a plain li >= 2
        let (base, k_reg) = match k {
            Some(kp) => match mul_check_base_const(&insts, first_ix, d, None) {
                Some((b, _)) => (b, kp),
                None => continue,
            },
            None => match mul_check_base_const(&insts, first_ix, d, t_reg) {
                Some((b, kk)) => (b, kk),
                None => continue,
            },
        };
        sites.push(MulCheckSite {
            base,
            k: k_reg,
            sc,
            span: (offs[first_ix], span_end),
            first_ix,
            bne_ix: i,
            deleted: false,
        });
    }
    // ---- prove and mark deletions (proofs (a)(b)(c)) ----
    let mut cuts: Vec<(usize, usize)> = Vec::new(); // spans, descending later
    let mut records = Vec::new();
    for ci in 0..sites.len() {
        for ei in 0..ci {
            if sites[ei].deleted {
                continue;
            }
            // (a) constant domination: k_e < k_c on the same base register
            if sites[ei].base != sites[ci].base || sites[ei].k >= sites[ci].k {
                continue;
            }
            let (e_bne_ix, e_base, e_sc, e_span) = {
                let e = &sites[ei];
                (e.bne_ix, e.base, e.sc, e.span)
            };
            let c_first_ix = sites[ci].first_ix;
            // (b)+(c): def-use + interval whitelist over the instructions
            // strictly between the two checks (a trampoline-form dominated
            // site's own jal is inside its span — start past the span end)
            let mut interval = Vec::new();
            let mut ok = true;
            let mut j = e_bne_ix + 1;
            while offs[j] < e_span.1 {
                j += 1;
            }
            while j < c_first_ix {
                let cur = &insts[j];
                if inst_rd(cur) == Some(e_base) {
                    ok = false; // (b) base redefined between the checks
                    break;
                }
                let (r1, r2) = inst_rs(cur);
                if r1 == Some(e_sc) || r2 == Some(e_sc) {
                    ok = false; // stale scratch consumed before redefinition
                    break;
                }
                let cls = match cur {
                    Inst::RegAlu { .. }
                    | Inst::ImmAlu { .. }
                    | Inst::ShiftImm { .. }
                    | Inst::Lui { .. }
                    | Inst::Auipc { .. }
                    | Inst::UnAlu { .. }
                    | Inst::BitImm { .. }
                    | Inst::Slliuw { .. }
                    | Inst::Load { .. } => MulMergeIvKind::Pure,
                    Inst::Store { op: StoreOp::Sd, rs1, .. } if *rs1 == S0 => {
                        MulMergeIvKind::FrameSd
                    }
                    Inst::Branch { .. } if has_arith_fixup(offs[j]) => {
                        MulMergeIvKind::GuardArith
                    }
                    Inst::Branch { .. } if trampoline_jal_end(j).is_some() => {
                        // arith guard in trampoline form: its jal is part of
                        // the guard — skip past it
                        interval.push(MulMergeIvKind::GuardArith);
                        j += 2;
                        continue;
                    }
                    _ => {
                        ok = false; // observable store / ecall / foreign flow
                        break;
                    }
                };
                interval.push(cls);
                j += 1;
            }
            if !ok {
                continue;
            }
            let (e_span, e_k, e_base_reg) = {
                let e = &sites[ei];
                (e.span, e.k, e.base)
            };
            sites[ei].deleted = true;
            cuts.push(e_span);
            records.push(MulMergeRecord {
                base_reg: e_base_reg,
                k_dominated: e_k,
                k_retained: sites[ci].k,
                dom_at: e_span.0,
                ret_at: sites[ci].span.0,
                interval,
            });
        }
    }
    // ---- apply cuts, highest position first ----
    cuts.sort_by(|a, b| b.0.cmp(&a.0));
    for (s, e) in cuts {
        let cut = e - s;
        // all previously applied cuts sit above this one, so (s, e) is
        // still addressable in the current buffer
        // paranoia: no label may live strictly inside the cut, and the
        // only fixups inside must be the deleted branch's own
        if asm.labels.iter().any(|l| l.map_or(false, |p| p > s && p < e)) {
            continue;
        }
        if !asm.fixups.iter().all(|f| {
            !(f.at > s && f.at < e)
                || matches!(f.kind, FixKind::Branch | FixKind::CBranchZ | FixKind::Jal)
        }) {
            continue;
        }
        if !asm.fixups.iter().any(|f| f.at >= s && f.at < e) {
            continue; // the guard branch fixup must be there to remove
        }
        // backward branches hopping the cut have baked displacements
        repatch_hopped_backward(asm, profile, s, e);
        asm.fixups.retain(|f| f.at < s || f.at >= e);
        asm.buf.drain(s..e);
        for l in asm.labels.iter_mut() {
            if let Some(p) = l {
                if *p >= e {
                    *p -= cut;
                }
            }
        }
        for f in asm.fixups.iter_mut() {
            if f.at >= e {
                f.at -= cut;
            }
        }
    }
    records
}

/* ---- P3-GUARDTAIL GT-a: guard thunk copies for 2-byte c.bz reach ---- */

/// One applied guard-copy cluster (GT-a pin record).
#[derive(Debug, Clone, PartialEq)]
pub struct GuardCopyRecord {
    /// exit code the copy materializes (90 = fault_arith, 71 = fault_cast)
    pub code: u64,
    /// final offset of the copy thunk (`li a0, code` first byte), in the
    /// finished body's coordinates
    pub copy_at: usize,
    /// copy tail form: `c.j` straight to the shared thunk (true, 6-byte
    /// copy) or a 4-byte `jal trap` (false, 8-byte copy)
    pub form_cj: bool,
    /// pre-transform offsets of the converted guard branches (original
    /// stream coordinates, ascending)
    pub sites: Vec<usize>,
}

/// Re-patch baked (backward-resolved) branches whose displacement hops an
/// inserted region `[s, e)`: the site sits above the insert, its target
/// below, so the displacement grows in magnitude by exactly `e - s` — the
/// mirror of [`repatch_hopped_backward`]. Grown displacements can leave an
/// encoding window; callers pre-check hopper windows (see
/// `guard_hoppers_ok`) and `validate_branch_windows` post-verifies the
/// whole stream, so any residual violation is a loud internal error, never
/// a silently mis-encoded branch.
pub(crate) fn repatch_widened_backward(asm: &mut Asm, profile: u8, s: usize, e: usize) {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else { return };
    let ins = e - s;
    for i in 0..insts.len() {
        let pos = offs[i];
        // fixup-backed branches are shifted by the caller; only baked
        // (already-resolved) ones need re-patching here
        if asm.fixups.iter().any(|f| f.at == pos) {
            continue;
        }
        let (old_rel, abs_pre) = match &insts[i] {
            Inst::Branch { target, .. } | Inst::Jal { target, .. } => {
                // reconstruct the PRE-insert absolute target in the right
                // frame: the splice already happened, so positions at/above
                // the insert carry the +ins shift while positions below do
                // not. Treating below-insert sites as shifted under-counted
                // abs_pre by `ins` — a trampoline hop landing exactly on the
                // insert start was then classified "unchanged" and left
                // hopping INTO the copy (`li a0,90` on a live path: the
                // generator campaign's spurious Err(90) mode-splits).
                let abs = if pos >= e {
                    pos as i64 - ins as i64 + *target
                } else {
                    pos as i64 + *target
                };
                (*target, abs)
            }
            _ => continue,
        };
        // two cases with a GROWING displacement (checked against encoding
        // windows by `guard_hoppers_ok`, post-verified by
        // `validate_branch_windows`):
        //   site above the insert, target below:  rel -= ins
        //   site below the insert, target at/above it (a spanner — e.g. a
        //   loop back-edge whose header the copy precedes): rel += ins
        // everything else (both sides on one side of the insert) is
        // unchanged.
        let new_rel = if pos >= e {
            if abs_pre >= s as i64 {
                continue; // both above: unchanged
            }
            old_rel - ins as i64
        } else {
            if abs_pre < s as i64 {
                continue; // both below: unchanged
            }
            old_rel + ins as i64
        };
        match &insts[i] {
            Inst::Branch { kind, rs1, rs2, .. } => {
                if lens[i] == 2 {
                    let cf3 = if matches!(kind, crate::calc::decode::BrKind::Eq) { 6 } else { 7 };
                    let patched = Asm::c_bz(cf3, *rs1, new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::branch_word(brkind_f3(*kind), *rs1, *rs2, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            Inst::Jal { rd, .. } => {
                if lens[i] == 2 {
                    let patched = Asm::c_j(new_rel as i32);
                    asm.buf[pos..pos + 2].copy_from_slice(&patched.to_le_bytes());
                } else {
                    let patched = Asm::jal_word(*rd, new_rel as u32);
                    asm.buf[pos..pos + 4].copy_from_slice(&patched.to_le_bytes());
                }
            }
            _ => unreachable!("matched above"),
        }
    }
}

/// Encoding-window limit of a baked branch/jump by decoded shape.
pub(crate) fn baked_window(len: usize, is_branch: bool) -> i64 {
    if is_branch {
        if len == 2 { 254 } else { 4094 }
    } else if len == 2 {
        2046
    } else {
        0xF_FFFE
    }
}

/// Post-apply soundness net: every baked (fixup-free) branch/jump in the
/// stream must sit inside its encoding window. Fixup-backed slots are
/// window-checked by `finish`, so this covers exactly what the byte-level
/// ops above could have disturbed.
pub(crate) fn validate_branch_windows(asm: &Asm, profile: u8) -> Ret<()> {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else {
        return errf!("rv64 lower: internal code/pool layout error");
    };
    for i in 0..insts.len() {
        if asm.fixups.iter().any(|f| f.at == offs[i]) {
            continue;
        }
        let (rel, is_branch) = match &insts[i] {
            Inst::Branch { target, .. } => (*target, true),
            Inst::Jal { target, .. } => (*target, false),
            _ => continue,
        };
        if rel.abs() > baked_window(lens[i], is_branch) || rel & 1 != 0 {
            return errf!(
                "rv64 lower: internal code/pool layout error (baked branch {:#x} rel {} out of window)",
                offs[i],
                rel
            );
        }
    }
    Ok(())
}

/// A planned GT-a transformation in original-stream coordinates. `bound` is
/// the smallest original offset whose byte moves when the op applies (cuts:
/// `site+2` — the top half of the 4-byte branch drains; inserts: `p` —
/// everything at/after the copy shifts). Ops apply in descending `bound`
/// order, so lower ops' coordinates stay valid. `cluster` indexes
/// [`GuardCopyRecord`] / the copy label table.
pub(crate) struct GuardTailOp {
    pub(crate) bound: usize,
    pub(crate) delta: i64,
    pub(crate) cluster: usize,
    /// cut op: site offset (bytes [site, site+2) become the c.bz slot)
    pub(crate) site: Option<usize>,
    /// insert op: copy position, exit code, c.j form
    pub(crate) ins: Option<(usize, u64, bool)>,
}

/// One planned cluster before form commitment / application.
pub(crate) struct GuardTailPlan {
    pub(crate) code: u64,
    /// (offset, fixup seq, branch f3, compare register) per site, ascending
    pub(crate) sites: Vec<(usize, usize, u32, u8)>,
}

/// GT-a driver: give clustered zero-form guard branches (`bne/beq rs1, x0
/// → shared fault thunk`, direct 4-byte fixup form, rs1 in the RVC window)
/// a 2-byte `c.bz` path by inserting a copy thunk — `li a0, <code>` plus a
/// jump — within ±254 B of every site of its cluster.
///
/// Semantics: the copy materializes the SAME exit code into a0 and jumps to
/// the SAME shared thunk (which rewrites a0 with the same constant before
/// trapping), so every trap lands on the identical observable state;
/// taken/not-taken decisions still compare the same register against zero.
/// Pure encoding shape — trap timing and observability are per-value
/// unchanged.
///
/// Determinism: site recognition, clustering, copy placement and form are
/// pure functions of the post-merge instruction stream + the wide_c_ids
/// set (sites widened in a previous relaxation pass stay 4-byte; that set
/// only grows, so the whole plan is monotone-convergent). Planning runs in
/// three phases — cluster geometry (all labels combined, worst-case 8-byte
/// copies), then copy form per cluster with the full op set known, then
/// application — so overlapping clusters of different thunks cannot make a
/// c.j displacement drift out of its window. Re-derived identically on
/// every relaxation pass.
pub(crate) fn insert_guard_thunk_copies(
    asm: &mut Asm,
    profile: u8,
    fault_arith: usize,
    fault_cast: usize,
    trap_label: usize,
    body_end: usize,
    thunk_arith_pos: usize,
    thunk_cast_pos: usize,
) -> Ret<Vec<GuardCopyRecord>> {
    use crate::calc::decode::{BrKind, Inst};
    // debug-only kill switch (same class as the other diag_env_enabled
    // knobs: compiled out of release builds, so production bytes and
    // determinism are untouched) — the rv96 scratch differential harness
    // lowers GT-on and GT-off streams in one process and compares them
    if diag_env_enabled("RV64_GT_DISABLE") {
        return Ok(Vec::new());
    }
    // RVC-only transform: a 2-byte c.bz does not exist in an uncompressed
    // profile, and its site scan would otherwise "convert" plain 4-byte
    // branches into undecodable C encodings (caught by the C3 generator's
    // Im-mode campaign: `internal code/pool layout error`)
    if profile & container::CALC_PROFILE_COMPRESSED == 0 {
        return Ok(Vec::new());
    }
    // The shared routines and the fault thunks are already in the buffer
    // (the caller emits them before this pass so the copy→thunk distance is
    // exact); the site scan and every insertion stay inside the BODY.
    let Some((offs, insts, lens)) = decode_stream(&asm.buf[..body_end], profile) else {
        return Ok(Vec::new());
    };
    // ---- phase 0: recognize sites (4B direct-fixup zero-form guards) ----
    // trampoline-form guards carry their fixup on the jal, not on the
    // inverted branch, and fall out of the recognition here
    let mut sites: Vec<(usize, usize, usize, u32, u8)> = Vec::new(); // off,label,seq,f3,rs1
    for i in 0..insts.len() {
        let (kind, rs1) = match &insts[i] {
            Inst::Branch { kind: k @ (BrKind::Eq | BrKind::Ne), rs1, rs2: 0, .. } => (k, *rs1),
            _ => continue,
        };
        if lens[i] != 4 || !rvc_win(rs1) {
            continue;
        }
        let Some(f) = asm
            .fixups
            .iter()
            .find(|f| f.at == offs[i] && f.kind == FixKind::Branch)
            .map(|f| (f.label, f.seq))
        else {
            continue;
        };
        let (label, seq) = f;
        if label != fault_arith && label != fault_cast {
            continue;
        }
        // A zero-form window-register guard reaches here only in its
        // WIDENED form: at emission such a site is an optimistic 2-byte
        // c.bz slot (CBranchZ fixup), and `finish` widens it to this 4-byte
        // Branch fixup exactly when the shared thunk sits outside the
        // ±256 B window (seq ∈ wide_c_ids by construction). Those are the
        // proven-unreachable sites a copy thunk serves; the not-yet-widened
        // slots keep their original target and need no copy.
        sites.push((offs[i], label, seq, brkind_f3(*kind), rs1));
    }
    if sites.is_empty() {
        return Ok(Vec::new());
    }
    // ---- phase 1: cluster per thunk label (strict 510 B windows), then
    // resolve feasibility with worst-case 8-byte copies; drop the farthest
    // site of an infeasible or unprofitable window (deterministic) ----
    let mut plans: Vec<GuardTailPlan> = Vec::new();
    let mut accepted_ops: Vec<(usize, i64)> = Vec::new(); // (bound, delta)
    // O(1) shift lookup: prefix sums over a difference array indexed by
    // stream position, rebuilt whenever accepted_ops grows. A linear
    // shift_at here made phase 1 O(sites^3) on guard-heavy bodies
    // (~2200 sites -> 26 min CPU spin, caught on a19 bulk relaxation).
    let mut shift_prefix = vec![0i64; asm.buf.len() + 1];
    let virt_shift = |extra: &[(usize, i64)], x: usize| -> i64 {
        extra.iter().filter(|o| o.0 <= x).map(|o| o.1).sum()
    };
    // safe boundaries (fall-through-impossible positions) as a set — the
    // pick loop probes it per candidate and must not rescan the stream.
    // A LINKING jal (rd != x0, e.g. the u128 divmod call) is NOT one: the
    // callee returns to the position right after it, so a copy thunk
    // parked there would execute on the return path (the fault epilogue's
    // a7=0 ecall then traps the whole kernel).
    let safe_set: std::collections::HashSet<usize> = insts
        .iter()
        .zip(offs.iter().zip(lens.iter()))
        .filter(|(inst, _)| match inst {
            Inst::Jal { rd: 0, .. } | Inst::Jalr { .. } | Inst::Trap => true,
            _ => false,
        })
        .map(|(_, (o, l))| o + l)
        .collect();
    for &label in &[fault_arith, fault_cast] {
        let mut grp: Vec<&(usize, usize, usize, u32, u8)> =
            sites.iter().filter(|s| s.1 == label).collect();
        grp.sort_by_key(|s| s.0);
        let code = if label == fault_arith { EXIT_ARITH } else { EXIT_CAST };
        let mut win_start = 0usize;
        while win_start < grp.len() {
            let mut end = win_start + 1;
            while end < grp.len() && grp[end].0 - grp[win_start].0 <= 510 {
                end += 1;
            }
            let mut planned = false;
            loop {
                let n = end - win_start;
                if n < 2 {
                    break; // singleton: net ≤ 0 by construction
                }
                // worst-case net (8-byte copy) must be strictly positive;
                // the 6-byte c.j form is decided in phase 2 with full ops
                if 2 * n as i64 - 8 <= 0 {
                    break;
                }
                let span = grp[end - 1].0 - grp[win_start].0;
                // post-cut span (each converted site sheds 2 bytes): a copy
                // inside the cluster reaches both ends only if this holds
                if span - 2 * (n - 1) + 8 > 508 {
                    end -= 1;
                    continue;
                }
                // rough copy window in original coordinates, then exact
                // displacement check over decoded instruction starts
                // (largest start first — deterministic)
                // early-window sites sit before offset 246: keep the
                // rough window arithmetic signed, clamped at 0
                let lo = (grp[end - 1].0 as i64 + 8 - 254).max(0) as usize;
                let hi = grp[win_start].0 + 254;
                // fall-through safety: the copy sits between original
                // instructions, so it is reachable by straight-line flow
                // unless the preceding instruction never falls through.
                // Only boundaries directly after an unconditional control
                // transfer (jal/c.j/c.jr — also after `ebreak`, which traps)
                // qualify; a copy elsewhere would execute `li a0,90; jump
                // to the fault thunk` on an otherwise-trap-free path.
                let mut picked: Option<usize> = None;
                if lo <= hi {
                    let virt: Vec<(usize, i64)> = grp[win_start..end]
                        .iter()
                        .map(|s| (s.0 + 2, -2i64))
                        .collect();
                    for &p in offs.iter().rev() {
                        if p > hi {
                            continue;
                        }
                        if p < lo {
                            break;
                        }
                        if !safe_set.contains(&p) {
                            continue;
                        }
                        let cf = p as i64 + shift_prefix[p] + virt_shift(&virt, p);
                        let ok = grp[win_start..end].iter().all(|s| {
                            let fi = s.0 as i64
                                + shift_prefix[s.0]
                                + virt_shift(&virt, s.0)
                                + i64::from(p <= s.0) * 8;
                            let d = cf - fi;
                            d.abs() <= 254 && d & 1 == 0
                        });
                        let hops = guard_hoppers_ok(asm, profile, p, 8);
                        if ok && hops {
                            picked = Some(p);
                            break;
                        }
                    }
                }
                let Some(p) = picked else {
                    end -= 1;
                    continue;
                };
                plans.push(GuardTailPlan {
                    code,
                    sites: grp[win_start..end]
                        .iter()
                        .map(|s| (s.0, s.2, s.3, s.4))
                        .collect(),
                });
                for s in &grp[win_start..end] {
                    accepted_ops.push((s.0 + 2, -2));
                }
                accepted_ops.push((p, 8));
                // refresh the O(1) shift table for subsequent windows
                for v in shift_prefix.iter_mut() {
                    *v = 0;
                }
                for &(pos, d) in &accepted_ops {
                    shift_prefix[pos] += d;
                }
                for i in 1..shift_prefix.len() {
                    shift_prefix[i] += shift_prefix[i - 1];
                }
                planned = true;
                break;
            }
            // on success skip past the accepted window; on failure retry
            // from the next site as anchor — a failed window's early sites
            // must not swallow later, independently viable clusters
            win_start = if planned { end } else { win_start + 1 };
        }
    }
    if plans.is_empty() {
        return Ok(Vec::new());
    }
    // ---- phase 2 + 2.5: copy form per plan (all ops known, exact
    // geometry) with per-site displacement verification ----
    // `all_ops`: cuts (−2 per site) + inserts; insert deltas start at the
    // worst-case 8 and drop to 6 when the cluster takes the c.j form.
    // Phase 1 picked positions under worst-case 8-byte assumptions; here
    // every converted site's exact final displacement to its copy is
    // re-verified (overlapping clusters of different thunks can interleave
    // their ops), and any cluster with a violating site is dropped wholesale
    // (its ops leave the geometry) until the remaining set verifies clean.
    let mut all_ops: Vec<(usize, i64, usize, bool)> = Vec::new(); // bound, delta, cluster, is_insert
    for (ci, pl) in plans.iter().enumerate() {
        let p = accepted_ops
            .iter()
            .filter(|o| o.1 > 0)
            .nth(ci)
            .map(|o| o.0)
            .expect("insert op per plan");
        for s in &pl.sites {
            all_ops.push((s.0 + 2, -2, ci, false));
        }
        all_ops.push((p, 8, ci, true));
    }
    let final_at = |ops: &[(usize, i64, usize, bool)], x: usize| -> i64 {
        x as i64 + ops.iter().filter(|o| o.0 <= x).map(|o| o.1).sum::<i64>()
    };
    // verify every converted site's displacement to its copy (exact)
    let sites_ok = |ops: &[(usize, i64, usize, bool)], live: &[usize]| -> bool {
        live.iter().all(|&ci| {
            let Some(ins) = ops.iter().find(|o| o.3 && o.2 == ci) else { return true };
            let (p, delta, _, _) = (ins.0, ins.1, ins.2, ins.3);
            let cf = final_at(ops, p) - delta;
            plans[ci].sites.iter().all(|s| {
                let fi = final_at(ops, s.0);
                let d = cf - fi;
                d.abs() <= 254 && d & 1 == 0
            })
        })
    };
    let mut forms: Vec<bool> = vec![false; plans.len()]; // c.j per cluster
    let mut live: Vec<usize> = (0..plans.len()).collect();
    loop {
        // decide forms for the live clusters (c.j first: bigger net, needs
        // the finished copy→thunk displacement inside the ±2 KiB window)
        for &ci in live.iter() {
            let want = 2 * plans[ci].sites.len() as i64;
            // The copy's own jump targets the code's shared fault thunk,
            // whose final address is its pre-insertion position plus every
            // applied delta (all insertions sit inside the body).
            let thunk_pos =
                if plans[ci].code == EXIT_ARITH { thunk_arith_pos } else { thunk_cast_pos };
            let mut chose = (8i64, false);
            for &(d, cj) in &[(6i64, true), (8i64, false)] {
                if want - d <= 0 {
                    continue;
                }
                let trial: Vec<(usize, i64, usize, bool)> = all_ops
                    .iter()
                    .map(|o| match o {
                        (b, _, c, true) if *c == ci => (*b, d, *c, true),
                        _ => *o,
                    })
                    .collect();
                let ins = trial.iter().find(|o| o.3 && o.2 == ci).expect("insert");
                let cf = final_at(&trial, ins.0) - d;
                let thunk_final = thunk_pos as i64 + trial.iter().map(|o| o.1).sum::<i64>();
                // the c.j sits at cf+4 (after the 4-byte li)
                let dist = thunk_final - cf - 4;
                if dist >= -2048 && dist <= 2046 && dist & 1 == 0 {
                    chose = (d, cj);
                    break;
                }
            }
            let ins = all_ops.iter_mut().find(|o| o.3 && o.2 == ci).unwrap();
            ins.1 = chose.0;
            forms[ci] = chose.1;
        }
        if sites_ok(&all_ops, &live) {
            break;
        }
        // drop the lowest-indexed violating cluster and retry (deterministic)
        let mut dropped = None;
        'outer: for &ci in live.iter() {
            let ins = all_ops.iter().find(|o| o.3 && o.2 == ci).expect("insert");
            let (p, delta, _, _) = (ins.0, ins.1, ins.2, ins.3);
            let cf = final_at(&all_ops, p) - delta;
            for s in &plans[ci].sites {
                let d = cf - final_at(&all_ops, s.0);
                if d.abs() > 254 || d & 1 != 0 {
                    dropped = Some(ci);
                    break 'outer;
                }
            }
        }
        let Some(ci) = dropped else {
            break; // no violation attributable to a live cluster: done
        };
        live.retain(|&c| c != ci);
        all_ops.retain(|o| o.2 != ci);
    }
    let mut ops: Vec<GuardTailOp> = Vec::new();
    for &ci in live.iter() {
        let pl = &plans[ci];
        let p = all_ops.iter().find(|o| o.3 && o.2 == ci).map(|o| o.0).unwrap();
        let delta = all_ops.iter().find(|o| o.3 && o.2 == ci).map(|o| o.1).unwrap();
        let form_cj = forms[ci];
        if 2 * pl.sites.len() as i64 - delta <= 0 {
            continue; // unprofitable after form resolution (cannot happen:
                      // phase 1 gated on the 8-byte worst case)
        }
        for s in &pl.sites {
            ops.push(GuardTailOp {
                bound: s.0 + 2,
                delta: -2,
                cluster: ci,
                site: Some(s.0),
                ins: None,
            });
        }
        ops.push(GuardTailOp {
            bound: p,
            delta,
            cluster: ci,
            site: None,
            ins: Some((p, pl.code, form_cj)),
        });
    }
    let mut rec_by_cluster: Vec<Option<GuardCopyRecord>> = vec![None; plans.len()];
    for o in ops.iter().filter(|o| o.ins.is_some()) {
        rec_by_cluster[o.cluster] = Some(GuardCopyRecord {
            code: o.ins.unwrap().1,
            copy_at: 0, // filled after application (final coordinates)
            form_cj: o.ins.unwrap().2,
            sites: plans[o.cluster].sites.iter().map(|s| s.0).collect(),
        });
    }
    if ops.is_empty() {
        return Ok(Vec::new());
    }
    // ---- phase 3: apply, descending by bound ----
    // one label per plan (only live ones get placed)
    let copy_labels: Vec<usize> = (0..plans.len()).map(|_| asm.new_label()).collect();
    ops.sort_by(|a, b| b.bound.cmp(&a.bound));
    for op in &ops {
        if let Some(site) = op.site {
            // rewrite [site, site+2) into the c.bz slot, drain [site+2, site+4)
            let (_off, _label, _seq, f3, rs1) = *sites
                .iter()
                .find(|s| s.0 == site)
                .expect("site recorded");
            let off = site;
            let cf3 = if f3 == 0 { 6 } else { 7 }; // c.beqz / c.bnez
            // repatch FIRST, on the intact stream: after the placeholder
            // write, the drained half [off+2, off+4) still holds the branch
            // word's top half — not a decodable instruction — and the
            // repatch's stream walk would die there and silently skip every
            // baked branch hopping this cut (the HIP5 trap bug)
            repatch_hopped_backward(asm, profile, off + 2, off + 4);
            let patched = Asm::c_bz(cf3, rs1, 0);
            asm.buf[off..off + 2].copy_from_slice(&patched.to_le_bytes());
            asm.buf.drain(off + 2..off + 4);
            for l in asm.labels.iter_mut() {
                if let Some(pp) = l {
                    if *pp >= off + 4 {
                        *pp -= 2;
                    }
                }
            }
            for f in asm.fixups.iter_mut() {
                if f.at >= off + 4 {
                    f.at -= 2;
                }
            }
            // retarget the site's own fixup onto its cluster's copy label
            let f = asm.fixups.iter_mut().find(|f| f.at == off).expect("site fixup");
            f.kind = FixKind::CBranchZ;
            f.label = copy_labels[op.cluster];
        } else if let Some((p, code, form_cj)) = op.ins {
            let delta = op.delta as usize;
            let fill: Vec<u8> = {
                let mut v = vec![0u8; delta];
                // li a0, code (addi a0, x0, imm — 90/71 exceed the c.li range,
                // so this is always the 4-byte I-form, matching the shared
                // thunks' own `asm.li(A0, code)`)
                let w = (((code as u32) & 0xFFF) << 20) | ((A0 as u32) << 7) | 0b0010011;
                v[0..4].copy_from_slice(&w.to_le_bytes());
                if !form_cj {
                    // jal x0, trap — fixup-backed, resolved by finish
                    v[4..8].copy_from_slice(&Asm::jal_word(ZERO, 0).to_le_bytes());
                } else {
                    // c.j placeholder — displacement baked after all ops
                    // (the copy→thunk distance is shifted by exactly the
                    // applied deltas, all of which sit inside the body)
                    v[4..6].copy_from_slice(&Asm::c_j(0).to_le_bytes());
                }
                v
            };
            asm.buf.splice(p..p, fill);
            for l in asm.labels.iter_mut() {
                if let Some(pp) = l {
                    if *pp >= p {
                        *pp += delta;
                    }
                }
            }
            for f in asm.fixups.iter_mut() {
                if f.at >= p {
                    f.at += delta;
                }
            }
            // push the copy's own jal fixup only AFTER the shift loops — it
            // is written at the copy's in-buffer tail (p+4) and must not
            // shift itself (a self-shifted fixup would patch the wrong word
            // in `finish` and leave the real jal unresolvable)
            if !form_cj {
                asm.fixups.push(Fixup {
                    at: p + 4,
                    label: trap_label,
                    kind: FixKind::Jal,
                    seq: usize::MAX,
                    aux: 0,
                });
            }
            // the copy label sits at the copy's first byte (p in the
            // current buffer); set directly — `place` would pin it to the
            // buffer end, which is the literal pool region here
            assert!(asm.labels[copy_labels[op.cluster]].is_none(), "copy label placed twice");
            asm.labels[copy_labels[op.cluster]] = Some(p);
            repatch_widened_backward(asm, profile, p, p + delta);
        }
    }
    // ---- post-apply: fill copy_at records, bake c.j displacements,
    // validate every baked branch window ----
    let total_shift: i64 = ops.iter().map(|o| o.delta).sum();
    for ci in live.iter() {
        let Some(rec) = rec_by_cluster[*ci].as_mut() else { continue };
        let (p, delta, _, _) = all_ops
            .iter()
            .find(|o| o.3 && o.2 == *ci)
            .copied()
            .expect("insert op");
        let shift: i64 = all_ops.iter().filter(|o| o.0 <= p).map(|o| o.1).sum();
        let cf = p as i64 + shift - delta;
        rec.copy_at = cf as usize;
        if rec.form_cj {
            // bake the c.j displacement: the code's shared fault thunk sits
            // at its pre-insertion position plus the total applied shift
            // (every insertion is inside the body). The c.j sits at cf+4
            // (after the 4-byte li), and the encoding adds its own address.
            let thunk_pos = if rec.code == EXIT_ARITH { thunk_arith_pos } else { thunk_cast_pos };
            let thunk_final = thunk_pos as i64 + total_shift;
            let dist = thunk_final - cf - 4;
            let at = cf as usize + 4;
            let patched = Asm::c_j(dist as i32);
            asm.buf[at..at + 2].copy_from_slice(&patched.to_le_bytes());
        }
    }
    validate_branch_windows(asm, profile)?;
    Ok(rec_by_cluster.into_iter().flatten().collect())
}

/// Pre-check that inserting `delta` bytes at `p` keeps every baked branch
/// hopping the insert inside its encoding window (the widened displacements
/// are re-encoded by [`repatch_widened_backward`]; later ops can only
/// shrink these again, and `validate_branch_windows` post-verifies).
pub(crate) fn guard_hoppers_ok(asm: &Asm, profile: u8, p: usize, delta: i64) -> bool {
    use crate::calc::decode::Inst;
    let Some((offs, insts, lens)) = decode_stream(&asm.buf, profile) else {
        return false;
    };
    for i in 0..insts.len() {
        let (rel, is_branch) = match &insts[i] {
            Inst::Branch { target, .. } => (*target, true),
            Inst::Jal { target, .. } => (*target, false),
            _ => continue,
        };
        if asm.fixups.iter().any(|f| f.at == offs[i]) {
            continue; // resolved by finish at final positions
        }
        let abs = offs[i] as i64 + rel;
        let grown = if rel < 0 {
            // backward: hopper iff target below the insert ≤ site
            abs < p as i64 && p <= offs[i]
        } else {
            // forward (+8 trampoline skips): hopper iff site below insert ≤ target
            offs[i] < p && p <= abs as usize
        };
        if !grown {
            continue;
        }
        if rel.abs() + delta > baked_window(lens[i], is_branch) {
            return false;
        }
    }
    true
}
