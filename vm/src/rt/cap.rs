#[derive(Debug, Clone, Default)]
pub struct SpaceCap {
    pub loaded_contract: usize, // 20
    pub call_depth: usize,      // 32

    pub value_size: usize, // 1280
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
    pub log_size: usize,   // 9600

    pub contract_size: usize, // 65535 * 1
    pub function_size: usize, // 65535 / 4
    pub inherit: usize,       // 12
    pub library: usize,       // 64
    pub p2sh_set: usize,      // 128
    pub p2sh_merkle_depth_max: usize, // 8
    pub p2sh_lockbox_size_max: usize, // 4096
    pub reentry_level: u32,   // 1, ACTION re-entry level limit

    pub intent_bind_depth: usize, // 10
    /// Max intent instances creatable per execution context (`IntentRuntime` total_created cap).
    pub intent_new: usize,
    /// Max keys per intent map instance (MKVMap entry cap and batch put limits).
    pub intent_key: usize,
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

