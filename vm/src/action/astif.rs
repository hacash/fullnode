


action_define!{AstIf, 101, 
    ActLv::Ast, // level
    // burn 90 fee , check child burn 90
    self.cond.burn_90() || self.br_if.burn_90() || self.br_else.burn_90(), 
    [],
    {
        cond:    AstSelect
        br_if:   AstSelect
        br_else: AstSelect
    },
    (self, ctx, gas {
        let oldsta = ctx.state_fork();
        match self.cond.execute(ctx) {
            // if br
            Ok(..) => {
                ctx.state_merge(oldsta); // merge sub state
                self.br_if.execute(ctx)
            },
            // else br
            Err(..) => {
                ctx.state_replace(oldsta); // drop sub state
                self.br_else.execute(ctx)
            }
        }.map(|(_,b)|b)
    })
}


pub fn create_ast_if(cond: AstSelect, br_if: AstSelect, br_else: AstSelect) -> AstIf {
    AstIf {
        cond,
        br_if,
        br_else,
        ..AstIf::new()
    }
}

