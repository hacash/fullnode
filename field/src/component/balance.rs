
// Satoshi
pub type SatoshiAuto = Fold64;
combi_optional!{ SatoshiOptional, 
	satoshi: Satoshi 
}
impl SatoshiAuto {
	pub fn to_satoshi(&self) -> Satoshi {
		Satoshi::from( self.uint() )
	}
	pub fn from_satoshi(sat: &Satoshi) -> SatoshiAuto {
		SatoshiAuto::from( sat.uint() ).unwrap()
	}
}


// AddrHac
combi_struct!{ AddrHac,
	address: Address
	amount : Amount
}

// HacAndSat
combi_struct!{ HacSat, 
	amount : Amount
	satoshi: SatoshiOptional
}

// AddrHacSat
combi_struct!{ AddrHacSat, 
	address: Address
	hacsat : HacSat
}

// AddrBalance
combi_struct!{ AddrBalance, 
	address: Address
	balance: Balance
}

combi_list!{ AssetAmtW1, Uint1, AssetAmt }

// Balance
combi_struct!{ Balance, 
	hacash:  Amount
	satoshi: SatoshiAuto
    diamond: DiamondNumberAuto
	assets: AssetAmtW1
}

pub const BALANCE_ASSET_MAX: usize = 20;

impl Balance {

	pub fn hac(amt: Amount) -> Self {
		Self {
			hacash: amt,
			..Default::default()
		}
	}

	pub fn asset(&self, seri: Fold64) -> Option<AssetAmt> {
		self.assets.list().iter().find(|a|a.serial==seri).map(|a|a.clone())
	}

	pub fn asset_must(&self, seri: Fold64) -> AssetAmt {
		self.asset(seri).unwrap_or(AssetAmt::new(seri))
	}

	pub fn asset_set(&mut self, amt: AssetAmt) -> Rerr {
		let assets = self.assets.as_mut();
		for i in 0..assets.len() {
			let ast = assets.get_mut(i).unwrap();
			if ast.serial == amt.serial {
				if 0 == *amt.amount {
					self.assets.drop(i).unwrap();// delete
				} else {
					*ast = amt; // update
				}
				return Ok(())
			}
		}
		self.assets.push(amt)?;
		if self.assets.length() > BALANCE_ASSET_MAX {
			return errf!("balance asset item quantity cannot big than {}", BALANCE_ASSET_MAX)
		}
		Ok(())
	}


}
