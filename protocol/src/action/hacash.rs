
/*
* simple hac to
*/
action_define!{ HacToTransfer, 1, 
    ActLv::MAINCALL, // level
    21 + 11, // gas = 32
    false, // burn 90 fee
    {
        to : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let env = ctx.env();
        let _from = env.tx.main; 
        let _to = self.to.real(&env.tx.addrs)?;
        // hac_transfer(ctx, state, &from, &to, &self.hacash)
        errf!("")
    })
}




