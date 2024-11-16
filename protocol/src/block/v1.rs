

// BlockV1
combi_struct_with_parse!{ BlockV1, 
    (self, buf, {
        // intro
        let mut intro = BlockIntro::default();
        let isk = intro.parse(buf)?;
        let trslen = intro.head.transaction_count.to_uint();
        self.intro = intro;
        // body
        self.transactions.set_count(trslen.into());
        self.transactions.parse(&buf[isk..])
    }),
    // head meta
	intro : BlockIntro
	// trs body
	transactions : DynVecTransaction
}


impl BlockExec for BlockV1 {
    fn execute(&self, ccnf: ctx::Chain, state: Box<dyn State>) -> Rerr {
        // create env
        let env = ctx::Env{
            chain: ccnf,
            block: ctx::Block{
                height: self.height().to_uint(),
                hash: self.hash_cache().clone(),
            },
            tx: ctx::Tx::default(),
        };
        // create context
        let mut ctxobj = ctx::ContextInst::new(env, state);
        // exec each tx
        for tx in self.transactions() {
            // set env
            ctxobj.env.tx.main = tx.main();
            ctxobj.env.tx.addrs = tx.addrs();
            // do exec
            tx.execute(&mut ctxobj)?;
        }


        todo!()
    }
}

impl BlockRead for BlockV1 {
    fn hash_cache(&self) -> &Hash { unimplemented!() }
}

impl Block for BlockV1 {}


impl BlockV1 {
    pub const VERSION: u8 = 1;
}
