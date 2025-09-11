



mod common;


#[cfg(test)]
mod deploy {
    use field::*;
    use field::interface::*;
    use protocol::action::*;

    use vm::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::rt::Bytecode::*;
    use vm::rt::AbstCall::*;
    use vm::contract::*;

    #[test]
    fn verify_codes() {

        verify_bytecodes(&build_codes!(
            PU8 1 JMPL 0 8 JMPL 0 2 RET
        )).unwrap()

    }


    #[test]
    fn recursion() {

        /*
            VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        */

        let recursion_fnstr= r##"
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


        let permithac_codes = lang_to_bytecodes(r##"
            local_move(0)
            let argv = $0
            let mei  = $1
            argv = buffer_left_drop(21, argv)
            mei = amount_to_mei(argv)
            return choise(mei<=4, true, false)
        "##).unwrap();



        let codes = lang_to_bytecodes(recursion_fnstr).unwrap();
        println!("{}", codes.bytecode_print(false).unwrap());
        println!("{} {}", codes.len(), codes.to_hex());

        println!("permithac: \n{}", permithac_codes.bytecode_print(true).unwrap());


        Contract::new()
        .call(Abst::new(PermitHAC).bytecode(permithac_codes))
        .call(Abst::new(PayableHAC).bytecode(build_codes!(
            P1 RET
        )))
        .func(Func::new("recursion").irnode(recursion_fnstr).unwrap())
        .testnet_deploy_print("2:244");    

    }


    #[test]
    // fn call_recursion() {
    fn call_transfer() {

        let adr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let mut act = HacFromTrs::new();
        act.from = AddrOrPtr::from_addr(adr);
        act.hacash = Amount::mei(19);

        curl_trs_1(vec![Box::new(act.clone())]);

    }








}