
impl CallFrame {

    pub fn start_call(&mut self, r: &mut Resoure, env: &mut ExecEnv, mode: CallMode, code: FnObj, param: Option<Value>) -> VmrtRes<Value> {
        use CallExit::*;
        use CallMode::*;
        if let Main | Abst = mode {} else {
            never!()
        }
        // to spend gas
        self.contract_count = r.contracts.len();
        let mut curr_frame = self.increase(r)?;
        curr_frame.depth = match mode { // set depth 0 or 1
            Main => 0,
            Abst => 1,
            _ => never!(),
        };
        // compile irnode and push func argv ...
        curr_frame.prepare(mode, code, param)?;
        // exec codes
        loop {
            let exit = curr_frame.execute(r, env)?; // call frame
            match exit {
                // call end
                Abort | Throw | Finish | Return => {
                    let retv = match exit {
                        Return | Throw => curr_frame.pop_value()?,
                        _ => Value::Nil,
                    };
                    curr_frame.reclaim(r); // reclaim resource
                    match exit {
                        Abort | Throw => return itr_err_fmt!(ThrowAbort, "vm end with error: {}", retv),
                        Finish | Return => {
                            match self.pop() {
                                Some(mut prev) => {
                                    prev.push_value(retv)?; // push func call result
                                    curr_frame = prev;
                                    // curr_frame.pc += 1; // exec next instruction
                                    continue // prev frame do execute
                                }
                                _ => return Ok(retv) // all call finish
                            }
                        }
                        _ => unreachable!()
                    }
                }
                // call next
                Call(fnptr) => {
                    let adrlist: Option<Vec<_>> = match curr_frame.mode {
                        Main => Some(env.ctx.tx().addrs().iter().map(|a|ContractAddress::new(*a)).collect()),
                        _ => None,
                    };
                    let (chgsrcadr, fnobj) = r.load_must_call(env.sta, fnptr.clone(), 
                        &curr_frame.ctxadr, &curr_frame.curadr, adrlist)?;
                    let fnobj = fnobj.as_ref().clone();
                    let is_public = fnobj.check_conf(FnConf::IsPublic);
                    // check gas
                    self.check_load_new_contract_and_gas(r, env)?;
                    // if call code
                    if let CodeCopy = fnptr.mode {
                        curr_frame.prepare(CodeCopy, fnobj, None)?; // no param
                        continue // do execute
                    }
                    // call next frame
                    let param = Some(curr_frame.pop_value()?);
                    self.push(curr_frame);
                    let mut next_frame = self.increase(r)?;
                    next_frame.prepare(fnptr.mode, fnobj, param)?;
                    let cadr = chgsrcadr.unwrap();
                    match fnptr.mode {
                        Location => {}
                        Library | Static => {
                            next_frame.curadr = cadr;
                        }
                        External => {
                            if ! is_public {
                                next_frame.reclaim(r); // reclaim resource
                                return itr_err_fmt!(CallNotPublic, "contract {} func sign {}", cadr.readable(), fnptr.fnsign.hex())
                            }
                            next_frame.ctxadr = cadr.clone(); 
                            next_frame.curadr = cadr; 
                        }
                        _ => unreachable!()

                    }
                }
            }
            unreachable!()
        }
    }



    pub fn _start_call_old(&mut self, r: &mut Resoure, env: &mut ExecEnv, mode: CallMode, code: FnObj, param: Option<Value>) -> VmrtRes<Value> {
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
                &curr_frame.ctxadr, &curr_frame.curadr, None)?;
            let fnobj = fnobj.as_ref().clone();
            let is_public = fnobj.check_conf(FnConf::IsPublic);
            // check gas
            self.check_load_new_contract_and_gas(r, env)?;
            // if call code
            if let CodeCopy = fnptr.mode {
                curr_frame.prepare(CodeCopy, fnobj, None)?; // no param
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
                    next_frame.curadr = srcadr.unwrap(); // setup src addr
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
                    next_frame.ctxadr = adr.clone(); // setup dst & src addr
                    next_frame.curadr = adr;
                }
                _ => unreachable!(),
            }
            // do execute
            curr_frame = next_frame;
            continue
        }

        
    
    }



    fn check_load_new_contract_and_gas(&mut self, r: &mut Resoure, env: &mut ExecEnv) -> VmrtErr {
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
