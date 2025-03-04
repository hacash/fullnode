

pub trait TxPool: Send + Sync {
    // for group
    fn count_at(&self,  _: usize) -> Ret<usize> { Ok(0) }
    fn first_at(&self,  _: usize) -> Ret<Option<TxPkg>> { Ok(None) }
    fn iter_at(&self,   _: usize, _: &mut dyn FnMut(&TxPkg)->bool) -> Rerr { Ok(()) }
    fn insert_at(&self, _: usize, _: TxPkg) -> Rerr { Ok(()) } // from group id
    fn delete_at(&self, _: usize, _: &[Hash]) -> Rerr { Ok(()) } // from group id
    fn find_at(&self,   _: usize, _: &Hash) -> Option<TxPkg> { None } // from group id
    fn clear_at(&self,  _: usize) -> Rerr { Ok(()) } // by group id
    fn retain_at(&self, _: usize, _: &mut dyn FnMut(&TxPkg)->bool) -> Rerr { Ok(()) }
    // all
    fn insert_by(&self, _: TxPkg, _: &dyn Fn(&TxPkg)->usize) -> Rerr { Ok(()) }
    fn find(&self,   _: &Hash) -> Option<TxPkg> { None }
    fn drain(&self,  _: &[Hash]) -> Ret<Vec<TxPkg>> { Ok(vec![]) }
    // 
    fn print(&self) -> String { s!("") }
}



