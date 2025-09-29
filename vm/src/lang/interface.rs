
pub trait AST {
    fn as_any(&self) -> &dyn Any { unimplemented!() }
    fn is_null(&self) -> bool { false }
    fn expression(&self) -> bool { false }
}

