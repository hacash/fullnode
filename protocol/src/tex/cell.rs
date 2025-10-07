
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

    CellTrsZhuIn
    CellTrsZhuOut

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