


action_define!{AstSelect, 100, 
    ActLv::Ast, // level
    // burn 90 fee, check any sub child action
    self.actions.list().iter().any(|a|a.burn_90()),
    [],
    {
        exe_min: Uint1
        exe_max: Uint1
        actions: DynListActionW1
    },
    (self, ctx, gas {
        let slt_min = *self.exe_min as usize;
        let slt_max = *self.exe_max as usize;
        let slt_num = self.actions.length();
        // check number
        if slt_min > slt_max {
            return errf!("action ast select max cannot less than min")
        }
        if slt_max > slt_num {
            return errf!("action ast select max cannot more than list num")
        }
        if slt_num > 200 {
            return errf!("action ast select num cannot more than 200")
        }
        // execute
        let mut ok = 0;
        let mut rv = vec![];
        for act in self.actions.list() {
            if ok >= slt_max {
                break // ok full
            }
            // try execute
            let oldsta = ctx.state_fork();
            if let Ok((g, r)) = act.execute(ctx) {
                gas += g;
                rv = r;
                ok += 1;
                ctx.state_merge(oldsta); // merge sub state
            } else {
                ctx.state_replace(oldsta); // drop sub state
            }
        }
        // check at least
        if ok < slt_min {
            return errf!("action ast select must succeed at least {} but only {}", slt_min, ok)
        }
        // ok
        Ok(rv)
    })
}


pub fn create_ast_select(min: u8, max: u8, acts: Vec<Box<dyn Action>>) -> AstSelect {
    AstSelect {
        exe_min: Uint1::from(min),
        exe_max: Uint1::from(max),
        actions: DynListActionW1::from_list(acts).unwrap(),
        ..AstSelect::new()
    }
}




