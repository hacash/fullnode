

#[macro_export]
macro_rules! inst_state_define {
    ($class:ident ) => {

        pub struct $class {
            sta: ArcDynState,
        }

        impl $class {
            fn wrap(s: ArcDynState) -> Self {
                Self {
                    sta: s
                }
            }
        }


    };
}




/*
* test
*/
inst_state_define!{ TestSta834765495863457 }


