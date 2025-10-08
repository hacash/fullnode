
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
        let cnd = self.haczhu.uint().$check_op(&zhu);
        maybe!(cnd, Ok(()), err())
    }
}


impl TexCell for $class {}

}}



define_cell_cond_zhu!{ 11, CellCondZhuLe, le }
define_cell_cond_zhu!{ 12, CellCondZhuGe, ge }
define_cell_cond_zhu!{ 13, CellCondZhuEq, eq }



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
        let cnd = self.satoshi.uint().$check_op(&sat);
        maybe!(cnd, Ok(()), err())
    }
}


impl TexCell for $class {}

}}



define_cell_cond_sat!{ 14, CellCondSatLe, le }
define_cell_cond_sat!{ 15, CellCondSatGe, ge }
define_cell_cond_sat!{ 16, CellCondSatEq, eq }



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
        let cnd = self.diamond.uint().$check_op(&dia);
        maybe!(cnd, Ok(()), err())
    }
}


impl TexCell for $class {}

}}



define_cell_cond_dia!{ 17, CellCondDiaLe, le }
define_cell_cond_dia!{ 18, CellCondDiaGe, ge }
define_cell_cond_dia!{ 19, CellCondDiaEq, eq }



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
        let cnd = self.asset.amount.uint().$check_op(&ast.amount.uint());
        maybe!(cnd, Ok(()), err())
    }
}


impl TexCell for $class {}

}}



define_cell_cond_asset!{ 20, CellCondAssetLe, le }
define_cell_cond_asset!{ 21, CellCondAssetGe, ge }
define_cell_cond_asset!{ 22, CellCondAssetEq, eq }



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
        let err = ||errf!("cell condition check failed");
        let cnd = self.height.uint().$check_op(&chei);
        maybe!(cnd, Ok(()), err())
    }
}


impl TexCell for $class {}

}}



define_cell_cond_height!{ 23, CellCondHeightLe, le }
define_cell_cond_height!{ 24, CellCondHeightGe, ge }



/*****************************************************/


