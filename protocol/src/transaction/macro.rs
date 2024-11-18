




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
            fn execute(&self, _ctxobj: &mut dyn Context) -> Rerr {
                

                todo!()
            }
        }


        impl TransactionRead for $class {
            fn ty(&self) -> u8 {
                self.ty.to_uint()
            }
        }

        impl Transaction for $class {
            fn as_read(&self) -> &dyn TransactionRead {
                self
            }
        }


        impl $class {
            pub const TYPE: u8 = $tyid;

            pub fn new_by(addr: Address, fee: Amount) -> $class {
                $class{
                    ty: Uint1::from($tyid),
                    timestamp: Timestamp::from(curtimes()),
                    addrlist: AddrOrList::from_addr(addr),
                    fee: fee,
                    actions: DynListAction::default(),
                    signs: SignListW2::default(),
                    gas_max : Uint1::default(),
                    ano_mark: Fixed1::default(),
                }
            }

        }


    )
}
