
#[derive(Debug, Clone, Default)]
pub struct SpaceCap {
    pub max_gas_of_tx: usize, // 65535
    pub call_depth: usize,    // 16 max 127        
    pub load_contract: usize, // 20

    pub max_value_size: usize,

    pub total_stack: usize, // 16*16 = 256
    pub total_local: usize, // 16*16 = 256

    pub max_heap_seg: usize, // 64: 256 * 64 = 16kb

    pub max_global: usize, // 32
    pub max_memory: usize, // 12

    pub max_contract_size: usize, // 65535
    pub inherits_parent: usize, // 4
    pub librarys_link:   usize, // 250

    // pub max_ctl_func: usize, // 200 cache
    // pub max_ctl_libx: usize, // 100 cache
    // pub max_ctl_body: usize, // 50  cache

}

impl SpaceCap {

    pub fn new(_hei: u64) -> SpaceCap {

        SpaceCap {
            max_gas_of_tx:   65535,
            call_depth:      16,
            load_contract:   20,
            max_value_size:  2048, 
            total_stack:     256,
            total_local:     256,
            max_heap_seg:    64,
            max_global:      20,
            max_memory:      12,
            max_contract_size: (u16::MAX as usize) * 2, // 65535*2
            inherits_parent: 4,
            librarys_link:   250,
            // max_ctl_func:   200,
            // max_ctl_libx:   100,
            // max_ctl_body:   50,
        }
    }

}



