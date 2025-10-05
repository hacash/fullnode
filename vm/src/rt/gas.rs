
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
        gst.set(1,  &[CU8, CU16, CU32, CU64, CU128, CBUF, CTO, TID, TIS, TNIL, TMAP, TLIST, PU8, PU16, P0, P1, P2, P3, PNBUF, PNIL, DUP, POP, NOP, NT, END, RET, ABT, ERR, AST, PRT]);
        // gst.set(2,  &[...]); // all other bytecode
        gst.set(3,  &[XLG, PUT, CHOISE, BRL, BRS, BRSL, BRSLN]);
        gst.set(4,  &[XOP, HREAD, HREADU, HREADUL, MOD, MUL, DIV, ITEMGET, HASKEY, LENGTH]);
        gst.set(5,  &[POW, KEYS, VALUES]);
        gst.set(6,  &[HWRITE, HWRITEX, HWRITEXL, INSERT, REMOVE, CLEAR, APPEND]);
        gst.set(8,  &[MGET, JOIN, REV, NEWLIST, NEWMAP]);
        gst.set(12, &[EXTENV, MPUT, CALLINR, PACKLIST, PACKMAP, UPLIST, CLONE]);
        gst.set(16, &[EXTFUNC,GGET, CALLCODE]);
        gst.set(24, &[EXTACTION, GPUT, CALLLIB, CALLSTATIC]);
        gst.set(32, &[SLOAD, STIME, CALL]); // CALLDYN
        gst.set(64, &[SSAVE, SRENT]);
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
    pub storege_value_base_size: i64,
    pub load_new_contract: i64,
    pub main_call_min: i64,
    pub abst_call_min: i64,
}

impl GasExtra {
    pub fn new(_hei: u64) -> Self {
        Self {
            local_one_alloc: 5, // 5 * num
            storege_value_base_size: 32,
            load_new_contract: 2 * GSCU as i64, // 64
            main_call_min:     1 * GSCU as i64, // 64
            abst_call_min:     4 * GSCU as i64,     // 32
        }
    }
}


