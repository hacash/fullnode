


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


impl BlockExec for BlockV1 {}

impl BlockRead for BlockV1 {}

impl Block for BlockV1 {}


impl BlockV1 {
    pub const VERSION: u8 = 1;
}
