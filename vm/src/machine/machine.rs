use value::Value;



#[allow(dead_code)]
pub struct MachineBox {
    gas: i64,
    gas_price: i64,
    pub machine: Option<Machine>
} 

impl Drop for MachineBox {
    fn drop(&mut self) {
        // println!("\n---------------\n[MachineBox Drop] Reclaim resoure))\n---------------\n");
        let m = self.machine.take().unwrap();
        MACHINE_MANAGER.reclaim(m.remove());
    }
}

impl MachineBox {
    
    pub fn new(m: Machine) -> Self {
        Self { 
            gas: i64::MIN, // init in first call
            gas_price: 0,
            machine: Some(m)
        }
    }

    fn check_gas(&mut self, ctx: &mut dyn Context) -> Ret<i64> {
        const L: i64 = i64::MIN;
        match self.gas {
            L     => self.init_gas(ctx),
            L..=0 => errf!("gas has run out"),
            g     => Ok(g) // gas > 0
        }
    }

    fn init_gas(&mut self, ctx: &mut dyn Context) -> Ret<i64> {
        // init gas
        let gas_limit = SpaceCap::new(ctx.env().block.height).max_gas_of_tx as i64;
        let (feer, gasfee) = ctx.tx().fee_extend()?;
        if feer == 0 {
            return errf!("gas extend cannot empty on contract call")
        }
        let main = ctx.env().tx.main;
        protocol::operate::hac_check(ctx, &main, &gasfee)?;
        let mut gas = ctx.tx().size() as i64 * feer as i64;
        up_in_range!(gas, 0, gas_limit);  // max 65535
        self.gas = gas;
        self.gas_price = Self::gas_price(ctx);
        Ok(gas)
    }


    fn spend_gas(&mut self, ctx: &mut dyn Context, gas_rem: i64) -> Ret<i64>{
        assert!(self.gas_price > 0, "gas price error");
        assert!(gas_rem >= 0, "gas use error");
        let cost = self.gas - gas_rem;
        assert!(cost > 0, "gas use error");
        // do spend
        let cost_per = cost * (self.gas_price / GSCU as i64);
        assert!(cost_per > 0, "gas cost error");
        let cost_amt = Amount::unit238(cost_per as u64);
        let main = ctx.env().tx.main;
        protocol::operate::hac_sub(ctx, &main, &cost_amt)?;
        // reset gas
        self.gas = gas_rem;
        Ok(cost)
    }

    fn gas_price(ctx: &dyn Context) -> i64 {
        let gs = ctx.tx().fee_purity() as i64;
        gs // calc by fee got
    }


}

impl VM for MachineBox {
    fn usable(&self) -> bool { true }
    fn call(&mut self, 
        ctx: &mut dyn Context, sta: &mut dyn State,
        ty: u8, kd: u8, data: &[u8], param: Vec<u8>
    ) -> Ret<(i64, Vec<u8>)> {
        // init gas & check balance
        let gas = &mut self.check_gas(ctx)?;
        let machine = self.machine.as_mut().unwrap();
        // env & do call
        let sta = &mut VMState::wrap(sta);
        let exenv = &mut ExecEnv{ ctx, sta, gas };
        let cty: CallTy = std_mem_transmute!(ty);
        let resv = match cty {
            CallTy::Main => {
                let cty = map_itr_err!(CodeType::parse(kd))?;
                machine.main_call(exenv, cty, data.to_vec())
            },
            CallTy::Abst => {
                let kid: AbstCall = std_mem_transmute!(kd);
                let cadr = ContractAddress::parse(data)?;
                machine.abst_call(exenv, kid, cadr, param)
            }
        }.map(|a|a.to_bytes())?;
        // spend gas
        let cost = self.spend_gas(ctx, *gas)?;
        // ok
        Ok((cost, resv))
    }
}




/*********************************/





#[allow(dead_code)]
pub struct Machine {
    r: Resoure,
    frames: CallFrame,
}



impl Machine {

    pub fn create(r: Resoure) -> Self {
        Self {
            r,
            frames: CallFrame::new(),
        }
    }

    pub fn remove(mut self) -> Resoure {
        self.frames.reclaim(&mut self.r);
        self.r
    }

    pub fn main_call(&mut self, env: &mut ExecEnv, ctype: CodeType, codes: Vec<u8>) -> Ret<Value> {
        let fnobj = FnObj{confs: 0, ctype, codes};
        map_itr_err!(self.do_call(env, CallMode::Main, fnobj, None))
    }

    pub fn abst_call(&mut self, env: &mut ExecEnv, syscty: AbstCall, contract_addr: ContractAddress, param: Vec<u8>) -> Ret<Value> {
        let Some(fnobj) = map_itr_err!(self.r.load_abstfn(env.sta, &contract_addr, syscty))? else {
            return Ok(Value::Nil) // not find call
        };
        map_itr_err!(self.do_call(env, CallMode::Abst, 
            fnobj.as_ref().clone(), Some(Value::bytes(param))))
    }

    fn do_call(&mut self, env: &mut ExecEnv, mode: CallMode, code: FnObj, param: Option<Value>) -> VmrtRes<Value> {
        self.frames.start_call(&mut self.r, env, mode, code, param)
    }


}


#[cfg(test)]
mod machine_test {

/*
    i64::MAX  = 9223372036854775807
    10000 HAC =   10000000000000000:236

    0.00000001 = 1:240 = 10000:236




*/


}
