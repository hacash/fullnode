
impl CallFrame {


    pub fn start_call(&mut self, r: &mut Resoure, env: &mut ExecEnv, mode: CallMode, code: FnObj, param: Option<Value>) -> VmrtRes<Value> {
        self.contract_count = r.contracts.len();
        use CallExit::*;
        use CallMode::*;
        
        let mut curr_frame = self.increase(r)?;
        curr_frame.depth = env.ctx.depth().to_isize(); // set depth 0 or 1
        curr_frame.prepare(mode, code, param)?;
        
        loop {
            // execute
            let exit = curr_frame.execute(r, env)?; // call frame
            // if finish
            let retv = match exit {
                Return | Throw => curr_frame.pop_value()?,
                _ => Value::Nil,
            };
            // throw error 
            if let Abort | Throw = exit {
                curr_frame.reclaim(r); // reclaim resource
                return itr_err_fmt!(ThrowAbort, "vm end with error: {}", retv)
            }
            if let Finish | Return = exit {
                curr_frame.reclaim(r); // reclaim resource
                if let Some(mut prev_frame) = self.pop() {
                    prev_frame.push_value(retv)?;
                    curr_frame = prev_frame;
                    curr_frame.pc += 1; // exec next instruction
                    continue // prev frame do execute
                }else{
                    return Ok(retv) // all call finish
                }
            }
            // if call function
            let Call(fnptr) = exit else { unreachable!() };
            // load user func
            let (srcadr, fnobj) = r.load_must_call(env.sta, fnptr.clone(), 
                &curr_frame.ctxbase, &curr_frame.curcall)?;
            let fnobj = fnobj.as_ref().clone();
            let is_public = fnobj.conf(FnConf::IsPublic);
            // check gas
            self.check_load_new_contract_gas(r, env)?;
            // if call code
            if let Code = fnptr.mode {
                curr_frame.prepare(Code, fnobj, None)?; // no param
                continue // do execute
            }
            // call next frame
            let param = Some(curr_frame.pop_value()?);
            self.push(curr_frame);
            let mut next_frame = self.increase(r)?;
            next_frame.prepare(fnptr.mode, fnobj, param)?;
            // mode setup
            match fnptr.mode {
                Library | Static => {
                    next_frame.curcall = srcadr.unwrap(); // setup src addr
                },
                Location => {},
                External => {
                    let CallTarget::Addr(adr) = fnptr.target else {
                        unreachable!()
                    };
                    if ! is_public {
                        next_frame.reclaim(r); // reclaim resource
                        return itr_err_fmt!(CallNotPublic, "contract {} func sign {}", adr.readable(), fnptr.fnsign.hex())
                    }
                    next_frame.ctxbase = adr.clone(); // setup dst & src addr
                    next_frame.curcall = adr;
                }
                _ => unreachable!(),
            }
            // do execute
            curr_frame = next_frame;
            continue
        }

        
    
    }



    fn check_load_new_contract_gas(&mut self, r: &mut Resoure, env: &mut ExecEnv) -> VmrtErr {
        let ctlnum = &mut self.contract_count;
        // check gas
        let ctln = r.contracts.len();
        match ctln - *ctlnum {
            0 => {},
            1 => {
                // check and sub gas
                *env.gas -= r.gas_extra.load_one_new_contract;
                if *env.gas < 0 {
                    return itr_err_code!(OutOfGas)
                }
                // update count
                *ctlnum = ctln;
            },
            _ => unreachable!() // just load one or zero
        };
        Ok(())
    }
    

}
