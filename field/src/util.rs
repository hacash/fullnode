


#[macro_export]
macro_rules! impl_field_only_new {
    ($class:ident) => {
        impl Field for $class {
            fn new() -> Self {
                Self::default()
            }
        }
    };
}



