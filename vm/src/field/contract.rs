use std::collections::*;



// Contract Head
combi_struct!{ ContractMeta, 
    vrsn: Fixed1 // 4bit16 = version
	mark: Fixed3
	mext: Fixed4
}

// Contract Abst Call
combi_struct!{ ContractAbstCall, 
	sign: Fixed1
	mark: Fixed2
	cdty: Fixed1 // 3bit8 = codetype
    code: BytesW2
}

// Contract User Func
combi_struct!{ ContractUserFunc, 
	sign: Fixed4
	mark: Fixed3
	cdty: Fixed1 // 1bit = is_public, 3bit8 = codetype
    code: BytesW2
}

// Contract address list
combi_list!(ContractAddrsssListW1, Uint1, ContractAddress);

impl ContractAddrsssListW1 {
	pub fn check_repeat(&self, src: &Self) -> bool {
		self.lists.iter().any(|a|src.lists.contains(a))
	}
}


// Func List
combi_list!(ContractAbstCallList, Uint1, ContractAbstCall);
combi_list!(ContractUserFuncList, Uint2, ContractUserFunc);


macro_rules! func_list_merge_define {
	($ty:ty) => {
		// return edit or push
		fn addition(&mut self, func: $ty) -> Ret<bool> {
			let list = self.list();
			for i in 0..self.length() {
				if list[i].sign == func.sign {
					self.replace(i, func)?;
					return Ok(false)
				}
			}
			// push
			self.push(func)?;
			Ok(false)
		}
		// return edit or push
		fn check_merge(&mut self, src: &Self) -> VmrtRes<bool> {
			let mut edit = false;
			for a in src.list() {
				if map_err_itr!(ContractUpgradeErr, self.addition(a.clone()))? {
					edit = true;
				}
			}
			Ok(edit)
		}
	};
}


impl ContractAbstCallList {
	func_list_merge_define!{ ContractAbstCall }
}

impl ContractUserFuncList {
	func_list_merge_define!{ ContractUserFunc }
}

// Contract
combi_struct!{ ContractSto, 
	metas: ContractMeta
	inherits:  ContractAddrsssListW1
    librarys:  ContractAddrsssListW1
	abstcalls: ContractAbstCallList
	userfuncs: ContractUserFuncList
    morextend: Uint8
}


impl ContractSto {

	/*
    	return Upgrade or Append for check
	*/
	pub fn merge(&mut self, src: &ContractSto, hei: u64) -> VmrtRes<bool> {
		use ItrErrCode::*;
		src.check(hei)?;
		let cap = SpaceCap::new(hei);
		if self.inherits.length() + src.inherits.length() > cap.inherits_parent {
			return itr_err_fmt!(InheritsError, "inherits number overflow")
		}
		if self.librarys.length() + src.librarys.length() > cap.librarys_link {
			return itr_err_fmt!(LibrarysError, "librarys link number overflow")
		}
		// inhs and libs check repeat
		if self.inherits.check_repeat(&src.inherits) {
			return itr_err_fmt!(InheritsError, "inherits cannot repeat")
		}
		if self.librarys.check_repeat(&src.librarys) {
			return itr_err_fmt!(LibrarysError, "librarys cannot repeat")
		}
		// append inherits and librarys
		self.inherits.append(src.inherits.lists.clone()).unwrap();
		self.librarys.append(src.librarys.lists.clone()).unwrap();
		// merge abst call
		let edit1 = self.abstcalls.check_merge(&src.abstcalls)?;
		// merge usrfun call
		let edit2 = self.userfuncs.check_merge(&src.userfuncs)?;
		// check size
		if self.size() > cap.max_contract_size {
			return itr_err_fmt!(ContractError, "contract size overflow, max {}", cap.max_contract_size)
		}
		// ok
		Ok(edit1 || edit2)
	}

	pub fn check(&self, hei: u64) -> VmrtErr {
		use ItrErrCode::*;
		let cap = SpaceCap::new(hei);
		if self.inherits.length() > cap.inherits_parent {
			return itr_err_fmt!(InheritsError, "inherits number overflow")
		}
		if self.librarys.length() > cap.librarys_link {
			return itr_err_fmt!(LibrarysError, "librarys link number overflow")
		}
		// abst call
		for a in self.abstcalls.list() {
			AbstCall::check(a.sign[0])?;
			let ctype = CodeType::parse(a.cdty[0])?;
			try_compile_check(ctype, &a.code)?; // // check compile
		}
		// usrfun call
		for a in self.userfuncs.list() {
			let ctype = CodeType::parse(a.cdty[0])?;
			try_compile_check(ctype, &a.code)?; // check compile
		}
		// check size
		if self.size() > cap.max_contract_size {
			return itr_err_fmt!(ContractError, "contract size overflow, max {}", cap.max_contract_size)
		}
		if 0 != *self.morextend {
			return itr_err_fmt!(ContractError, "extend data format error")
		}
		// ok
		Ok(())
	}
}


//////////////////////////////////////




#[derive(Default)]
pub struct ContractObj {
	pub sto: ContractSto,
	pub abstfns: HashMap<AbstCall, Arc<FnObj>>,
	pub userfns: HashMap<FnSign, Arc<FnObj>>,
}


impl ContractSto {

	pub fn into_obj(self) -> VmrtRes<ContractObj> {
		let mut obj = ContractObj {
			sto: self,
            ..Default::default()
		};
		// parse sytmcalls
		for a in obj.sto.abstcalls.list() {
			let code = FnObj::create(a.cdty[0], a.code.to_vec())?;
			let cty = std_mem_transmute!( a.sign[0] );
			obj.abstfns.insert(cty, code.into());
		}
		// parse userfuncs
		for a in obj.sto.userfuncs.list() {
			let code = FnObj::create(a.cdty[0], a.code.to_vec())?;
			let cty = a.sign.to_array();
			obj.userfns.insert(cty, code.into());
		}
		// ok
		Ok(obj)
	}
}
















