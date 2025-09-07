



mod common;


#[cfg(test)]
mod deploy {
     use field::interface::*;

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
            if foo > 10 {
                return 10
            }
            let bar = $1
            bar = 1 as u16
            bar = self.recursion(foo + bar)
            return foo + bar
        "##;

        let codes = lang_to_bytecodes(ircodestr).unwrap();
        println!("{}", codes.bytecode_print(false).unwrap());
        println!("{} {}", codes.len(), codes.to_hex());

        Contract::new()
        .call(Abst::new(PayableHAC).bytecode(build_codes!(
            CU16 DUP ADD RET
        )))
        .func(Func::new("recursion").irnode(ircodestr).unwrap())
        .testnet_deploy_print("8:246");
        

    }










}