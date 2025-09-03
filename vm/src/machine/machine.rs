use value::Value;



#[allow(dead_code)]
pub struct MachineBox {
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
            machine: Some(m)
        }
    }
}

impl VM for MachineBox {
    fn usable(&self) -> bool { true }
    fn call(&mut self, ctx: &mut dyn Context, sta: &mut dyn State, ty: u8, kd: u8, data: &[u8], param: Vec<u8>) -> Ret<Vec<u8>> {
        // gas &&  check balance
        let gas_limit = SpaceCap::new(ctx.env().block.height).max_gas_of_tx as i64;
        let mut gas = ctx.tx().size() as i64;
        let (feer, gasfee) = ctx.tx().fee_extend()?;
        if feer == 0 {
            return errf!("gas extend cannot empty on contract call")
        }
        let main = ctx.env().tx.main;
        protocol::operate::hac_check(ctx, &main, &gasfee)?;
        gas *= feer as i64;
        if gas > gas_limit {
            gas = gas_limit; // max 65535
        }
        let gas = &mut gas;
        // env
        let sta = &mut VMState::wrap(sta);
        let exenv = &mut ExecEnv{ ctx, sta, gas };
        let cty: CallTy = std_mem_transmute!(ty);
        match cty {
            CallTy::Main => {
                let cty = map_itr_err!(CodeType::parse(kd))?;
                self.machine.as_mut().unwrap().main_call(exenv, cty, data.to_vec())
            },
            CallTy::Abst => {
                let kid: AbstCall = std_mem_transmute!(kd);
                let cadr = ContractAddress::parse(data)?;
                self.machine.as_mut().unwrap().abst_call(exenv, kid, cadr, param)
            }
        }.map(|a|a.to_bytes())

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
        let Some(fnobj) = map_itr_err!(self.r.load_abst(env.sta, &contract_addr, syscty))? else {
            return Ok(Value::Nil) // not find call
        };
        map_itr_err!(self.do_call(env, CallMode::Abst, 
            fnobj.as_ref().clone(), Some(Value::bytes(param))))
    }

    fn do_call(&mut self, env: &mut ExecEnv, mode: CallMode, code: FnObj, param: Option<Value>) -> VmrtRes<Value> {
        self.frames.start_call(&mut self.r, env, mode, code, param)
    }

}




