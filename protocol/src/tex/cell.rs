
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
    CellTrsAssetOut    // 8

    CellCondZhuLess    // 11
    CellCondZhuMore    // 12
    CellCondSatLess    // 13
    CellCondSatMore    // 14
    CellCondDiaLess    // 15
    CellCondDiaMore    // 16
    CellCondAssetLess  // 17
    CellCondAssetMore  // 18
    CellCondHeightLess // 19
    CellCondHeightMore // 20

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