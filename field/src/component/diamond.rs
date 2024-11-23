
/*
* Diamond Status
*/
pub const DIAMOND_STATUS_NORMAL                : Uint1 = Uint1::from(1);
pub const DIAMOND_STATUS_LENDING_TO_SYSTEM     : Uint1 = Uint1::from(2);
pub const DIAMOND_STATUS_LENDING_TO_USER       : Uint1 = Uint1::from(3);


/*
* Diamond Inscripts
*/
combi_list!{ Inscripts, 
	Uint1, BytesW1
}

impl Inscripts {
	pub fn array(&self) -> Vec<String> {
		let mut resv = Vec::with_capacity(self.lists.len());
		for li in &self.lists {
			let rdstr = bytes_try_to_readable_string(li.as_ref());
			resv.push(match rdstr {
				None => hex::encode(li.as_ref()),
				Some(s) => s,
			});
		}
		resv
	}
}


/*
* Diamond
*/
combi_struct!{ DiamondSto, 
	status    : Uint1
	address   : Address
	prev_engraved_height : BlockHeight
	inscripts : Inscripts
 }


/*
* DiamondSmelt
*/
combi_struct!{ DiamondSmelt, 
	diamond                   : DiamondName
	number                    : DiamondNumber
	born_height               : BlockHeight
	born_hash                 : Hash // block
	prev_hash                 : Hash // block
	miner_address             : Address
	bid_fee                   : Amount
	nonce                     : Fixed8
	// custom_message           : HashOptional
	average_bid_burn          : Uint2
	life_gene                 : Hash
}



/*
* DiamondOwnedForm
*/
combi_struct!{ DiamondOwnedForm, 
	names : BytesW4
}
impl DiamondOwnedForm {

	pub fn readable(&self) -> String {
		String::from_utf8_lossy( self.names.as_ref() ).to_string()
	}
	
	pub fn push_one(&mut self, dian: &DiamondName) {
		let mut bytes = dian.serialize();
		self.names.append(&mut bytes).unwrap();
	}

	pub fn drop_one(&mut self, dian: &DiamondName) -> Ret<usize> {
		let list = DiamondNameListMax200::one(dian.clone());
		self.drop(&list)
	}

	pub fn push(&mut self, dian: &DiamondNameListMax200) {
		let mut bytes = dian.form();
		self.names.append(&mut bytes).unwrap();
	}

	// return balance quantity
	pub fn drop(&mut self, dian: &DiamondNameListMax200) -> Ret<usize> {
		const DS: usize = DiamondName::SIZE;
		let form: &mut Vec<u8> = self.names.as_mut(); 
		let mut mstep = form.len() / DS;
		let mut istep = 0usize;
		let mut dropn = 0usize; // drop count
		let mut delst = dian.hashset();
		// loop
		while istep < mstep {
			let ix = istep * DS;
			let id = DiamondName::from(form[ix .. ix+DS].try_into().unwrap());
			if delst.contains(&id) {
				mstep -= 1;
				dropn += 1;
				delst.remove(&id);
				let tail = mstep*DS .. mstep*DS+DS;
				form.copy_within(tail, ix);
				if delst.is_empty() {
					break // all finish
				}
			}else{
				// next
				istep += 1;
			}
		}
		// check
		let ndlen = dian.length();
		assert!(dropn == ndlen, "DiamondOwnedForm need drop {} but do {}, drop {} in {}", 
			ndlen, dropn, dian.readable(), self.names.to_readable());
		// ok
		let _ = form.split_off(mstep * DS); // drop tail
		self.names.update_count()?;
		Ok(dropn)
	}


}



