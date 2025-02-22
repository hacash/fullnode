

pub trait TxPool: Send + Sync {
    fn count_at(&self, _: usize) -> Ret<usize> { Ok(0) }
    fn first_at(&self, _: usize) -> Ret<Option<TxPkg>> { Ok(None) }
    fn iter_at(&self, _: &mut dyn FnMut(&TxPkg)->bool, _: usize) -> Rerr { Ok(()) }
    fn insert_at(&self, _: TxPkg, _: usize) -> Rerr { Ok(()) } // from group id
    fn delete_at(&self, _: &[Hash], _: usize) -> Rerr { Ok(()) } // from group id
    fn find_at(&self, _: &Hash, _: usize) -> Option<TxPkg> { None } // from group id
    fn clear_at(&self, _: usize) -> Rerr { Ok(()) } // by group id
    fn retain_at(&self, _: &mut dyn FnMut(&TxPkg)->bool, _: usize) 
        -> Rerr { Ok(()) }

    fn find(&self, _hx: &Hash) -> Option<TxPkg> { None }
    fn insert(&self, _: TxPkg) -> Rerr { Ok(()) }
    fn drain(&self, _: &[Hash]) -> Ret<Vec<TxPkg>> { Ok(vec![]) }

    fn print(&self) -> String { s!("") }
}