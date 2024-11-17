
// Contract Head
combi_struct!{ ContractHead, 
	marks: Fixed10
}

// Contract System Call
combi_struct!{ ContractSystemCall, 
    mark: Fixed2
	sign: Uint1
    code: BytesW2
}

// Contract User Func
combi_struct!{ ContractClientFunc, 
    mark: Fixed6
	sign: Uint4
    code: BytesW2
}

// Func List
combi_list!{ ContractSystemCallList, Uint1, ContractSystemCall }
combi_list!{ ContractClientFuncList, Uint2, ContractClientFunc }


//////////////////////////////////////



// Contract
combi_struct!{ Contract, 
	headmarks: ContractHead
	sytmcalls: ContractSystemCallList
	functions: ContractClientFuncList
}
