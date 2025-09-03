

macro_rules! vmsto {
    ($ctx: expr) => {
        VMState::wrap($ctx.state())
    };
}



action_define!{ContractDeploy, 122, 
    ActLv::TopOnly, // level
    false, [],
    {   
        marks: Fixed4 // zero
        nonce: Uint4 
        contract: ContractSto
        protocol_fee: Amount // 9 times of tx fee
    },
    (self, ctx, _gas {
        if self.marks.not_zero() {
            // compatibility for future
            return errf!("marks byte error")
        }
        let hei = ctx.env().block.height;
        let maddr = ctx.env().tx.main;
        // sub protocol fee
        checked_sub_contract_protocol_fee(ctx, &self.protocol_fee)?;
        // check contract
        let caddr = ContractAddress::calculate(&maddr, &self.nonce);
        if vmsto!(ctx).contract_exist(&caddr) {
            return errf!("contract {} already exist", (*caddr).readable())
        }
        // check
        map_itr_err!(self.contract.check(hei))?;
        // save the contract
        vmsto!(ctx).contract_set(&caddr, &self.contract);
        Ok(vec![])
    })
}


action_define!{ContractChange, 123, 
    ActLv::TopOnly, // level
    false, [],
    {   
        marks: Fixed2 // zero
        address: Address // contract address
        contract: ContractSto
        protocol_fee: Amount // 9 times of tx fee
    },
    (self, ctx, _gas {
        use AbstCall::*;
        if self.marks.not_zero() {
            return errf!("marks byte error")
        }
        let hei = ctx.env().block.height;
        // sub protocol fee
        checked_sub_contract_protocol_fee(ctx, &self.protocol_fee)?;
        // load old
        let caddr = ContractAddress::from_addr(self.address)?;
        let Some(mut contract) = vmsto!(ctx).contract(&caddr) else {
            return errf!("contract {} not exist", (*caddr).readable())
        };
        // merge and check
		map_itr_err!(self.contract.check(hei))?;
        let is_edit = map_itr_err!(contract.merge(&self.contract, hei))?;
        let depth = 1; // sys call depth is 1
        let cty = CallTy::Abst as u8;
        let sys = maybe!(is_edit, Change, Append) as u8; // Upgrade or Append
        setup_vm_run(depth, ctx, cty, sys, caddr.as_bytes(), vec![])?;
        // save the new
        vmsto!(ctx).contract_set(&caddr, &contract);
        Ok(vec![]) 
    })
}




/**************************************/


fn checked_sub_contract_protocol_fee(ctx: &mut dyn Context, ptcfee: &Amount) -> Rerr {

    let _hei = ctx.env().block.height;
    let maddr = ctx.env().tx.main;
    let tx9fee = &ctx.env().tx.fee.dist_mul(9)?;
    // check fee
    if ptcfee < tx9fee { 
        return errf!("protocol fee must need at least {} but just got {}", tx9fee, ptcfee)
    }
    operate::hac_sub(ctx, &maddr, ptcfee)?;
    Ok(())
}