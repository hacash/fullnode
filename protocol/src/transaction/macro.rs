




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
    
            fn hash(&self) -> Hash {
                self.hash_ex(vec![]) // no fee field
            }
            
            fn hash_with_fee(&self) -> Hash {
                self.hash_ex(self.fee.serialize()) // with fee
            }
        
            fn ty(&self) -> u8 {
                self.ty.uint()
            }
        
            fn main(&self) -> Address {
                self.addrs()[0] // must
            }
            
            fn addrs(&self) -> Vec<Address> { 
                self.addrlist.list() // must
            }

            fn fee(&self) -> &Amount {
                &self.fee
            }
        
            fn timestamp(&self) -> &Timestamp {
                &self.timestamp
            }
        
            fn action_count(&self) -> &Uint2 {
                self.actions.count()
            }
            fn actions(&self) -> &Vec<Box<dyn Action>> {
                self.actions.list()
            }
        
            fn signs(&self) -> &Vec<Sign> {
                self.signs.list()
            }
            
            // burn_90_percent_fee
            fn burn_90(&self) -> bool {
                for act in self.actions() {
                    if act.burn_90() {
                        return true // burn
                    }
                }
                false // not
            }
        
            // fee_miner_received
            fn fee_got(&self) -> Amount {
                let mut gfee = self.fee().clone();
                if self.burn_90() && gfee.unit() > 1 {
                    gfee.unit_sub(1); // burn 90
                }
                gfee
            } 
        
            fn req_sign(&self) -> Ret<HashSet<Address>> {
                let adary = self.addrs();
                let mut addrs = HashSet::from([self.main()]);
                for act in self.actions() {
                    for adr in act.req_sign(&adary)? {
                        if adr.version() == Address::PRIVAKEY {
                            addrs.insert(adr); // just PRIVAKEY
                        }
                    }
                }
                Ok(addrs)
            }
        
            fn verify_signature(&self) -> Rerr {
                verify_tx_signature(self)
            }
            
        }
        
        impl Transaction for $class {
        
            fn as_read(&self) -> &dyn TransactionRead {
                self
            }
        
            fn set_fee(&mut self, fee: Amount) {
                self.fee = fee;
            }
        
            fn fill_sign(&mut self, acc: &Account) -> Ret<Sign> {
                let mut fhx = self.hash();
                if acc.address() == self.main().as_bytes() {
                    fhx = self.hash_with_fee();
                }
                // do sign
                let apbk = acc.public_key().serialize_compressed();
                let signobj = Sign{
                    publickey: Fixed33::from( apbk ),
                    signature: Fixed64::from( acc.do_sign(&fhx) ),
                };
                // insert
                self.insert_sign(signobj.clone())?;
                Ok(signobj)
            }
        
            fn push_sign(&mut self, signobj: Sign) -> Rerr {
                self.insert_sign(signobj)
            }
        
            fn push_action(&mut self, act: Box<dyn Action>) -> Rerr {
                self.actions.push(act)
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

            fn hash_ex(&self, adfe: Vec<u8>) -> Hash {
                let mut stuff = vec![
                    self.ty.serialize(),
                    self.timestamp.serialize(),
                    self.addrlist.serialize(),
                    adfe, /* self.fee.serialize()*/
                    self.actions.serialize()
                ].concat();
                // ignore signs data
                if $tyid >= TransactionType3::TYPE {
                    stuff.append(&mut self.gas_max.serialize());
                    stuff.append(&mut self.ano_mark.serialize());
                }
                let hx = x16rs::calculate_hash(stuff);
                Hash::must(&hx[..])
            }
        
            fn insert_sign(&mut self, signobj: Sign) -> Rerr {
                let plen = self.signs.count().uint() as usize;
                if plen >= u16::MAX as usize - 1 {
                    return errf!("sign object too much")
                }
                let curaddr = Address::from(Account::get_address_by_public_key(*signobj.publickey));
                // insert
                let apbk = signobj.publickey.as_ref();
                let mut istid = usize::MAX;
                let sglist = self.signs.list();
                for i in 0..plen {
                    let pbk = sglist[i].publickey.as_bytes();
                    if apbk == pbk {
                        istid = i;
                        break
                    }
                }
                // append
                if istid == usize::MAX {
                    self.signs.push(signobj)?;
                }else{
                    // replace
                    self.signs.as_mut()[istid] = signobj;
                }
                if let Ok(yes) = verify_target_signature(&curaddr, self) {
                    if yes {
                        return Ok(())
                    }
                }
                // verify error
                errf!("address {} verify signature failed", curaddr.readable())
            }
        










        }


    )
}
