
#[cfg(test)]
#[allow(unused)]
mod library {

    use field::*;
    use field::interface::*;

    use sha2::digest::typenum::Abs;
    use vm::action::*;
    use vm::ir::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::contract::*;
    use vm::ContractAddress;

    fn addr(s: &str) -> Address {
        Address::from_readable(s).unwrap()
    }

    #[test]
    fn t1() {

        println!("{}", lang_to_ircode("
            lib C = 0: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            var n1 = C:f1()
            end
            // return n1 + 2
        ").unwrap().ircode_print(false).unwrap());

    }

    #[test]
    fn deploy() {

        // VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        Contract::new()
        .func(Func::new("f1").public().fitsh("return 1").unwrap())
        .func(Func::new("f2").fitsh("return 2").unwrap())
        .testnet_deploy_print("8:244");

        // nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK
        Contract::new()
        .lib(addr("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa"))
        .func(Func::new("f3").public().fitsh("
            lib C = 0: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            var n1 = C::f1()
            var n2 = C::f2()
            return n1 + n2
        ").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 1);

        // hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH
        Contract::new()
        .func(Func::new("f4").public().fitsh("return 4").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 2);
        
        // oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L
        Contract::new()
        .func(Func::new("f5").public().fitsh("return 5").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 3);

        // cmhfWCVLLosyQujfPnf86spZVW4exD2yr
        Contract::new()
        .lib(addr("nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK"))
        .lib(addr("hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH"))
        .lib(addr("oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L"))
        .func(Func::new("f6").public().fitsh("
            lib C0 = 0
            lib C1 = 1
            lib C2 = 2
            print C0::f3()
            print C1::f4()
            print C2::f5()
            end
        ").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 4);


        





    }


}

