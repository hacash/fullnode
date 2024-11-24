
/*
* 
*/
action_define!{ DiamondSingleTransfer, 5, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [], // need sign
    {
        diamond   : DiamondName  
        to        : AddrOrPtr 
    },
    (self, ctx, _gas {
        let from  = ctx.env().tx.main;
        let to    = ctx.addr(&self.to)?;
        let dlist = DiamondNameListMax200::one(self.diamond);
        do_diamonds_transfer(&dlist, &from, &to, ctx)  
    })
}

/*
* 
*/
action_define!{ DiamondFromToTransfer, 6, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [self.from], // need sign
    {
        from      : AddrOrPtr
        to        : AddrOrPtr
        diamonds  : DiamondNameListMax200
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.addr(&self.to)?;
        do_diamonds_transfer(&self.diamonds, &from, &to, ctx) 
    })
}


/*
* 
*/
action_define!{ DiamondToTransfer, 7, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [], // need sign
    {
        to        : AddrOrPtr
        diamonds  : DiamondNameListMax200
    },
    (self, ctx, _gas {
        let from = ctx.env().tx.main;
        let to   = ctx.addr(&self.to)?;
        do_diamonds_transfer(&self.diamonds, &from, &to, ctx) 
    })
}


/*
* 
*/
action_define!{ DiamondFromTransfer, 8, 
    ActLv::MAIN_CALL, // level
    false, // burn 90 fee
    [self.from], // need sign
    {
        from      : AddrOrPtr
        diamonds  : DiamondNameListMax200 
    },
    (self, ctx, _gas {
        let from = ctx.addr(&self.from)?;
        let to   = ctx.env().tx.main;
        do_diamonds_transfer(&self.diamonds, &from, &to, ctx) 
    })
}


/**************************/


fn do_diamonds_transfer(diamonds: &DiamondNameListMax200, from: &Address, to: &Address, ctx: &mut dyn Context) -> Ret<Vec<u8>> {
    // check
    let dianum = diamonds.check()?;
    let isdf = ctx.env().chain.diamond_form;
    //transfer
    let mut state = CoreState::wrap(ctx.state());
    for dianame in diamonds.list() {
        hacd_move_one_diamond(&mut state, from, to, &dianame)?; // move one
    }
    if isdf {
        diamond_owned_move(&mut state, from, to, diamonds)?;
    }
    // transfer
    hacd_transfer(&mut state, from, to, &DiamondNumber::from(dianum as u32), &diamonds)
}

