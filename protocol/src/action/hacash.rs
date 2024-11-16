
/*
* simple hac to
*/
action_define!{ HacToTransfer, 1, 
    ActLv::MAINCALL, // level
    false, // burn 90 fee
    {
        to : AddrOrPtr
        hacash : Amount
    },
    (self, ctx, _gas {
        let env = ctx.env();
        let from = env.tx.main; 
        let to = self.to.real(&env.tx.addrs)?;
        hac_transfer(ctx, &from, &to, &self.hacash)
    })
}




