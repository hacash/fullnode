/***********************************/

// Per-opcode base gas units (`GasTable::gas`). The interpreter may add dynamic metering
// on top (e.g. `FINPOW3` exponent bits, compare on wide integers).
//
// Arithmetic ladder (see `GasTable::new` grouping):
// - 2: add/sub/min/max/inc/dec - single-step integer ops on stack slots.
// - 4: mul/div/mod, abs diff - widening or division.
// - 6: div_up/div_exact_op, POW, ADDMOD, CLAMP - extra branches or triple operands without full mul-div path.
// - 8: MULADD, MULSUB - one multiply plus add/sub.
// - 10: MULMOD - multiply then mod.
// - 12: MULDIV/MULDIVUP - wide multiply and divide.
// - 6/10/18: FIN* family base decode+dispatch tiers.
// - 32: FINPOW3 - high base like storage reads; extra per exponent in execute.
//
// Unrelated opcodes may share a tier when base interpreter cost is similar.
// Reserved bytecode bytes stay at default 1.

#[derive(Clone)]
pub struct GasTable {
    table: [u8; 256],
}

impl Default for GasTable {
    fn default() -> Self {
        Self { table: [1; 256] }
    }
}

impl GasTable {
    pub fn new(_hei: u64) -> Self {
        let mut gst = Self { table: [1; 256] };
        gst.set(2, &[BRL, BRS, BRSL, BRSLN, XLG, XOP]);
        gst.set(2, &[AND, OR, EQ, NEQ, LT, GT, LE, GE, NOT]);
        gst.set(3, &[BSHR, BSHL, BXOR, BOR, BAND]);
        // Arithmetic: binary (see module doc ladder)
        gst.set(2, &[ADD, SUB, MAX, MIN, INC, DEC]);
        gst.set(4, &[MUL, DIV, MOD, ABSDIFF]);
        gst.set(5, &[SQRT, SQRTUP]);
        gst.set(6, &[DIVUP, DIVEXACT, POW, ADDMOD, CLAMP, FIN2]);
        gst.set(10, &[FIN3, FINP3, FINP4]);
        gst.set(18, &[FIN4]);
        gst.set(32, &[FINPOW3]);
        // Arithmetic: triple-operand mul pipeline
        gst.set(8, &[MULADD, MULSUB]);
        gst.set(10, &[MULMOD]);
        gst.set(12, &[MULDIV, MULDIVUP]);
        // Arithmetic: four-operand handled by FIN4 family
        // Other
        gst.set(4, &[INSERT, REMOVE, TAKEFIRST, TAKELAST, APPEND]);
        gst.set(5, &[MGET, GGET, NEWLIST, NEWMAP]);
        gst.set(6, &[CLEAR, KEYS, VALUES, TUPLE2LIST, UNPACK]);
        gst.set(8, &[CLONE, MERGE, PACKLIST, PACKMAP, PACKTUPLE]);
        gst.set(10, &[MPUT, GPUT, CALLSELF, CALLSELFVIEW, CALLSELFPURE]);
        gst.set(12, &[MTAKE, CALLUSEVIEW, CALLUSEPURE]);
        gst.set(16, &[NTENV, NTCTL, NTFUNC, CALLTHIS, CALLSUPER, CODECALL]);
        gst.set(20, &[LOG1, CALLEXTVIEW]);
        gst.set(24, &[LOG2, CALLEXT, CALL]);
        gst.set(28, &[LOG3, ACTENV, SDEL]);
        gst.set(32, &[LOG4, ACTVIEW, SLOAD, SSTAT]);
        gst.set(48, &[ACTION]);
        gst.set(64, &[SNEW, SEDIT, SRENT, SRECV, SGET]);
        gst.set(128, &[SPUT]);
        gst
    }

    fn set(&mut self, gas: u8, btcds: &[Bytecode]) {
        for cd in btcds {
            let i = *cd as usize;
            self.table[i] = gas;
        }
    }

    #[inline(always)]
    pub fn gas(&self, code: u8) -> i64 {
        self.table[code as usize] as i64
    }
}

/***********************************/

#[derive(Default, Clone)]
pub struct GasExtra {
    pub compute_limit: i64,  // <=0 means disabled
    pub resource_limit: i64, // <=0 means disabled
    pub storage_limit: i64,  // <=0 means disabled
    pub one_local_alloc: i64,
    pub new_contract_load: i64,
    pub main_call_base: i64,
    pub p2sh_call_base: i64,
    pub abst_call_base: i64,
    // Space alloc
    pub memory_key_cost: i64,
    pub global_key_cost: i64,
    pub storege_value_base_size: i64,
    pub storage_key_cost: i64,
    pub storage_edit_mul: i64,
    // Status dynamic gas is independently priced from storage gas.
    pub status_read_byte_mul: i64,
    pub status_write_key_byte_mul: i64,
    pub status_write_value_byte_mul: i64,
    pub container_cmp_header: usize,
    stack_move_item: i64,
    // Dynamic, resource-based gas parameters.
    stack_copy_div: i64,
    stack_write_div: i64,
    stack_cmp_div: i64,
    stack_op_div: i64,
    heap_read_div: i64,
    heap_write_div: i64,
    log_div: i64,
    compile_div: i64,
    contract_div: i64,
    compo_byte_div: i64,
    compo_item_read_div: i64,
    compo_item_edit_div: i64,
    compo_item_copy_div: i64,
    ntfunc_div: i64,
    act_div: i64,
    burn_div: i64,
    rpow_exp_bit_mul: i64,
    rpow_exp_base: i64,
    heap_grow_exp_segments: usize,
    heap_grow_linear_seg: u64,
}

impl GasExtra {
    pub fn new(_hei: u64) -> Self {
        use base::{decode_gas_budget};
        Self {
            compute_limit: decode_gas_budget(72),  // 18009
            resource_limit: decode_gas_budget(56), // 6100
            storage_limit: decode_gas_budget(99),  // 111911
            // Load or alloc
            one_local_alloc: 5,    // 5 * num
            new_contract_load: 32, // base gas for loading a new contract
            main_call_base: 3 * 16, // 48
            p2sh_call_base: 4 * 16, // 64
            abst_call_base: 5 * 16, // 80
            // Space alloc
            memory_key_cost: 20,
            global_key_cost: 32,
            storege_value_base_size: 20,
            storage_key_cost: 1024,
            storage_edit_mul: 4,
            status_read_byte_mul: 8,
            status_write_key_byte_mul: 32,
            status_write_value_byte_mul: 32,
            // other
            container_cmp_header: 12,
            // Dynamic divisors (byte/N, item/N)
            stack_copy_div: 32,
            stack_write_div: 28,
            stack_cmp_div: 24,
            stack_op_div: 20,
            stack_move_item: 1,
            heap_read_div: 16,
            heap_write_div: 12,
            log_div: 1,
            compile_div: 16,
            contract_div: 64,
            ntfunc_div: 16,
            act_div: 12,
            burn_div: 1,
            rpow_exp_bit_mul: 8,
            rpow_exp_base: 1,
            heap_grow_exp_segments: 8,
            heap_grow_linear_seg: 256,
            // Compo
            compo_byte_div: 40,
            compo_item_read_div: 4,
            compo_item_edit_div: 2,
            compo_item_copy_div: 1,
        }
    }

    #[inline(always)]
    fn div_op(len: usize, div: i64) -> i64 {
        if div <= 0 || len == 0 {
            return 0;
        }
        (len as i64 - 1) / div + 1
    }

    #[inline(always)]
    fn linear_bytes(len: usize, mul: i64) -> i64 {
        if mul <= 0 || len == 0 {
            return 0;
        }
        let len = i64::try_from(len).unwrap_or(i64::MAX);
        len.saturating_mul(mul)
    }

    #[inline(always)]
    pub fn stack_copy(&self, len: usize) -> i64 {
        Self::div_op(len, self.stack_copy_div)
    }

    #[inline(always)]
    pub fn stack_write(&self, len: usize) -> i64 {
        Self::div_op(len, self.stack_write_div)
    }

    #[inline(always)]
    pub fn stack_cmp(&self, len: usize) -> i64 {
        Self::div_op(len, self.stack_cmp_div)
    }

    #[inline(always)]
    pub fn stack_op(&self, len: usize) -> i64 {
        Self::div_op(len, self.stack_op_div)
    }

    #[inline(always)]
    pub fn stack_move_items(&self, n: usize) -> i64 {
        if n == 0 {
            return 0;
        }
        self.stack_move_item.saturating_mul(n as i64)
    }

    #[inline(always)]
    pub fn nt_bytes(&self, len: usize) -> i64 {
        Self::div_op(len, self.ntfunc_div)
    }

    #[inline(always)]
    pub fn act_bytes(&self, len: usize) -> i64 {
        Self::div_op(len, self.act_div)
    }

    #[inline(always)]
    pub fn heap_read(&self, len: usize) -> i64 {
        Self::div_op(len, self.heap_read_div)
    }

    #[inline(always)]
    pub fn heap_write(&self, len: usize) -> i64 {
        Self::div_op(len, self.heap_write_div)
    }

    #[inline(always)]
    pub fn log_bytes(&self, total_bytes: usize) -> i64 {
        Self::div_op(total_bytes, self.log_div)
    }

    #[inline(always)]
    pub fn storage_read(&self, val_len: usize) -> i64 {
        (val_len as i64).saturating_add(self.storege_value_base_size.max(0))
    }

    #[inline(always)]
    pub fn storage_write(&self, val_len: usize) -> i64 {
        self.storage_read(val_len).saturating_mul(2)
    }

    #[inline(always)]
    pub fn status_read(&self, val_len: usize) -> i64 {
        Self::linear_bytes(val_len, self.status_read_byte_mul)
    }

    #[inline(always)]
    pub fn status_write(&self, key_len: usize, val_len: usize) -> i64 {
        Self::linear_bytes(key_len, self.status_write_key_byte_mul).saturating_add(
            Self::linear_bytes(val_len, self.status_write_value_byte_mul),
        )
    }

    #[inline(always)]
    pub fn compile_bytes(&self, len: usize) -> i64 {
        Self::div_op(len, self.compile_div)
    }

    #[inline(always)]
    pub fn ir_format_bytes(&self, raw_ir_len: usize) -> i64 {
        Self::div_op(raw_ir_len, self.compile_div)
    }

    #[inline(always)]
    pub fn compo_items_read(&self, n: usize) -> i64 {
        Self::div_op(n, self.compo_item_read_div)
    }

    #[inline(always)]
    pub fn compo_items_edit(&self, n: usize) -> i64 {
        Self::div_op(n, self.compo_item_edit_div)
    }

    #[inline(always)]
    pub fn compo_items_copy(&self, n: usize) -> i64 {
        Self::div_op(n, self.compo_item_copy_div)
    }

    #[inline(always)]
    pub fn compo_bytes(&self, len: usize) -> i64 {
        Self::div_op(len, self.compo_byte_div)
    }

    #[inline(always)]
    pub fn contract_bytes(&self, len: usize) -> i64 {
        Self::div_op(len, self.contract_div)
    }

    #[inline(always)]
    pub fn burn_extra(&self, raw: i64) -> i64 {
        Self::div_op(raw.max(0) as usize, self.burn_div)
    }

    #[inline(always)]
    pub fn rpow_extra(&self, exp_bits: i64) -> i64 {
        if exp_bits <= 0 {
            return 0;
        }
        exp_bits
            .saturating_mul(self.rpow_exp_bit_mul)
            .saturating_add(self.rpow_exp_base)
    }

    #[inline(always)]
    pub fn heap_grow_gas(&self, oldseg: usize, seg: usize, limit: usize) -> VmrtRes<i64> {
        let newseg = oldseg
            .checked_add(seg)
            .ok_or_else(|| ItrErr::new(OutOfHeap, "heap segment overflow"))?;
        if newseg > limit {
            return Err(ItrErr::new(OutOfHeap, "out of heap"));
        }
        // Gas is an abstraction of space usage: the first 8 segments are charged exponentially (2,4,8,16,32,64,128,256), then linear 256 per segment. Price is based on existing heap size so multiple HGROW(1) cannot bypass.
        let mut gas: u64 = 0;
        for s in oldseg..newseg {
            let add = if s < self.heap_grow_exp_segments {
                1u64.checked_shl((s + 1) as u32).unwrap_or(u64::MAX)
            } else {
                self.heap_grow_linear_seg
            };
            gas = gas
                .checked_add(add)
                .ok_or_else(|| ItrErr::new(HeapError, "heap grow gas overflow"))?;
        }
        i64::try_from(gas).map_err(|_| ItrErr::new(HeapError, "heap grow gas overflow"))
    }
}

/***************************************/

