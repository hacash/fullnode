

/*****************************************************/

macro_rules! define_cell_trs_zhu { ( $cid: expr, $class: ident, $zhu_op: ident, $state_op: ident ) => {


combi_struct!{ $class,
    cellid: Uint1
    haczhu: Fold64
}

impl $class {
    
    pub const CID: u8 = $cid;

    pub fn new(zhu: Fold64) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            haczhu: zhu,
        }
    }
}



impl CellExec for $class {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        let amt = Amount::zhu(self.haczhu.uint());
        $zhu_op(ctx, taradr, &amt)?;
        // tex add
        let tex = ctx.tex_state();
        let Some(zhures) = tex.zhu.$state_op(self.haczhu.uint() as i64) else {
            return errf!("cell state coin zhu overflow")
        };
        tex.zhu = zhures;
        Ok(())
    }
}


impl TexCell for $class {}

     
}}


/*****************************************************/


define_cell_trs_zhu!{ 1, CellTrsZhuIn,  hac_add, checked_sub } 
define_cell_trs_zhu!{ 2, CellTrsZhuOut, hac_sub, checked_add } 

