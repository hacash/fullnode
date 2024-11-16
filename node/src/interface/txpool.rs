

pub trait TxPool: Send + Sync {
    fn count_at(&self, _: usize) -> Ret<usize> { Ok(0) }
    fn iter_at(&self, _: &mut dyn FnMut(&Box<TxPkg>)->bool, _: usize) -> Rerr { Ok(()) }
    fn insert_at(&self, _: Box<TxPkg>, _: usize) -> Rerr { Ok(()) } // from group id
    fn delete_at(&self, _: &Vec<Hash>, _: usize) -> Rerr { Ok(()) } // from group id
    fn find_at(&self, _: &Hash, _: usize) -> Option<Box<TxPkg>> { None } // from group id
    fn clear_at(&self, _: usize) -> Rerr { Ok(()) } // by group id
    fn drain_filter_at(&self, _: &dyn Fn(&Box<TxPkg>)->bool, _: usize) 
        -> Rerr { Ok(()) }

    fn find(&self, _hx: &Hash) -> Option<Box<TxPkg>> { None }
    fn insert(&self, _: Box<TxPkg>) -> Rerr { Ok(()) }
    fn drain(&self, _: &Vec<Hash>) -> Ret<Vec<Box<TxPkg>>> { Ok(vec![]) }

    fn print(&self) -> String { s!("") }
}