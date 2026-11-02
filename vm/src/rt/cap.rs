/// Max materialized call-data body (`extract_call_data`): ACTION bodies,
/// Concat-NTFUNC argv, and calc kernel input.
///
/// Calc `input_max` is admitted against `SpaceCap::call_data_size` (this
/// value on L1). Bodies are assembled from ≤ `value_size` chunks in a
/// one-layer `Compo` list; `Tuple` is packed argv and is never a byte carrier.
///
/// CONSENSUS: this is a runtime rule, not height-gated. Changing it is a
/// coordinated consensus upgrade (it widens what ACTION / Concat-NTFUNC
/// bodies are accepted). The cap only rises.
pub const CALL_DATA_MAX: usize = 5120;

/// Default of `SpaceCap::calc_mem_kib_max` and of the calc v1 verify cap.
/// L1 stays at 64 KiB; a sidechain raises the `SpaceCap` field (the only
/// knob) up to `calc::CALC_MEM_KIB_ENGINE_MAX`. Defined here because
/// `SpaceCap` compiles in codec-only builds; the calc module re-exports it.
pub const CALC_MEM_KIB_MAX: u32 = 64;

#[derive(Debug, Clone)]
pub struct SpaceCap {
    pub loaded_contract: usize, // 20
    pub call_depth: usize,      // 32

    pub value_size: usize, // 1280
    /// Max materialized ACTION / Concat-NTFUNC / calc input length
    /// (`extract_call_data` only). Does not apply to `Value::valid`, CAT, PUT,
    /// HREAD, or return values.
    pub call_data_size: usize, // 5120
    pub tuple_length: usize,
    pub compo_length: usize,

    pub storage_period: u64,
    pub storage_live_max_periods: u64,
    pub storage_recv_max_periods: u64,

    pub stack_slot: usize, // 16*16 = 256
    pub local_slot: usize, // 16*16 = 256

    pub heap_segment: usize, // 64: 256 * 64 = 16kb

    pub global: usize,           // 40
    pub memory: usize,           // 24
    pub kv_key_size: usize,      // 128
    pub status_pure_size: usize, // 128

    /// Max total log bytes per execution context (sum of `val_size()` across all LOG1..LOG4 calls).
    /// Set to 0 to disable logging entirely.
    pub log_size: usize, // 9600

    pub contract_size: usize,         // 65535 * 1
    pub function_size: usize,         // 65535 / 4
    pub inherit: usize,               // 12
    pub library: usize,               // 64
    pub p2sh_set: usize,              // 128
    pub p2sh_merkle_depth_max: usize, // 8
    pub p2sh_lockbox_size_max: usize, // 4096
    pub reentry_level: u32,           // 1, ACTION re-entry level limit

    pub intent_bind_depth: usize, // 10
    /// Max intent instances creatable per execution context (`IntentRuntime` total_created cap).
    pub intent_new: usize,
    /// Max keys per intent map instance (MKVMap entry cap and batch put limits).
    pub intent_key: usize,

    /// Deploy admission for a calc kernel's `mem_kib`. L1 stays at 64.
    /// A sidechain raises this up to `calc::CALC_MEM_KIB_ENGINE_MAX`
    /// (2_097_147 KiB, about 2 GiB). The container field is already `u32`;
    /// this is the only knob.
    pub calc_mem_kib_max: u32,
    /// Max encoded calc container (header + code + rodata + init).
    /// Independent of `function_size`. Default matches `function_size`
    /// (65535/4). The contract body prefix is `u16`, so this knob stops
    /// at 65535; a sidechain may raise the default up to that.
    pub calc_image_max: usize,
}

impl Default for SpaceCap {
    fn default() -> Self {
        Self::new(0)
    }
}

impl SpaceCap {
    pub const DEFAULT_TUPLE_LENGTH: usize = 32;

    /// Upper bound on scalar payload bytes for field `Value` serialization (`Bytes` length must be `< u16::MAX`).
    pub const FIELD_BYTES_SERIALIZE_MAX: usize = u16::MAX as usize - 1;

    pub fn new(_hei: u64) -> SpaceCap {
        const U16M: usize = u16::MAX as usize; // 65535

        SpaceCap {
            loaded_contract: 20,
            call_depth: 32,
            value_size: 1280, // = 32 * 40, diamond name list max bytes: 200*6 = 1200
            call_data_size: CALL_DATA_MAX,
            tuple_length: Self::DEFAULT_TUPLE_LENGTH,
            compo_length: 128,
            storage_period: 100,
            storage_live_max_periods: 30000,
            storage_recv_max_periods: 3000,
            stack_slot: 256,
            local_slot: 256,
            heap_segment: 64,
            global: 40,
            memory: 24,
            kv_key_size: 128,
            status_pure_size: 128,
            log_size: 9600,
            contract_size: U16M * 1, // 65535*1
            function_size: U16M / 4, // 65535/4
            inherit: 12,
            library: 64,
            p2sh_set: 128,
            p2sh_merkle_depth_max: 8,
            p2sh_lockbox_size_max: 4096,
            reentry_level: 1, // allow 1 re-entry (2 call layers total)
            intent_bind_depth: 10,
            intent_new: 128,
            intent_key: 32,
            calc_mem_kib_max: CALC_MEM_KIB_MAX,
            calc_image_max: (u16::MAX as usize) / 4,
        }
    }

    /// Whether an encoded scalar length is allowed for persistence under `value_max` (per-key/per-field cap).
    #[inline(always)]
    pub fn scalar_field_len_fits(len: usize, value_max: usize) -> bool {
        len < u16::MAX as usize && len <= value_max
    }

    #[inline(always)]
    pub fn field_scalar_len_ok(&self, len: usize) -> bool {
        Self::scalar_field_len_fits(len, self.value_size)
    }

    /// Injected runtime configs must not use `storage_period == 0` (division / rent math); normalize to default.
    pub fn normalize_zero_storage_period(&mut self, height: u64) {
        if self.storage_period == 0 {
            self.storage_period = SpaceCap::new(height).storage_period;
        }
    }

    #[inline(always)]
    pub fn storage_live_max_blocks(&self) -> u64 {
        self.storage_period
            .saturating_mul(self.storage_live_max_periods)
    }

    #[inline(always)]
    pub fn storage_recv_max_blocks(&self) -> u64 {
        self.storage_period
            .saturating_mul(self.storage_recv_max_periods)
    }
}

/// Execution context for NTFUNC dispatch. Catalog metadata stays in
/// `native_catalog`; the interpreter supplies the active `SpaceCap`.
#[derive(Clone, Copy)]
pub struct NativeFnEnv<'a> {
    pub cap: &'a SpaceCap,
}

impl<'a> NativeFnEnv<'a> {
    #[inline]
    pub fn new(cap: &'a SpaceCap) -> Self {
        Self { cap }
    }
}
