
#[cfg(test)]
#[allow(unused)]
mod main {

    use field::*;
    use field::interface::*;

    use vm::action::*;
    use vm::ir::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::contract::*;

    #[test]
    fn deploy() {

        let _fn1 = r##"
            param { num }
            var data = memory_get("k")
            if data is not nil {
                return 0 
            }
            return num * 2 + 1
        "##;


        let _fn2 = r##"
            print global_get("k")
            end
        "##;

        // println!("{}", lang_to_ircode(&recursion_fn).unwrap().ircode_print(false).unwrap());

        let contract = Contract::new()
        .func(Func::new("f1").fitsh(_fn1).unwrap())
        .func(Func::new("f2").fitsh(_fn2).unwrap())
        ;
        // println!("\n\n{}\n\n", contract.serialize().to_hex());
        contract.testnet_deploy_print("8:244");    

    }


    #[test]
    fn call2() {


        let maincodes = lang_to_bytecode(r##"
            lib C = 1: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            global_put("k", 123 as u32)
            callcode C::f2
            end
        "##).unwrap();

        let act = ContractMainCall::from_bytecode(maincodes).unwrap();
        // print
        curl_trs_3(vec![Box::new(act)], "24:244");
    }


    #[test]
    fn call1() {


        let maincodes = lang_to_bytecode(r##"
            lib C = 1: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            var num = C:f1(2)
            print num
            end
        "##).unwrap();

        println!("{}", maincodes.bytecode_print(true).unwrap());

        let act = ContractMainCall::from_bytecode(maincodes).unwrap();

        // print
        curl_trs_3(vec![Box::new(act)], "24:244");

    }




}