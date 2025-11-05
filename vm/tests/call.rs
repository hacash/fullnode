
#[cfg(test)]
#[allow(unused)]
mod call {

    use field::*;
    use field::interface::*;

    use vm::ir::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::contract::*;

    fn addr(s: &str) -> Address {
        Address::from_readable(s).unwrap()
    }


    #[test]
    fn inh1() {

        // VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        Contract::new()
        .func(Func::new("f1").public().fitsh("return 1").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 0);
    
        // nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK
        Contract::new()
        .lib(addr("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa"))
        .func(Func::new("f2").fitsh("
            lib C = 0
            return C::f1() + 1
        ").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 1);

        // hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH
        Contract::new()
        .lib(addr("nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK"))
        .func(Func::new("f3").fitsh("
            lib C = 0
            return C:f2() + 1
        ").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 2);

        // oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L
        Contract::new()
        .lib(addr("hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH"))
        .inh(addr("nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK"))
        .func(Func::new("f4").public().fitsh("
            lib C = 0
            return C:f3() + self.f2() + 1
        ").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 3);

        // main call
        Maincall::new().fitsh("
            lib C = 4
            print C.f4() + 1
            end
        ").unwrap()
        .testnet_call_print("8:244");


        




    }


    #[test]
    fn deploy() {

        let recursion_fn = r##"
            var num = pick(0)
            // print num 
            if num >= 30 {
                return "overflow"
            }
            var nk = "num_k"
            var mmm = memory_get(nk)
            if mmm is nil {
                mmm = 1
                memory_put(nk, mmm)
            }
            if mmm >= 10 {
                return "ok"
            }
            memory_put(nk, mmm + 1)
            return self.recursion(num + 1)
        "##;

        println!("{}", lang_to_bytecode(recursion_fn).unwrap().bytecode_print(false).unwrap());


        let contract = Contract::new()
        .func(Func::new("recursion").public().fitsh(recursion_fn).unwrap())
        ;
        // println!("\n\n{}\n\n", contract.serialize().to_hex());
        contract.testnet_deploy_print("8:244");    




    }


    #[test]
    // fn call_recursion() {
    fn maincall1() {

        use vm::action::*;

        let maincodes = lang_to_bytecode(r##"
            lib C = 1: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            C.recursion(1)
            end
        "##).unwrap();

        println!("{}", maincodes.bytecode_print(true).unwrap());

        let act = ContractMainCall::from_bytecode(maincodes).unwrap();

        // print
        curl_trs_3(vec![Box::new(act)], "24:244");

    }




}