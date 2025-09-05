



mod common;


#[cfg(test)]
mod deploy {

    use vm::*;
    use vm::rt::Bytecode::*;
    use vm::rt::AbstCall::*;
    use vm::contract::*;

    #[test]
    fn recursion() {
        

        Contract::new()
        .call(AbstFunc::new(PayableHAC).bytecode(build_codes!(
            CU16 DUP ADD RET
        )))
        .func(UserFunc::new("testadd").irnode(r##"
            let foo = $0
            let bar = $1
            foo = 1 as u8
            bar = 9 as u16
            return foo + bar
        "##).unwrap())
        .pfee("8:247")
        .testnet_deploy_print("8:246");
        
    }










}