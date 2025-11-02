



mod common;


#[cfg(test)]
mod deploy {
    use field::*;
    use field::interface::*;
    use protocol::action::*;

    use vm::*;
    use vm::ir::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::rt::Bytecode::*;
    use vm::rt::AbstCall::*;
    use vm::contract::*;

    #[test]
    fn deploy_update_2() {

        let _cadr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let contract = Contract::new()
        .func(Func::new("f1").public().fitsh(" return 1 ").unwrap())
        .func(Func::new("f2").public().fitsh(" return 2 ").unwrap());
        contract.testnet_deploy_print("8:244");  

        let contract = Contract::new()  
        .syst(Abst::new(AbstCall::Change).bytecode(build_codes!(P1 RET)).unwrap())
        .func(Func::new("f4").public().fitsh(" return 4 ").unwrap());
        contract.testnet_update_print(_cadr, "8:244");

    }


    #[test]
    fn deploy_update() {


        let irnode = lang_to_irnode(" return 1 ").unwrap();
        println!("{:?}", irnode);


        let _cadr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let contract = Contract::new()
        .syst(Abst::new(AbstCall::Append).bytecode(build_codes!(P0 RET)).unwrap())
        .syst(Abst::new(AbstCall::Change).bytecode(build_codes!(P0 RET)).unwrap())
        .func(Func::new("f1").public().fitsh(" return 1 ").unwrap())
        .func(Func::new("f2").public().fitsh(" return 2 ").unwrap());
        contract.testnet_deploy_print("8:244");    

        let contract = Contract::new()        
        .func(Func::new("f2").public().fitsh(" return 2 ").unwrap())
        .func(Func::new("f3").public().fitsh(" return 3 ").unwrap());
        contract.testnet_update_print(_cadr, "8:244");

        let contract = Contract::new()  
        .syst(Abst::new(AbstCall::Append).bytecode(build_codes!(P1 RET)).unwrap())
        .func(Func::new("f4").public().fitsh(" return 4 ").unwrap());
        contract.testnet_update_print(_cadr, "8:244");

        let contract = Contract::new()  
        .func(Func::new("f5").public().fitsh(" return 5 ").unwrap());
        contract.testnet_update_print(_cadr, "8:244");

        let contract = Contract::new()  
        .func(Func::new("f4").public().fitsh(" return 41 ").unwrap());
        contract.testnet_update_print(_cadr, "8:244");

        let contract = Contract::new()  
        .syst(Abst::new(AbstCall::Change).bytecode(build_codes!(P1 RET)).unwrap())
        .func(Func::new("f6").public().fitsh(" return 6 ").unwrap());
        contract.testnet_update_print(_cadr, "8:244");

        let contract = Contract::new()  
        .func(Func::new("f4").public().fitsh(" return 42 ").unwrap());
        contract.testnet_update_print(_cadr, "8:244");


    }


    #[test]
    fn recursion() {

        /*
            VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa


        */

        let recursion_fnstr= r##"
            bytecode {
                PU8 1
                PU8 2
            }
            var foo $0
            var bar $1
            if foo > 10 {
                return 10
            }
            bar = 1 as u16
            bar = self.recursion(foo + bar)
            return foo + bar
        "##;


        let payable_hac_codes = lang_to_bytecode(r##"
            var pms $0 = pick(0)
            var adr $1
            var res $2
            assert type_id(pms) == 15
            assert type_is_list(pms)
            adr = item_get(0, pms)
            adr = pms[3]
            assert type_is(12, adr)

            let bdt = pms + adr
            res = 1 + 2
            assert bdt

            return res
        "##).unwrap();



        let codes = lang_to_bytecode(recursion_fnstr).unwrap();
        println!("{}", codes.bytecode_print(false).unwrap());
        println!("{} {}", codes.len(), codes.to_hex());

        println!("payable_hac: \n{}", payable_hac_codes.bytecode_print(true).unwrap());
        println!("payable_hac codes: {}", payable_hac_codes.to_hex());


        let permit_hac = convert_ir_to_bytecode(&build_codes!(
            RET CHOISE
                GT CU64 EXTENV 1 PU8 10
                PU8 99
                PU8 0 
        )).unwrap();

        let contract = Contract::new()
        .argv(vec![0])
        .syst(Abst::new(Construct).bytecode(build_codes!(
            CU8 RET
        )).unwrap())
        .syst(Abst::new(PermitHAC).bytecode(permit_hac).unwrap())
        .syst(Abst::new(PayableHAC).bytecode(payable_hac_codes).unwrap())
        .func(Func::new("recursion").fitsh(recursion_fnstr).unwrap())
        ;
        // println!("\n\n{}\n\n", contract.serialize().to_hex());
        contract.testnet_deploy_print("2:244");    

    }


    #[test]
    // fn call_recursion() {
    fn maincall1() {

        use vm::action::*;

        let maincodes = lang_to_bytecode(r##"
            throw "1"
        "##).unwrap();

        println!("{}", maincodes.bytecode_print(true).unwrap());

        let mut act = ContractMainCall::new();
        act.ctype = Uint1::from(0);
        act.codes = BytesW2::from(maincodes).unwrap();

        // print
        curl_trs_1(vec![Box::new(act)]);

    }


    #[test]
    // fn call_recursion() {
    fn call_transfer() {

        let adr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let mut act = HacToTrs::new();
        act.to = AddrOrPtr::from_addr(adr);
        act.hacash = Amount::mei(5);

        curl_trs_1(vec![Box::new(act.clone())]);

    }








}