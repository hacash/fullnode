
macro_rules! define_cell_cond_zhu { ( $cid: expr, $class: ident, $check_op: ident) => {


combi_struct!{ $class,
    cellid: Uint1
    haczhu: Fold64
}

impl $class {
    
    pub const CID: u8 = $cid;

    pub fn new(haczhu: Fold64) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            haczhu,
        }
    }
}



impl CellExec for $class {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        let sta = ctx.clone_mut().state();
        let bls = CoreState::wrap(sta).balance(taradr).unwrap_or_default();
        let err = ||errf!("cell condition zhu check failed");
        let Some(zhu) = bls.hacash.to_zhu_u128() else {
            return err();
        };
        if zhu > u64::MAX as u128 {
            return err();
        }
        let zhu = zhu as u64;
        match self.haczhu.uint().$check_op(&zhu) {
            true => Ok(()),
            false => err(),
        }
    }
}


impl TexCell for $class {}

}}



define_cell_cond_zhu!{ 11, CellCondZhuLess, le }
define_cell_cond_zhu!{ 12, CellCondZhuMore, ge }



/*****************************************************/



macro_rules! define_cell_cond_sat { ( $cid: expr, $class: ident, $check_op: ident) => {


combi_struct!{ $class,
    cellid: Uint1
    satoshi: Fold64
}

impl $class {
    
    pub const CID: u8 = $cid;

    pub fn new(satoshi: Fold64) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            satoshi,
        }
    }
}



impl CellExec for $class {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        let sta = ctx.clone_mut().state();
        let sat = CoreState::wrap(sta).balance(taradr).unwrap_or_default().satoshi.uint();
        let err = ||errf!("cell condition sat check failed");
        match self.satoshi.uint().$check_op(&sat) {
            true => Ok(()),
            false => err(),
        }
    }
}


impl TexCell for $class {}

}}



define_cell_cond_sat!{ 13, CellCondSatLess, le }
define_cell_cond_sat!{ 14, CellCondSatMore, ge }



/*****************************************************/



macro_rules! define_cell_cond_dia { ( $cid: expr, $class: ident, $check_op: ident) => {


combi_struct!{ $class,
    cellid: Uint1
    diamond: Fold64
}

impl $class {
    
    pub const CID: u8 = $cid;

    pub fn new(diamond: Fold64) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            diamond,
        }
    }
}



impl CellExec for $class {

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        let sta = ctx.clone_mut().state();
        let dia = CoreState::wrap(sta).balance(taradr).unwrap_or_default().diamond.uint();
        let err = ||errf!("cell condition dia check failed");
        match self.diamond.uint().$check_op(&dia) {
            true => Ok(()),
            false => err(),
        }
    }
}


impl TexCell for $class {}

}}



define_cell_cond_dia!{ 15, CellCondDiaLess, le }
define_cell_cond_dia!{ 16, CellCondDiaMore, ge }



/*****************************************************/




/*****************************************************/



macro_rules! define_cell_cond_asset { ( $cid: expr, $class: ident, $check_op: ident) => {


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
        let bls = CoreState::wrap(sta).balance(taradr).unwrap_or_default();
        let aid = self.asset.serial;
        let ast = bls.asset_must(aid);
        let err = ||errf!("cell condition asset <{}> check failed", aid.uint());
        match self.asset.amount.uint().$check_op(&ast.amount.uint()) {
            true => Ok(()),
            false => err(),
        }
    }
}


impl TexCell for $class {}

}}



define_cell_cond_asset!{ 17, CellCondAssetLess, le }
define_cell_cond_asset!{ 18, CellCondAssetMore, ge }



/*****************************************************/



macro_rules! define_cell_cond_height { ( $cid: expr, $class: ident, $check_op: ident) => {


combi_struct!{ $class,
    cellid: Uint1
    height: BlockHeight
}

impl $class {
    
    pub const CID: u8 = $cid;

    pub fn new(hei: u64) -> Self {
        Self {
            cellid: Uint1::from(Self::CID),
            height: BlockHeight::from(hei),
        }
    }
}



impl CellExec for $class {

    fn execute(&self, ctx: &mut dyn Context, _: &Address) -> Rerr {
        let chei = ctx.env().block.height;
        match self.height.uint().$check_op(&chei) {
            true => Ok(()),
            false => errf!("cell condition check failed")
        }
    }
}


impl TexCell for $class {}

}}



define_cell_cond_height!{ 19, CellCondHeightLess, le }
define_cell_cond_height!{ 20, CellCondHeightMore, ge }



/*****************************************************/


