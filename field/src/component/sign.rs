
// Sign Item
combi_struct!{ Sign, 
	publickey: Fixed33
	signature: Fixed64
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



