
// Sign Item
combi_struct!{ Sign, 
	publickey: Fixed33
	signature: Fixed64
}

impl Sign {
	pub fn create_by(acc: &Account, stuff: &Hash) -> Self {
		Self{
			publickey: Fixed33::from(acc.public_key().serialize_compressed()),
			signature: Fixed64::from( acc.do_sign(&stuff) ),
		}
	}
}


// SignCheckData
combi_struct!{ SignCheckData, 
	signdata: Sign
	stuffstr: BytesW2
}


// SignList MaxLen 255
combi_list!(SignListW1, Uint1, Sign);


// SignList MaxLen 65535
combi_list!(SignListW2, Uint2, Sign);



