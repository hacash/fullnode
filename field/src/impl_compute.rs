

macro_rules! compute_opt_impl {
    ($class:ident, $vn: ident, $op: ident, $opt: ident ) => (   
        impl $op for $class {
            type Output = Self;
            #[inline]
            fn $opt(self, other: Self) -> Self {
                Self {$vn: self.$vn.$opt(other.$vn)}
            }
        }
    )
}


macro_rules! compute_other_impl {
    ($class:ident, $vn: ident, $op: ident, $opt: ident, $vty:ty, $eqty:ty) => (   
        impl $op<$eqty> for $class {
            type Output = Self;
            #[inline]
            fn $opt(self, other: $eqty) -> Self {
                Self {$vn: self.$vn.$opt(other as $vty)}
            }
        }
    )
}


macro_rules! compute_assign_impl {
    ($class:ident, $vn:ident, $op:ident, $opt: ident) => (
        concat_idents!{opa = $op, Assign {
            impl opa for $class {
                concat_idents!{opa2 = $opt, _assign {
                    #[inline]
                    fn opa2(&mut self, other: Self) {
                        self.$vn.opa2(other.$vn);
                    }
                }}
            }   
        }}

    )
}


macro_rules! compute_assign_other_impl {
    ($class:ident, $vn:ident, $op:ident, $opt:ident, $vty:ty, $eqty:ty) => (
        concat_idents!{opa = $op, Assign {
            impl opa<$eqty> for $class {
                concat_idents!{opa2 = $opt, _assign {
                    #[inline]
                    fn opa2(&mut self, other: $eqty) {
                        self.$vn.opa2(other as $vty);
                    }
                }}
            }
        }}
    )
}


macro_rules! compute_other_list_impl {
    ($class:ident, $vn:ident, $op:ident, $opt:ident, $vty:ty, $( $eqtys:ty ),+) => (   
        $(
            compute_other_impl!{$class, $vn, $op, $opt, $vty, $eqtys}
        )+
    )
}


macro_rules! compute_other_all_impl {
    ($class:ident, $vn:ident, $op:ident, $opt:ident, $vty:ty) => ( 
        compute_other_list_impl!{$class, $vn, $op, $opt, $vty, 
            i8, u8, i16, u16, i32, u32, i64, u64
        } 
    )
}


macro_rules! compute_assign_other_list_impl {
    ($class:ident, $vn:ident, $op:ident, $opt:ident, $vty:ty, $( $eqtys:ty ),+) => (   
        $(
            compute_assign_other_impl!{$class, $vn, $op, $opt, $vty, $eqtys}
        )+
    )
}


macro_rules! compute_assign_other_all_impl {
    ($class:ident, $vn:ident, $op:ident, $opt:ident, $vty:ty) => ( 
        compute_assign_other_list_impl!{$class, $vn, $op, $opt, $vty, 
            i8, u8, i16, u16, i32, u32, i64, u64
        } 
    )
}


macro_rules! compute_one_impl {
    ($class:ident, $vn:ident, $op:ident, $opt:ident, $vty:ty) => ( 
        compute_opt_impl!{$class, $vn, $op, $opt}
        compute_other_all_impl!{$class, $vn, $op, $opt, $vty}
        compute_assign_impl!{$class, $vn, $op, $opt}
        compute_assign_other_all_impl!{$class, $vn, $op, $opt, $vty}
    )
}


macro_rules! compute_impl {
    ($class:ident, $vn:ident, $vty:ty) => ( 
        compute_one_impl!{$class,$vn, Add, add, $vty}
        compute_one_impl!{$class,$vn, Sub, sub, $vty}
        compute_one_impl!{$class,$vn, Mul, mul, $vty}
        compute_one_impl!{$class,$vn, Div, div, $vty}
    )
}