
#[derive(Debug, Clone, Default)]
pub struct SpaceCap {
    pub max_gas_of_tx: usize, // 65535
    pub load_contract: usize, // 20
    pub call_depth: usize,    // 32

    pub max_value_size: usize, // 1280
    pub max_compo_length: usize,

    pub total_stack: usize, // 16*16 = 256
    pub total_local: usize, // 16*16 = 256

    pub max_heap_seg: usize, // 64: 256 * 64 = 16kb

    pub max_global: usize, // 32
    pub max_memory: usize, // 12

    pub max_contract_size: usize, // 65535 * 2
    pub one_function_size: usize, // 65535 / 4
    pub inherits_parent: usize, // 4
    pub librarys_link:   usize, // 250

    // pub max_ctl_func: usize, // 200 cache
    // pub max_ctl_libx: usize, // 100 cache
    // pub max_ctl_body: usize, // 50  cache

}

impl SpaceCap {

    pub fn new(_hei: u64) -> SpaceCap {
        const U16M: usize = u16::MAX as usize; // 65535

        SpaceCap {
            max_gas_of_tx:     U16M / 4, // 65535/4
            load_contract:       20,
            call_depth:          32,
            max_value_size:    1280, // = 32 * 40, diamond name list max bytes: 200*6 = 1200 
            max_compo_length:   128,
            total_stack:        256,
            total_local:        256,
            max_heap_seg:        64,
            max_global:          20,
            max_memory:          16,
            max_contract_size: U16M * 2, // 65535*2
            one_function_size: U16M / 4, // 65535/4
            inherits_parent:      4,
            librarys_link:      100,
            // max_ctl_func:   200,
            // max_ctl_libx:   100,
            // max_ctl_body:   50,
        }
    }

}



