

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



/*****************************************************/



macro_rules! define_cell_trs_sat { ( $cid: expr, $class: ident, $zhu_op: ident, $state_op: ident ) => {


combi_struct!{ $class,
    cellid: Uint1
    satnum: Fold64
}

impl $class {
    
    pub const CID: u8 = $cid;

    pub fn new(zhu: Fold64) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            satnum: zhu,
        }
    }
}



impl CellExec for $class {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        let sat = Uint8::from(self.satnum.uint());
        $zhu_op(ctx, taradr, &sat)?;
        // tex add
        let tex = ctx.tex_state();
        let Some(zhures) = tex.zhu.$state_op(self.satnum.uint() as i64) else {
            return errf!("cell state coin zhu overflow")
        };
        tex.zhu = zhures;
        Ok(())
    }
}


impl TexCell for $class {}

     
}}



/*****************************************************/



define_cell_trs_sat!{ 3, CellTrsSatIn,  sat_add, checked_sub } 
define_cell_trs_sat!{ 4, CellTrsSatOut, sat_sub, checked_add } 




/*****************************************************/



combi_struct!{ CellTrsDiaIn,
    cellid: Uint1
    dianum: DiamondNumber
}

impl CellTrsDiaIn {
    
    pub const CID: u8 = 5;

    pub fn new(dianum: DiamondNumber) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            dianum,
        }
    }
}



impl CellExec for CellTrsDiaIn {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        // tex add
        let tex = ctx.tex_state();
        tex.record_diamond_in(taradr, self.dianum.uint() as usize)
    }
}


impl TexCell for CellTrsDiaIn {}



combi_struct!{ CellTrsDiaOut,
    cellid: Uint1
    diamonds: DiamondNameListMax200
}

impl CellTrsDiaOut {
    
    pub const CID: u8 = 6;

    pub fn new(diamonds: DiamondNameListMax200) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            diamonds,
        }
    }
}



impl CellExec for CellTrsDiaOut {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        let dn = self.diamonds.check()?;
        let sta = ctx.clone_mut().state();
        let state = &mut CoreState::wrap(sta);
        let dian = DiamondNumber::from_usize(dn)?;
        let dlist = self.diamonds.clone();
        hacd_transfer(state, taradr, &SETTLEMENT_ADDR, &dian, &dlist)?;
        // tex add
        let tex = ctx.tex_state();
        tex.record_diamond_out(self.diamonds.clone())
    }
}


impl TexCell for CellTrsDiaOut {}




/*****************************************************/



macro_rules! define_cell_trs_asset { ( $cid: expr, $class: ident, $asset_op: ident, $state_op: ident ) => {


combi_struct!{ $class,
    cellid: Uint1
    asset:  AssetAmt
}

impl $class {
    
    pub const CID: u8 = $cid;

    pub fn new(asset: AssetAmt) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            asset,
        }
    }
}



impl CellExec for $class {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        let sta = ctx.clone_mut().state();
        let state = &mut CoreState::wrap(sta);
        $asset_op(state, taradr, &self.asset)?;
        // tex add
        let tex = ctx.tex_state();
        let rcd = tex.assets.entry(self.asset.serial).or_insert(0);
        let Some(assetres) = rcd.$state_op(self.asset.amount.uint() as i128) else {
            return errf!("cell state asset <{}> overflow", self.asset.serial.uint())
        };
        *rcd = assetres;
        Ok(())
    }
}


impl TexCell for $class {}

     
}}



/*****************************************************/



define_cell_trs_asset!{ 7, CellTrsAssetIn,  asset_add, checked_sub } 
define_cell_trs_asset!{ 8, CellTrsAssetOut, asset_sub, checked_add } 


