

#[allow(dead_code)]
pub struct MachineBox {
    gas: i64,
    gas_price: i64,
    machine: Option<Machine>
} 

impl Drop for MachineBox {
    fn drop(&mut self) {
        // println!("\n---------------\n[MachineBox Drop] Reclaim resoure))\n---------------\n");
        match self.machine.take() {
            Some(m) => global_machine_manager().reclaim(m.r),
            _ => ()
        }
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

    fn check_gas(&mut self, ctx: &mut dyn Context) -> Rerr {        
        const L: i64 = i64::MIN;
        match self.gas {
            L     => self.init_gas(ctx),
            L..=0 => errf!("gas has run out"),
            _     => Ok(()) // gas > 0
        }
    }

    fn init_gas(&mut self, ctx: &mut dyn Context) -> Rerr {
        // init gas
        let spscp = &self.machine.as_mut().unwrap().r.space_cap;
        let gas_limit = spscp.max_gas_of_tx as i64;
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
        Ok(())
    }

    fn check_cost(&self, cty: CallMode, mut cost: i64) -> Ret<i64> {
        use CallMode::*;
        assert!(cost > 0, "gas cost error");
        // min use
        let gsext = &self.machine.as_ref().unwrap().r.gas_extra;
        let min_use = match cty {
            Main => gsext.main_call_min,
            Abst => gsext.abst_call_min,
            _ => unreachable!()
        };
        up_in_range!(cost, min_use, i64::MAX);
        Ok(cost)
    }


    fn spend_gas(&self, ctx: &mut dyn Context, cost: i64) -> Rerr {
        assert!(self.gas_price > 0, "gas price error");
        // do spend
        let cost_per = cost * (self.gas_price / GSCU as i64);
        assert!(cost_per > 0, "gas cost error");
        let cost_amt = Amount::unit238(cost_per as u64);
        let main = ctx.env().tx.main;
        protocol::operate::hac_sub(ctx, &main, &cost_amt)?;
        Ok(())
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
        ty: u8, kd: u8, data: &[u8], param: Box<dyn Any>
    ) -> Ret<(i64, Vec<u8>)> {
        use CallMode::*;
        // init gas & check balance
        self.check_gas(ctx)?;
        let gas = &mut self.gas;
        let gas_record = *gas;
        // env & do call
        let machine = self.machine.as_mut().unwrap();
        let not_in_calling = ! machine.is_in_calling();
        let sta = &mut VMState::wrap(sta);
        let exenv = &mut ExecEnv{ ctx, sta, gas };
        let cty: CallMode = std_mem_transmute!(ty);
        let resv = match cty {
            Main => {
                let cty = CodeType::parse(kd)?;
                machine.main_call(exenv, cty, data.to_vec())
            },
            Abst => {
                let kid: AbstCall = std_mem_transmute!(kd);
                let cadr = ContractAddress::parse(data)?;
                let Ok(param) = param.downcast::<Value>() else {
                    return errf!("argv type not match")
                };
                machine.abst_call(exenv, kid, cadr, *param)
            }
            _ => unreachable!()
        }.map(|a|a.raw())?;
        let gas_current = *gas;
        let mut cost = gas_record - gas_current;
        cost = self.check_cost(cty, cost)?;
        // spend gas, but in calling do not spend
        if not_in_calling {
            self.spend_gas(ctx, cost)?;
        }
        // ok
        Ok((cost, resv))
    }
}




/*********************************/





#[allow(dead_code)]
pub struct Machine {
    r: Resoure,
    frames: Vec<CallFrame>,
}



impl Machine {

    pub fn is_in_calling(&self) -> bool {
        ! self.frames.is_empty()
    }

    pub fn create(r: Resoure) -> Self {
        Self {
            r,
            frames: vec![],
        }
    }

    pub fn main_call(&mut self, env: &mut ExecEnv, ctype: CodeType, codes: Vec<u8>) -> Ret<Value> {
        let fnobj = FnObj{ ctype, codes, confs: 0, agvty: None};
        let v = self.do_call(env, CallMode::Main, fnobj, None, None)?;
        Ok(v)
    }

    pub fn abst_call(&mut self, env: &mut ExecEnv, cty: AbstCall, contract_addr: ContractAddress, param: Value) -> Ret<Value> {
        let adr = contract_addr.readable();
        let Some(fnobj) = self.r.load_abstfn(env.sta, &contract_addr, cty)? else {
            // return Ok(Value::Nil) // not find call
            return errf!("abst call {:?} not find in {}", cty, adr) // not find call
        };
        let fnobj = fnobj.as_ref().clone();
        let param =  Some(param);
        let rv = self.do_call(env, CallMode::Abst, fnobj, Some(contract_addr), param)?;
        if rv.check_true() {
            return errf!("call {}.{:?} return error code {}", adr, cty, rv.to_uint())
        }
        Ok(rv)
    }

    fn do_call(&mut self, env: &mut ExecEnv, mode: CallMode, code: FnObj, ctxadr: Option<ContractAddress>, param: Option<Value>) -> VmrtRes<Value> {
        self.frames.push(CallFrame::new()); // for reclaim
        let res = self.frames.last_mut().unwrap().start_call(&mut self.r, env, mode, code, ctxadr, param);
        self.frames.pop().unwrap().reclaim(&mut self.r); // do reclaim
        res
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
