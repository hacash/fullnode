use std::any::Any;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntentId(pub usize);

#[derive(Clone)]
pub struct HandleItem(Rc<dyn Any>);

impl HandleItem {
    pub fn new<T: Any>(value: T) -> Self {
        Self(Rc::new(value))
    }

    pub fn is<T: Any>(&self) -> bool {
        self.0.is::<T>()
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }

    pub fn downcast<T: Any>(self) -> std::result::Result<Rc<T>, Self> {
        self.0.downcast::<T>().map_err(Self)
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for HandleItem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Handle(..)")
    }
}

impl PartialEq for HandleItem {
    fn eq(&self, other: &Self) -> bool {
        // Intentionally pointer identity. Handles are runtime references, and VM semantic value
        // equality does not treat them as comparable content values.
        self.ptr_eq(other)
    }
}

impl Eq for HandleItem {}

impl Value {
    pub fn handle<T: Any>(value: T) -> Self {
        Value::Handle(HandleItem::new(value))
    }

    pub fn match_handle(&self) -> Option<&HandleItem> {
        match self {
            Value::Handle(handle) => Some(handle),
            _ => None,
        }
    }
}
