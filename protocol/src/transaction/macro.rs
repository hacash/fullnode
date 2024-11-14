




macro_rules! transaction_define {
    ($class:ident, $tyid:expr) => (

        field::combi_struct!{ $class,
            ty         : Uint1
            timestamp  : Timestamp
            addrlist   : AddrOrList
            fee        : Amount
            actions    : DynListAction
            signs      : SignListW2
            gas_max    : Uint1
            ano_mark   : Fixed1
        }

        impl TxExec for $class {
            fn execute(&self, _: u64) -> Rerr {
                errf!("")
            }
        }


        impl TransactionRead for $class {
            fn ty(&self) -> u8 {
                self.ty.to_u8()
            }
        }

        impl Transaction for $class {
            fn as_read(&self) -> &dyn TransactionRead {
                self
            }
        }


        impl $class {
            pub const TYPE: u8 = $tyid;
        }


    )
}
