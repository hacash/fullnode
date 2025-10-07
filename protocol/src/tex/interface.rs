
pub trait CellExec {
    // return same as ActExec
    fn execute(&self, _: &mut dyn Context, _: &Address) -> Rerr { never!() }
}


pub trait TexCell: Send + Sync + DynClone + Field + CellExec {
    
}


clone_trait_object!{TexCell}

