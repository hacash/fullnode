



mod common;


#[cfg(test)]
mod deploy {

    use vm::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::rt::Bytecode::*;
    use vm::rt::AbstCall::*;
    use vm::contract::*;

    #[test]
    fn recursion() {

        /*
            VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        */

        let ircodestr = r##"
            local_move(0)
            bytecode {
                PU8 1
            }
            let foo = $0
            let bar = $1
            bar = 9 as u16
            return self.recursion(foo + bar)
        "##;


        println!("{}", lang_to_bytecodes(ircodestr).unwrap().bytecode_print(false).unwrap());




        Contract::new()
        .call(Abst::new(PayableHAC).bytecode(build_codes!(
            CU16 DUP ADD RET
        )))
        .func(Func::new("recursion").irnode(ircodestr).unwrap())
        .pfee("8:247")
        .testnet_deploy_print("8:246");
        

    }










}