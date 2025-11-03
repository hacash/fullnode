
#[cfg(test)]
#[allow(unused)]
mod inherit {

    use field::*;
    use field::interface::*;

    use sha2::digest::typenum::Abs;
    use vm::action::*;
    use vm::ir::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::contract::*;
    use vm::ContractAddress;

    #[test]
    fn addrs() {
        /*
        VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK
        hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH
        oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L
        cmhfWCVLLosyQujfPnf86spZVW4exD2yr
        WzK23CAKQFzoPpMEioBztv9yaASvJxNZM
        ezFkqc6Smyk5DGvMY6bMoYx6vsU4gs7ba
        UJ7Ypo4SpQibMudmEjJKbMUN7Zy9viyKS
        cCBdc3vTmsBzPXbn2SaQy6dfbpvM6aJmK
        bX96F9rJNYSBi3iE7vj2bQ75ChaTq5KsU
        SckiYHndzCkKApYhAa9fK2vLfkAunN3w3
        Td6MYJaoEbwo9JdebnCfcZs9qPAKuJz8A
        */
        let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        for i in 0 .. 12 {        
            let caddr = ContractAddress::calculate(&addr, &Uint4::from(i));
            println!("{}", caddr.readable());
        }
    }


    #[test]
    fn deploy() {



        // VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        let contract = Contract::new()
        .func(Func::new("f1").public().fitsh("return 1").unwrap())
        // .func(Func::new("f2").fitsh(" return 2 ").unwrap())
        .func(Func::new("f3").fitsh("return 3").unwrap())
        ;
        contract.testnet_deploy_print("8:244");

        // nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK
        let contract = Contract::new()
        .inh(Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap())
        .func(Func::new("f3").fitsh("return 31").unwrap())
        .func(Func::new("f4").fitsh("return 4").unwrap())
        .func(Func::new("f5").public().fitsh(r##"
            print self.f1()
            print self.f2()
            print self.f3()
            print self.f4()
            end
        "##).unwrap())
        ;
        contract.testnet_deploy_print_by_nonce("8:244", 1);


    }

    #[test]
    fn deploy2() {

        // VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        Contract::new()
        .func(Func::new("f1").public().fitsh("return 1").unwrap())
        .func(Func::new("f2").fitsh("return 2").unwrap())
        .func(Func::new("f3").fitsh("return 3").unwrap())
        .testnet_deploy_print("8:244");

        // nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK
        Contract::new()
        .func(Func::new("f3").fitsh("return 33").unwrap())
        .func(Func::new("f4").fitsh("return 44").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 1);

        // hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH
        Contract::new()
        .inh(Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap())
        .inh(Address::from_readable("nakxhQZ2bhKDwKhowM18wyPTDkTDL1yNK").unwrap())
        .syst(Abst::new(AbstCall::Append).fitsh("return 0").unwrap())
        .func(Func::new("f5").public().fitsh("
            print self.f1()
            print self.f2()
            print self.f3()
            print self.f4()
            end
        ").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 2);

        // oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L
        Contract::new()
        .func(Func::new("f6").fitsh("return 6").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 3);
        
        // cmhfWCVLLosyQujfPnf86spZVW4exD2yr
        Contract::new()
        .func(Func::new("f7").fitsh("return 7").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 4);
        
        // WzK23CAKQFzoPpMEioBztv9yaASvJxNZM
        Contract::new()
        .func(Func::new("f8").fitsh("return 8").unwrap())
        .testnet_deploy_print_by_nonce("8:244", 5);
        
        // 
        let adr = Address::from_readable("hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH").unwrap();
        Contract::new()
        .inh(Address::from_readable("oSPKj5vT2qkrS2ZWL2AMB6AHS5e9mi77L").unwrap())
        .inh(Address::from_readable("cmhfWCVLLosyQujfPnf86spZVW4exD2yr").unwrap())
        .func(Func::new("f9").public().fitsh("
            print self.f1()
            print self.f2()
            print self.f3()
            print self.f4()
            // print self.f5()
            print self.f6()
            print self.f7()
            // print self.f8()
            end
        ").unwrap())
        .testnet_update_print(adr, "8:244");




    }


    #[test]
    fn call1() {


        let maincodes = lang_to_bytecode(r##"
            lib C = 3: hXMHE4TjtUvvuzyevjjRruxiz2yxuT1zH
            C.f9()
            end
        "##).unwrap();

        let act = ContractMainCall::from_bytecode(maincodes).unwrap();
        // print
        curl_trs_3(vec![Box::new(act)], "24:244");
    }



}


/*



curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006908753000e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007af6010400000000000000000000000000000000000000000003de995b0300000081000005f20001ee25d722a68300000001000005f20001ee267b2317ff00000001000005f20001ee27000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263ce176dda0adc0fb6fc8666558c73862c69a3424aa1662a6775c81f57d2fcb32e9320d19fed38c140e9b655ab400579b91a71fe2579042653345a4742fd4e7390c0800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006908753000e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007af60104000000010000000000000000000000000000000000027b2317ff00000001000006f20001ee2021e79fc72b00000001000006f20001ee202c000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263cb15242777078e7d11b358024b219d4d28e4ae1ab162fcfffa81ef1ebf1eb82073d30f1298e5a64f15478d07fa42575627ab3c615cf471fbbac1b2a72aedaa7df0800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006908753000e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007af601040000000200000000000000000000000000020135d4470300daabea474d082733333c1b694d806501f400154d2b7c2ed1c02865afddf86931ed25b1af0001020000010005f20001ee240001c57f6d0b00000081000020f20005ea12de995b0328ea12d722a68328ea127b2317ff28ea12e79fc72b28ef000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c17bf52b7585b711707374b01858c876bc463ba51f95ea7a26ec22a36cdd67faa4c9061de8809bceca0edee00ce99d64ca041c53f9f3bd2302303db549bfb877d0800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006908753000e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007af6010400000003000000000000000000000000000000000001737d2c9200000001000006f20001ee2006000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c7f00cc37c2a597be8d5155e3c7387d731f9af1b85b8dc3502edce0064e3192067752843f7675d8fac4e02581dd62e5e7b8637aa82912cac53998737ee78ba31d0800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006908753000e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007af6010400000004000000000000000000000000000000000001b327224200000001000006f20001ee2007000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c16b94fc8fa69e50f9c5fe5fc93ddbf46a45a155464dcd065bee8f8e2983d525429d638fb61d5f60199dda191ce7c0c456cea9c5f6fd137d8c613aada8d9a97330800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006908753000e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007af601040000000500000000000000000000000000000000000142f912ee00000001000006f20001ee2008000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c1270e8c3c287a660c065c7ff3e66240795c166e77b849bcbac31e80a87e1204a6e23e4cf343c571e1f3dec20559ce33b012518d33c09a1af2d8395fd86a226be0800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006908753000e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007bf6010401bc82705a9eeffc16ac978a79303ffa3e5e5da0c3000000000000000000000201fd62d1bc4edc1d83f741b757b02c8e5eed74f3f8018860a6339e9297eab2ac7fdc4accce24ddb6c351000000013165001d0000008100002ef20007ea12de995b0328ea12d722a68328ea127b2317ff28ea12e79fc72b28ea12737d2c9228ea12b327224228ef000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c4d7819673228d9ccc51e697f916159ea8ffe3d7b97fbb1fde3a3b61eaf7573d340831c0f63d505804a8a323b0dbaad5bb63ed344d4a2fbbb9a0ee49e6ec42ae30800"




*/