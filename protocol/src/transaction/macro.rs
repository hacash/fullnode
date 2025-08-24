


macro_rules! transaction_define {
    ($class:ident, $tyid:expr) => (



field::combi_struct!{ $class,
    ty         : Uint1
    timestamp  : Timestamp
    addrlist   : AddrOrList
    fee        : Amount
    actions    : DynListActionW2
    signs      : SignListW2
    gas_max    : Uint1
    ano_mark   : Fixed1
}


impl TransactionRead for $class {

    fn hash(&self) -> Hash {
        self.hash_ex(vec![]) // no fee field
    }
    
    fn hash_with_fee(&self) -> Hash {
        self.hash_ex(self.fee.serialize()) // with fee
    }

    fn ty(&self) -> u8 {
        *self.ty
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
        self.actions().iter().any(|a|a.burn_90())
    }

    fn fee_pay(&self) -> Amount {
        self.fee().clone()
    }
    // fee_miner_received
    fn fee_got(&self) -> Amount {
        let mut gfee = self.fee().clone();
        if self.burn_90() && gfee.unit() > 1 {
            gfee = gfee.unit_sub(1).unwrap(); // burn 90
        }
        gfee
    }

    fn fee_extend(&self) -> Ret<(u16, Amount)> {
        let par = (*self.gas_max) as u16;
        let max = par * par;
        let fee = self.fee_got().dist_mul(max as u128)?;
        Ok((max, fee))
    }


    fn req_sign(&self) -> Ret<HashSet<Address>> {
        let addrs = &self.addrs();
        let mut adrsets = HashSet::from([self.main()]);
        for act in self.actions() {
            for ptr in act.req_sign() {
                let adr = ptr.real(addrs)?; 
                if adr.is_privakey() {
                    adrsets.insert(adr); // just PRIVAKEY
                }
            }
        }
        Ok(adrsets)
    }

    fn verify_signature(&self) -> Rerr {
        verify_tx_signature(self)
    }
    
	fn fee_purity(&self) -> u64 {
		let txsz = self.size() as u64;
		let fee238 = self.fee_got().to_238_u64().unwrap_or_default();
		fee238 / txsz
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


impl TxExec for $class {
    fn execute(&self, ctx: &mut dyn Context) -> Rerr {
        do_tx_execute(self, ctx)
    }
}


impl $class {
    pub const TYPE: u8 = $tyid;

    pub fn new_by(addr: Address, fee: Amount, ts: u64) -> $class {
        $class{
            ty: Uint1::from($tyid),
            timestamp: Timestamp::from(ts),
            addrlist: AddrOrList::from_addr(addr),
            fee: fee,
            actions: DynListActionW2::default(),
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
        let hx = sys::calculate_hash(stuff);
        Hash::must(&hx[..])
    }

    fn insert_sign(&mut self, signobj: Sign) -> Rerr {
        let plen = self.signs.length() as usize;
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




/*
* 
*/
fn do_tx_execute(tx: &dyn Transaction, ctx: &mut dyn Context) -> Rerr {
    const TXTY1: u8 = TransactionType1::TYPE;
    const _TXTY2: u8 = TransactionType2::TYPE;
    const _TXTY3: u8 = TransactionType3::TYPE;
    let env = ctx.env();
    let blkhei = env.block.height;
    let not_fast_sync = !env.chain.fast_sync;
    let hx = tx.hash();
    let main = tx.main();
    let fee = tx.fee();
    let mut state = CoreState::wrap(ctx.state());
    // may fast_sync
    if not_fast_sync {
        if tx.action_count().uint() == 0 {
            return errf!("tx actions cannot empty.")
        }
        // main check
        if ! main.is_privakey() {
            return errf!("tx fee address version must be PRIVAKEY type.")
        }
        let mty = tx.ty();
        // check BlockHeight more than 20w trs.Fee.Size() must less than 6 bytes.
        if blkhei > 20_0000 && fee.size() > 2+4 {
            return errf!("tx fee size cannot be more than 6 bytes when block height abover 200,000")
        }
        if blkhei > 33033 && mty <= TXTY1 { // last is 33019
            return errf!("Type 1 transactions have been deprecated after height 33,033")
        }
        // check tx exist
        if let Some(exhei) = state.tx_exist(&hx) { // have tx !!!
            // handle hacash block chain bug start
            let bugtx = Hash::from_hex(b"f22deb27dd2893397c2bc203ddc9bc9034e455fe630d8ee310e8b5ecc6dc5628");
            if *exhei != 63448 || hx != bugtx {
                return errf!("tx {} already exist in height {}", hx, *exhei)
            }
            // pass the BUG
        }
    }
    // set tx exist mark
    state.tx_exist_set(&hx, &BlockHeight::from(blkhei));
    /*
    if mty <= TXTY3 {
        if self.ano_mark[0] != 0 {
            return errf!("tx extend data error")
        }
    }
    if mty <= TXTY2 {
        if self.gas_max.value() != 0 {
            return errf!("tx extend data error")
        }
    }
    */
    // reset the vm
    ctx.vm_replace(VMNil::empty());
    // execute actions
    for action in tx.actions() {
        ctx.depth_set(-1); // set depth
        action.execute(ctx)?;
    }
    // spend fee
    operate::hac_sub(ctx, &main, fee)?;
    // ok finish
    Ok(())
}

