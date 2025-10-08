
macro_rules! define_tex_cell_create { ($f: ident, $( $ty: ty )+) => {
     

fn tex_cell_create(buf: &[u8])->Ret<(Box<dyn TexCell>, usize)>{
    let (cid, _) = Uint1::create(buf)?;
    Ok(match cid.uint() {
        $(
        <$ty>::CID => {
            let (obj, sz) = <$ty>::create(buf)?;
            (Box::new(obj), sz)
        }
        )+
        i => return errf!("cannot find tex cell id '{}'", i)
    })
}

   
}}




define_tex_cell_create!{ tex_cell_create, 

    CellTrsZhuIn       // 1
    CellTrsZhuOut      // 2
    CellTrsSatIn       // 3
    CellTrsSatOut      // 4
    CellTrsDiaIn       // 5
    CellTrsDiaOut      // 6
    CellTrsAssetIn     // 7
    CellTrsAssetOut    // G    CellCondZhuLe    // 11
    
    CellCondZhuLe    // 11
    CellCondZhuGe    // 12
    CellCondZhuEq    // 13
    CellCondSatLe    // 14
    CellCondSatGe    // 15
    CellCondSatEq    // 16
    CellCondDiaLe    // 17
    CellCondDiaGe    // 18
    CellCondDiaEq    // 19
    CellCondAssetLe  // 20
    CellCondAssetGe  // 21
    CellCondAssetEq  // 22
    CellCondHeightLe // 23
    CellCondHeightGe // 24
    

}



combi_dynlist!{ DnyTexCellListW1, Uint1, TexCell, tex_cell_create}



impl CellExec for DnyTexCellListW1 {
    fn execute(&self, ctx: &mut dyn Context, main: &Address) -> Rerr {        
        for cell in self.list() {
            cell.execute(ctx, main)?;
        }
        Ok(())
    }
}