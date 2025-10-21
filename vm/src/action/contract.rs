

pub const CONTRACT_STORE_FEE_MUL: u64 = 50;


macro_rules! vmsto {
    ($ctx: expr) => {
        VMState::wrap($ctx.state())
    };
}



action_define!{ContractDeploy, 122, 
    ActLv::TopUnique,
    false, [],
    {   
        protocol_cost: Amount
        nonce: Uint4 
        construct_argv: BytesW1 // max 1024
        _marks_:   Fixed4 // zero
        contract: ContractSto
    },
    (self, ctx, _gas {
        if self._marks_.not_zero() { // compatibility for future
            return errf!("marks byte error")
        }
        let hei = ctx.env().block.height;
        let maddr = ctx.env().tx.main;
        // check contract
        let caddr = ContractAddress::calculate(&maddr, &self.nonce);
        if vmsto!(ctx).contract_exist(&caddr) {
            return errf!("contract {} already exist", (*caddr).readable())
        }
        // spend protocol fee
        check_sub_contract_protocol_fee(ctx, &self.protocol_cost)?;
        // check
        map_itr_err!(self.contract.check(hei))?;
        let accf  = AbstCall::Construct;
        let hvaccf = self.contract.have_abst_call(accf);
        // save the contract
        vmsto!(ctx).contract_set(&caddr, &self.contract);
        // call the construct function
        let cargv = self.construct_argv.to_vec();
        if cargv.len() > SpaceCap::new(hei).max_value_size {
            return errf!("construct argv size overflow")
        }
        if hvaccf { // have Construct func
            let depth = 1; // sys call depth is 1
            let cty = CallMode::Abst as u8;
            setup_vm_run(depth, ctx, cty, accf as u8, caddr.as_bytes(), Value::Bytes(cargv))?;
            // drop Construct func
            let mut contract = self.contract.clone();
            contract.drop_abst_call(accf);
            vmsto!(ctx).contract_set(&caddr, &contract);
        }
        // ok finish
        Ok(vec![])
    })
}






action_define!{ContractUpdate, 123, 
    ActLv::TopUnique, // level
    false, [], // burn 90% fee
    {   
        protocol_cost: Amount
        address: Address // contract address
        _marks_: Fixed2 // zero
        contract: ContractSto
    },
    (self, ctx, _gas {
        use AbstCall::*;
        if self._marks_.not_zero() {
            return errf!("marks byte error")
        }
        let hei = ctx.env().block.height;
        // load old
        let caddr = ContractAddress::from_addr(self.address)?;
        let Some(mut contract) = vmsto!(ctx).contract(&caddr) else {
            return errf!("contract {} not exist", (*caddr).readable())
        };
        // spend protocol fee
        check_sub_contract_protocol_fee(ctx, &self.protocol_cost)?;
        // merge and check
		map_itr_err!(self.contract.check(hei))?;
        let is_edit = map_itr_err!(contract.merge(&self.contract, hei))?;
        let depth = 1; // sys call depth is 1
        let cty = CallMode::Abst as u8;
        let sys = maybe!(is_edit, Change, Append) as u8; // Upgrade or Append
        setup_vm_run(depth, ctx, cty, sys, caddr.as_bytes(), Value::Nil)?;
        // save the new
        vmsto!(ctx).contract_set(&caddr, &contract);
        Ok(vec![]) 
    })
}




/**************************************/



fn check_sub_contract_protocol_fee(ctx: &mut dyn Context, pfee: &Amount) -> Rerr {
    if pfee.is_negative() {
		return errf!("protocol fee cannot be negative")
    }
	if pfee.size() > 4 {
		return errf!("protocol fee amount size cannot over 4 bytes")
	}
    // let _hei = ctx.env().block.height;
    // let e = errf!("contract protocol fee calculate failed");
    let mul = CONTRACT_STORE_FEE_MUL as u128; // 50
    let tx50fee = ctx.tx().fee().dist_mul(mul)?;
    let maddr = ctx.env().tx.main;
    // println!("{}, {}, {}, {}", ctx.tx().size(), _ctlsz, ctx.tx().fee(), tx50fee);
    // check fee
    if pfee < &tx50fee { 
        return errf!("protocol fee must need at least {} but just got {}", &tx50fee, pfee)
    }
    operate::hac_sub(ctx, &maddr, pfee)?;
    Ok(())
}




/**************************************

fn check_sub_contract_protocol_fee(ctx: &mut dyn Context, ctlsz: usize, ptcfee: &Amount) -> Rerr {
    // let _hei = ctx.env().block.height;
    let e = errf!("contract protocol fee calculate failed");
    let mul = CONTRACT_STORE_FEE_MUL as u128; // 30
    let feep = ctx.tx().fee_purity() as u128 / GSCU as u128;
    let Some(rlfe) = feep.checked_mul(ctlsz as u128) else {
        return e
    };
    let Some(rlfe) = rlfe.checked_mul(mul) else {
        return e
    };
    let tx50fee = &Amount::coin_u128(rlfe, UNIT_238).compress(2, AmtCpr::Grow)?;
    if tx50fee <= ctx.tx().fee() {
        return e
    }
    println!("{}, {}, {}, {}", ctx.tx().size(), ctlsz, ctx.tx().fee(), tx50fee);
    let maddr = ctx.env().tx.main;
    // check fee
    if ptcfee < tx50fee { 
        return errf!("protocol fee must need at least {} but just got {}", tx50fee, ptcfee)
    }
    operate::hac_sub(ctx, &maddr, ptcfee)?;
    Ok(())
}


*/