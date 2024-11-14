

#[macro_export]
macro_rules! inst_state_define {
    ($class:ident, $( $idx:expr, $kn:ident, $kty:ty : $vty:ty)+ ) => {

        pub struct $class<'a> {
            sta: &'a mut dyn State,
        }

        impl<'a> $class<'a> {
            pub fn wrap(s: &'a mut dyn State) -> Self {
                Self {
                    sta: s,
                }
            }

            $(

                pub fn $kn(&self, _k: &$kty) -> Ret<Option<&$vty>> {
                    errf!("")
                }

                concat_idents!{ set_fn = $kn, _set {
                    pub fn set_fn(&mut self, _k: &$kty, _v: &$vty) -> Rerr {
                        errf!("")
                    }
                }}
                concat_idents!{ del_fn = $kn, _del {
                    pub fn del_fn(&mut self, _k: &$kty) -> Rerr {
                        errf!("")
                    }
                }}


                concat_idents!{ get_stat = get_, $kn, {
                pub fn get_stat(&self) -> Ret<&$vty> {
                    errf!("")
                }
                }}

                concat_idents!{ set_stat = set_, $kn {
                    pub fn set_stat(&mut self, _v: &$vty) -> Rerr {
                        errf!("")
                    }
                }}

            )+




        }



    };
}




/*
* test
*/
inst_state_define!{ TestSta834765495863457,
    1, balance, Address : Uint8
}


