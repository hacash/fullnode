
#[derive(Default)]
pub struct GasUse {
    pub compute: i64,
    pub storage: i64,
}

impl GasUse {
    pub fn total(&self) -> i64 {
        self.compute + self.storage
    }
}


/***********************************/


pub struct GasTable {
    table: [u8; 256]
}

impl Default for GasTable {
    fn default() -> Self {
        Self {
            table: [0; 256]
        }
    }
}


impl GasTable {

    pub fn new(_hei: u64) -> Self {
        use Bytecode::*;
        let mut gst = Self { table : [2; 256] };
        gst.set(1,  &[CU8, CU16, CU32, CU64, CU128, CBUF, TYPEID, PU8, PU16, P0, P1, PNBUF, DUP, POP, NOP, RET, END, AST, ERR, ABT]);
        // gst.set(2,  &[...]); // all other bytecode
        gst.set(3,  &[XLG, PUT, PUTX, MOVE, CHIOSE, BRL, BRS, BRSL, BRSLN]);
        gst.set(4,  &[XOP, HREAD, HREADU, HREADUL, MOD, MUL, DIV]);
        gst.set(5,  &[POW]);
        gst.set(6,  &[HWRITE, HWRITEX, HWRITEXL]);
        gst.set(8,  &[MGET, JOIN, REV]);
        gst.set(12, &[MPUT, CALLLOC]);
        gst.set(16, &[GGET, CALLCODE]);
        gst.set(24, &[GPUT, CALLLIB, CALLSTATIC]);
        gst.set(32, &[SLOAD, CALL]);
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


#[derive(Default)]
pub struct GasExtra {
    pub local_one_alloc: i64,
    pub local_put: i64,
    pub memory_put: i64, 
    pub memory_get: i64, 
    pub global_put: i64, 
    pub global_get: i64, 
    pub storage_read: i64,
    pub storage_save_base: i64,
    pub storage_recover: i64,
    pub load_one_new_contract: i64,
}

impl GasExtra {
    pub fn new(_hei: u64) -> Self {
        Self {
            local_one_alloc: 5, // 5 * num
            local_put: 1,   // 3
            memory_get: 6,  // 8
            memory_put: 10, // 12
            global_get: 14, // 16
            global_put: 22, // 24
            storage_read: 30, // 32,
            storage_save_base: 42, // (42+vlen) * period
            storage_recover: 62, // 64 for data recover
            load_one_new_contract: 64,
        }
    }
}


