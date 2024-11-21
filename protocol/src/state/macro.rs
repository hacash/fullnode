

macro_rules! get_key {
    ($idx:expr, $key:expr) => {{
        let prex = ($idx as u16).to_be_bytes();
        vec![prex.to_vec(), $key.serialize()].concat()
    }}
}

macro_rules! get_or_none {
    ($self:ident, $key:ident, $idx:expr, $vty:ty) => {{
        let k = get_key!($idx, $key);
        $self.sta.get(k).map(|v|<$vty>::must(&v))
    }}
}

macro_rules! get_or_default {
    ($self:ident, $idx:expr, $vty:ty) => {{
        let k = ($idx as u16).to_be_bytes();
        let mut v = <$vty>::default();
        if let Some(bts) = $self.sta.get(k.to_vec()) {
            v.parse(&bts).unwrap(); // must
        }
        v
    }}
}


#[macro_export]
macro_rules! inst_state_define {
    ($class:ident, $( $idx:expr, $kn:ident, $kty:ty : $vty:ty)+ ) => {

        concat_idents!{ classread = $class, Read {
            pub struct classread {
                sta: Arc<dyn State>,
            }

            impl classread {
                pub fn wrap(s: Arc<dyn State>) -> Self {
                    Self {
                        sta: s,
                    }
                }

                $(
                    pub fn $kn(&self, key: &$kty) -> Option<$vty> {
                        get_or_none!(self, key, $idx, $vty)
                    }

                    concat_idents!{ get_stat = get_, $kn, {
                    pub fn get_stat(&self) -> $vty {
                        get_or_default!(self, $idx, $vty)
                    }}
                    }
                )+
            }

        }}

        /**********************8 */

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

                pub fn $kn(&self, key: &$kty) -> Option<$vty> {
                    get_or_none!(self, key, $idx, $vty)
                }

                concat_idents!{ fn_set = $kn, _set {
                    pub fn fn_set (&mut self, key: &$kty, v: &$vty) {
                        let k = get_key!($idx, key);
                        self.sta.set(k, v.serialize())
                    }
                }}
                concat_idents!{ fn_del = $kn, _del {
                    pub fn fn_del(&mut self, key: &$kty) {
                        let k = get_key!($idx, key);
                        self.sta.del(k)
                    }
                }}


                concat_idents!{ get_stat = get_, $kn, {
                pub fn get_stat(&self) -> $vty {
                    get_or_default!(self, $idx, $vty)
                }
                }}

                concat_idents!{ set_stat = set_, $kn {
                    pub fn set_stat(&mut self, v: &$vty) {
                        let k = ($idx as u16).to_be_bytes().to_vec();
                        self.sta.set(k, v.serialize())
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


