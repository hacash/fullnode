


// BlockV1
combi_struct_with_parse!{ BlockV1, 
    (self, buf, {
        // intro
        let mut intro = BlockIntro::default();
        let mut seek = intro.parse(buf)?;
        let trslen = *intro.head.transaction_count;
        self.intro = intro;
        // body
        self.transactions.set_count(trslen.into());
        seek += self.transactions.parse(&buf[seek..])?;
        Ok(seek)
    }),
    // head meta
	intro : BlockIntro
	// trs body
	transactions : DynVecTransaction
}



/********************/



macro_rules! block_intro_fn_mount{
    ($fname:ident, $rty:ty) => (
        fn $fname(&self) -> &$rty {
            self.intro.$fname()
        }
    )
}


impl BlockRead for BlockV1 {

    fn hash(&self) -> Hash {
        self.intro.hash()
    }

    block_intro_fn_mount!{version, Uint1}
    block_intro_fn_mount!{height, BlockHeight}
    block_intro_fn_mount!{timestamp, Timestamp}
    block_intro_fn_mount!{difficulty, Uint4}
    block_intro_fn_mount!{nonce, Uint4}
    block_intro_fn_mount!{prevhash, Hash}
    block_intro_fn_mount!{mrklroot, Hash}
    block_intro_fn_mount!{transaction_count, Uint4}

    fn transaction_hash_list(&self, hash_with_fee: bool) -> Vec<Hash> {
        let mut list = vec![];
        // println!("self.transactions.list: {}", self.transactions.list().len());
        for t in self.transactions.list() {
            if hash_with_fee {
                list.push(t.hash_with_fee())
            }else{
                list.push(t.hash())
            }
        }
        list
    }

    fn transactions(&self) -> &Vec<Box<dyn Transaction>> {
        self.transactions.list()
    }

    fn coinbase_transaction(&self) ->  Ret<&dyn TransactionRead> {
        let txs = self.transactions();
        if txs.len() < 1 {
            return errf!("block must have coinbase tx")
        }
        let cbtx = &txs[0];
        if cbtx.ty() != TransactionCoinbase::TYPE {
            return errf!("block first tx must be coinbase")
        }
        Ok(cbtx.as_read())
    }
}

impl BlockExec for BlockV1 {
    fn execute(&self, ccnf: ctx::Chain, state: Box<dyn State>) -> Ret<Box<dyn State>> {
        // create env
        let mut env = ctx::Env{
            chain: ccnf,
            block: ctx::Block{
                height: self.height().uint(),
                hash: self.hash(),
                coinbase: Address::default(),
            },
            tx: ctx::Tx::default(),
        };
        // coinbase 
        let cbtx = self.coinbase_transaction()?;
        let base_addr = cbtx.main();
        env.block.coinbase = base_addr.clone();
        // create ctx
        let mut ctxobj = ctx::ContextInst::new(env, state, cbtx);
        let ctx = &mut ctxobj;
        let txs = self.transactions();
        let mut total_fee = Amount::zero();
        // exec each tx
        for tx in txs {
            ctx.env.tx = ctx::Tx::create(tx.as_read()); // set env
            ctx.txr = tx.as_read();
            tx.execute(ctx)?; // do exec
            total_fee = total_fee.add_mode_u64(&tx.fee_got())?; // add fee
        }
        // add fee
        if total_fee.is_positive() { // amt > 0
            operate::hac_add(ctx, &base_addr, &total_fee)?;
        }
        Ok(ctxobj.into_state())

    }
}




/********************/



impl Block for BlockV1 {

    fn as_read(&self) -> &dyn BlockRead { 
        self
    }

    fn update_mrklroot(&mut self) {
        let hash_with_fee = true;
        let hxlist = self.transaction_hash_list(hash_with_fee);
        let mrkl = calculate_mrklroot(&hxlist);
        self.set_mrklroot(mrkl);
    }

    fn set_mrklroot(&mut self, mkrt: Hash) {
        self.intro.head.mrklroot = mkrt;
    }

	fn set_nonce(&mut self, nonce: Uint4) {
        self.intro.meta.nonce = nonce;
	}

    fn replace_transaction(&mut self, i: usize, v: Box<dyn Transaction>) -> Rerr {
        self.transactions.replace(i, v)
    }

    fn push_transaction(&mut self, tx: Box<dyn Transaction>) -> Rerr {
        let ct = &mut self.intro.head.transaction_count;
        if ct.uint() + 1 == u32::MAX  {
            return errf!("transaction overflow")
        }
        *ct += 1;
        self.transactions.set_count(*ct);
        self.transactions.push(tx)
    }



    
}



/********************/


impl BlockV1 {

    pub const VERSION: u8 = 1;

    pub fn new() -> BlockV1 {
        let mut blk = <BlockV1 as Field>::new();
        blk.intro.head.version = Uint1::from(Self::VERSION);
        blk 
    }
}


