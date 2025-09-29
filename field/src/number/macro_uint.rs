

macro_rules! from_uint {
    ($class:ident, $vn:ident, $vt:ty, $tt:ty) => (
        impl From<$tt> for $class {
            fn from(item: $tt) -> Self {
                $class { $vn: item as $vt }
            }
        }
    )
}


macro_rules! from_uint_ary {
    ($class:ident, $vn:ident, $vt:ty, $( $tt:ty ),+) => (
        $(
            from_uint!{$class, $vn, $vt, $tt}
        )+
    )
}

macro_rules! from_uint_all {
    ($class:ident, $vn:ident, $vt:ty) => (
        from_uint_ary!{$class, $vn, $vt, 
            i8, u8, i16, u16, i32, u32, i64, u64, i128, u128
        }
    )
}

