
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
        let from = env.tx.main; 
        let to = self.to.real(&env.tx.addrs)?;
        hac_transfer(env, ctx.state(), &from, &to, &self.hacash)
    })
}




