
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
		let mut list = DiamondNameListMax200::default();
		list.push(dian.clone()).unwrap();
		self.drop(&list)
	}

	pub fn push(&mut self, dian: &DiamondNameListMax200) {
		let mut bytes = dian.form();
		self.names.append(&mut bytes).unwrap();
	}

	// return balance quantity
	pub fn drop(&mut self, dian: &DiamondNameListMax200) -> Ret<usize> {

		let l = DiamondName::SIZE;
		let srclen = dian.count().to_usize();
		let mut dianset = dian.hashset();
		let dstsz = self.names.length();
		let dstlen = dstsz / l;
		let mut rmleft = 0;
		for i in 0..dstlen {
			let x = i*l;
			let y = x+l;
			let dia = DiamondName::from(self.names.bytes[x..y].try_into().unwrap());
			if dianset.contains(&dia) {
				dianset.remove(&dia);
				if x == rmleft {
					// drop head, do nohing
				}else{
					let cvd = rmleft .. rmleft+l;
					self.names.bytes.copy_within(cvd, x);
				}
				rmleft += l;
			}
			if dianset.is_empty() {
				break // all finish
			}
		}
		if rmleft/l != srclen {
			println!("names = {}", self.readable());
			println!("rmleft/l={}, srclen={}, dstlen={}", rmleft/l, srclen, dstlen);
			return errf!("drop {} not match", srclen)
		}
		self.names.bytes = self.names.bytes.split_off(rmleft);
		self.names.count -= (srclen * l) as u64;
		Ok(self.names.count.to_usize())

	}

}



