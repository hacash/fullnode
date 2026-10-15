use super::ir::{IRNodeArray, IRNodeDouble, IRNodeLeaf, IRNodeParam1, IRNodeParam1Single};
use super::rt::Bytecode;

/// Encapsulates PrintOption-driven tweaks applied during decompilation.
pub struct DecompilationHelper<'a> {
    opt: &'a PrintOption<'a>,
}

impl<'a> DecompilationHelper<'a> {
    pub fn new(opt: &'a PrintOption<'a>) -> Self {
        Self { opt }
    }

    pub fn block_prefix(&self, arr: &IRNodeArray) -> String {
        let _ = arr;
        // NOTE: lib prelude emission is handled explicitly at the formatter top-level (see `Formater::print`). Keeping this empty avoids injecting file-level declarations into nested/inline contexts.
        String::new()
    }

    pub fn should_trim_root_block(&self, arr: &IRNodeArray) -> bool {
        use Bytecode::*;
        // `tab == 0` alone is not "root": root statements also print at tab 0
        // (named channel), so a while/if body block would satisfy the same
        // predicate and lose its braces. The one-shot ticket admits exactly
        // one eligible block per root print session — the file-level root.
        self.opt.trim_root_block
            && self.opt.tab == 0
            && arr.inst == IRBLOCK
            && self.opt.take_root_trim_ticket()
    }

    pub fn prepare_root_block(&self, arr: &IRNodeArray) -> (usize, Option<String>) {
        use Bytecode::*;
        // Skip leading empty placeholders (they serialize to nothing but may exist in the IR array to reserve positions for later ALLOC injection).
        let mut first_real_idx: usize = 0;
        while first_real_idx < arr.subs.len()
            && arr.subs[first_real_idx]
                .as_any()
                .downcast_ref::<super::ir::IRNodeEmpty>()
                .is_some()
        {
            first_real_idx += 1;
        }

        // Locate alloc index if present (alloc could be at first_real_idx or first_real_idx+1 due to placeholder insertion patterns).
        let mut alloc_index: Option<usize> = None;
        for (i, s) in arr
            .subs
            .iter()
            .enumerate()
            .skip(first_real_idx)
            .take(2)
        {
            if s.bytecode() == ALLOC as u8 {
                alloc_index = Some(i);
                break;
            }
        }

        if self.opt.trim_param_unpack {
            let param_idx = match alloc_index {
                Some(ai) => ai + 1,
                None => first_real_idx,
            };
            if let Some(line) = self.build_param_line(arr, param_idx) {
                if let Some(names) = self.infer_param_names(arr, param_idx) {
                    for i in 0..names.len() as u8 {
                        self.opt.mark_slot_put(i);
                    }
                }
                return (param_idx + 1, Some(line));
            }
            // No `param { ... }` rewrite for a canonical multi-param prelude
            // (`ALLOC(n)` + `UNPACK(ROLL0, P0)`): the alloc carries the slot
            // count, so trimming it would leave an orphan `unpack(...)` that
            // recompiles to different IR, and the unpack declares no names,
            // so the body must degrade to `$n`. Keep both prelude nodes and
            // suppress the covered slot names — the review channel's proven
            // byte-identical shape. (`n` counts every local slot: without a
            // sourcemap the true param count is not recoverable from the IR.)
            if let Some(ai) = alloc_index {
                if let Some(count) = Self::canonical_unpack_prelude_slots(arr, ai) {
                    for slot in 0..count {
                        self.opt.suppress_slot_name(slot);
                    }
                    return (ai, None);
                }
            }
        }

        if self.opt.trim_head_alloc {
            if let Some(ai) = alloc_index {
                return (ai + 1, None);
            }
        }

        (first_real_idx, None)
    }

    /// If the block contains a canonical param-unpack node at `start_idx`, return the
    /// corresponding `param { ... }` source line.
    pub fn try_build_param_line(&self, arr: &IRNodeArray, start_idx: usize) -> Option<String> {
        self.build_param_line(arr, start_idx)
    }

    fn matches_leaf(node: &dyn IRNode, inst: Bytecode) -> bool {
        node.as_any()
            .downcast_ref::<IRNodeLeaf>()
            .is_some_and(|leaf| leaf.inst == inst)
    }

    fn matches_param_prelude(&self, node: &dyn IRNode, count: u8) -> bool {
        use Bytecode::*;
        match count {
            1 => node
                .as_any()
                .downcast_ref::<IRNodeParam1Single>()
                .is_some_and(|single| {
                    single.inst == PUT
                        && single.para == 0
                        && Self::matches_leaf(&*single.subx, ROLL0)
                }),
            2.. => node
                .as_any()
                .downcast_ref::<IRNodeDouble>()
                .is_some_and(|double| {
                    double.inst == UNPACK
                        && Self::matches_leaf(&*double.subx, ROLL0)
                        && Self::matches_leaf(&*double.suby, P0)
                }),
            _ => false,
        }
    }

    /// Slots covered by a canonical multi-param prelude whose `param { ... }`
    /// rewrite did not happen: `ALLOC(n)` immediately followed by
    /// `UNPACK(ROLL0, P0)`. `n` is the block's total slot count. None for any
    /// other shape — the single-param `PUT(slot 0, ROLL0)` form rebinds itself
    /// as `var $n = roll_0()` and needs no special handling.
    fn canonical_unpack_prelude_slots(arr: &IRNodeArray, alloc_index: usize) -> Option<u8> {
        use Bytecode::*;
        let alloc = arr.subs.get(alloc_index)?;
        if alloc.bytecode() != ALLOC as u8 {
            return None;
        }
        let count = alloc
            .as_any()
            .downcast_ref::<IRNodeParam1>()?
            .para;
        let unpack = arr.subs.get(alloc_index + 1)?;
        unpack
            .as_any()
            .downcast_ref::<IRNodeDouble>()
            .is_some_and(|double| {
                double.inst == UNPACK
                    && Self::matches_leaf(&*double.subx, ROLL0)
                    && Self::matches_leaf(&*double.suby, P0)
            })
            .then_some(count)
    }

    /// Infer parameter names for a compiler-generated param prelude.
    /// Only SourceMap-backed param preludes are safe to rewrite as `param { ... }`.
    pub fn infer_param_names(&self, arr: &IRNodeArray, start_idx: usize) -> Option<Vec<String>> {
        if start_idx >= arr.subs.len() {
            return None;
        }
        let node = &arr.subs[start_idx];
        let map = self.opt.map?;
        let (Some(count), Some(names)) = (map.param_prelude_count(), map.param_names()) else {
            return None;
        };
        if names.len() != count as usize || !self.matches_param_prelude(&**node, count) {
            return None;
        }
        Some(names.clone())
    }

    fn build_param_line(&self, arr: &IRNodeArray, start_idx: usize) -> Option<String> {
        let names = self.infer_param_names(arr, start_idx)?;
        let indent = self.opt.indent.repeat(self.opt.tab);
        let params = names.join(", ");
        Some(format!("{}param {{ {} }}", indent, params))
    }

    pub fn should_flatten_syscall_cat(&self) -> bool {
        self.opt.flatten_syscall_cat
    }
}
