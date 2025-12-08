
impl CallFrame {

    pub fn start_call(&mut self, r: &mut Resoure, env: &mut ExecEnv, mode: CallMode, code: FnObj, 
        entry_addr: ContractAddress, 
        libs: Option<Vec<ContractAddress>>, 
        param: Option<Value>
    ) -> VmrtRes<Value> {
        use CallExit::*;
        use CallMode::*;
        // to spend gas
        self.contract_count = r.contracts.len();
        let mut curr_frame = self.increase(r)?;
        curr_frame.depth = match mode { // set depth 0 or 1
            Main => 0,
            Abst => 1,
            _ => never!(),
        };
        curr_frame.ctxadr = entry_addr.clone();
        curr_frame.curadr = entry_addr;
        // compile irnode and push func argv ...
        curr_frame.prepare(mode, code, param)?;
        // exec codes
        loop {
            let exit = curr_frame.execute(r, env)?; // call frame
            match exit {
                // end func
                Abort | Throw | Finish | Return => {
                    let mut retv = match exit {
                        Return | Throw => curr_frame.pop_value()?,
                        _ => Value::Nil,
                    };
                    curr_frame.check_output_type(&mut retv)?;
                    curr_frame.reclaim(r); // reclaim resource
                    match exit {
                        Abort | Throw => return itr_err_fmt!(ThrowAbort, "VM return error: {}", retv),
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
                // next call
                Call(fnptr) => {
                    let ctxadr = &curr_frame.ctxadr;
                    let curadr = &curr_frame.curadr;
                    let (chgsrcadr, fnobj) = r.load_must_call(env.sta, fnptr.clone(), 
                        ctxadr, curadr, &libs)?;
                    let fnobj = fnobj.as_ref().clone();
                    let fn_is_public = fnobj.check_conf(FnConf::Public);
                    // check gas
                    self.check_load_new_contract_and_gas(r, env)?;
                    // if call code
                    if let CodeCopy = fnptr.mode {
                        // println!("CodeCopy() ctxadr={}, curadr={}", ctxadr.prefix(7), curadr.prefix(7));
                        curr_frame.prepare(CodeCopy, fnobj, None)?; // no param
                        continue // do execute
                    }
                    // call next frame                    
                    // println!("{:?}() ctxadr={}, curadr={}", fnptr.mode, ctxadr.prefix(7), curadr.prefix(7));
                    let param = Some(curr_frame.pop_value()?);
                    self.push(curr_frame);
                    let next_frame = self.increase(r)?;
                    curr_frame = next_frame;
                    curr_frame.prepare(fnptr.mode, fnobj, param)?;
                    match fnptr.mode {
                        Inner | Library | Static => {
                            if let Some(cadr) = chgsrcadr {
                                curr_frame.curadr = cadr; // may change cur adr
                            }
                            // continue to do next call
                        }
                        Outer => {
                            let cadr = chgsrcadr.unwrap();
                            if ! fn_is_public {
                                curr_frame.reclaim(r); // reclaim resource
                                return itr_err_fmt!(CallNotPublic, "contract {} func sign {}", cadr.readable(), fnptr.fnsign.hex())
                            }
                            curr_frame.ctxadr = cadr.clone(); 
                            curr_frame.curadr = cadr; 
                            // continue to do next call
                        }
                        _ => unreachable!()
                    }
                    continue
                }
            }
            // panic!("unreachable exit {:?}", exit);
            // unreachable!()
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
                *env.gas -= r.gas_extra.load_new_contract;
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
