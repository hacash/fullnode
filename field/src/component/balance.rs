
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
		SatoshiAuto::from( sat.uint() )
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




// Balance
combi_struct!{ Balance, 
	hacash:  Amount
	satoshi: SatoshiAuto
    diamond: DiamondNumberAuto
}

impl Balance {

	pub fn from_hacash(amt: Amount) -> Balance {
		Balance{
			hacash: amt,
			satoshi: SatoshiAuto::default(),
			diamond: DiamondNumberAuto::default(),
		}
	}

}
