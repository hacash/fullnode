


mod hacds {


    use field::*;
    use field::interface::*;
    use vm::*;
    use vm::lang::*;
    use vm::contract::*;

    /*
    
    cargo test hacds::hip20 --  --nocapture
    cargo test hacds::deploy --  --nocapture
    cargo test hacds::deposit --  --nocapture
    cargo test hacds::withdraw --  --nocapture
    



curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "030068f3a51500e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001001001c6651728988000080135d4470300daabea474d082733333c1b694d80650548414344530b4841434420506965636573f8010100010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263cd6a94d27f6ba95c7710284b47449dc7820892380774ec9a9b277336cdaf628115546199f89b5c9d4e61f74a4196acc961c8d045b3a9e6944db44cf5537e1c6010800"

curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "030068f3a4ff00e63c33a796b3032ce6b856f68fccf06608d9ed18f401080001007a00000000f60104000000000000000000000000000000041b000001002e7f0472440024eba0a58524a68520c8820322096f75745f6861636473eb3892879387b2338532220405f5e100ee241c00000100427f0572440024eba28525eba2b48632220405f5e100248203b38632220405f5e100eba0a58724a68720c8820422086f75745f68616364eb389283049383043287ee241100000100297f0572440024eba58524820322086f75745f6861636482049287eba8388304eba2830485938728ee2412000001002a7f0572440024eba28525820322096f75745f686163647382049287eba8388304eba2830486938728ee240000000000000000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263cab46d2afb7aa25b8045c1c1af60da63aeb0f3623afa71927b3da704e295a737d0e9ccc9efa3f0c0241a14332de1175bf0ffcf5559b47745e72b53a9f26f8c7ce0800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "030068f39ade1600e63c33a796b3032ce6b856f68fccf06608d9ed180135d4470300daabea474d082733333c1b694d8065f4010c0002000729054b4b4b4b5641485958594859554554574e4b5759554b4b5a45595754554b001229017dcd650000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263cdab35500c0c1ac5e6c15ca0646bf6871558ef81de380c8690e73936ab4b462ee32e29a553f4c1b5297680ec068fe367ad454ded33946d25e3f26e705466c5d440800"


curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "030068f3a89d1600e63c33a796b3032ce6b856f68fccf06608d9ed180135d4470300daabea474d082733333c1b694d8065f4010c00020011290171e1a30000082903554554574e4b5759554b4b5a45595754554b00010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c7a6b7445805203e5dace84b3a8510b5a6b1387e7b6645b7561d45770fc7fb12f01a5ec88294b378e12db71cc52138eadc050a7904f533c1a8798a755d3c2354c0800"



    */

    #[test]
    fn deploy() {
        use vm::rt::AbstCall::*;

        let payable_hacd = lang_to_ircode(r##"
            param { addr, hacd, names }
            assert hacd > 0 && hacd <= 200
            var hk = "out_hacds"
            assert memory_get(hk) is nil
            let hacds = (hacd as u64) * 1_0000_0000
            memory_put(hk, hacds)
            return 0
        "##).unwrap();

        let payable_asset = lang_to_ircode(r##"
            param { addr, serial, amount }
            assert serial == 1 // check serial
            assert amount % 1_0000_0000 == 0
            var hacd = amount / 1_0000_0000
            assert hacd > 0 && hacd <= 200
            var hk = "out_hacd"
            assert memory_get(hk) is nil
            memory_put(hk, hacd as u32)
            return 0
        "##).unwrap();

        let permit_hacd = lang_to_ircode(r##"
            param { addr, dianum, names }
            assert dianum > 0 
            var hk = "out_hacd"
            var hacd = memory_get(hk)
            assert hacd is not nil
            assert hacd == dianum
            memory_put(hk, nil) // clear
            return 0
        "##).unwrap();

        let permit_asset = lang_to_ircode(r##"
            param { addr, serial, amount }
            assert serial == 1 // check serial
            var hk = "out_hacds"
            var hacds = memory_get(hk)
            assert hacds is not nil
            assert hacds == amount
            memory_put(hk, nil) // clear
            return 0
        "##).unwrap();


        

        // use vm::value::ValueTy as VT;

        let contract = Contract::new()
        .syst(Abst::new(PayableHACD).ircode(payable_hacd).unwrap())
        .syst(Abst::new(PayableAsset).ircode(payable_asset).unwrap())
        .syst(Abst::new(PermitHACD).ircode(permit_hacd).unwrap())
        .syst(Abst::new(PermitAsset).ircode(permit_asset).unwrap())
        ;
        // println!("\n{} bytes:\n{}\n\n", contract.serialize().len(), contract.serialize().to_hex());
        contract.testnet_deploy_print("8:244");    



    }



    #[test]
    fn deposit() {
        use protocol::action::*;
        
        let _addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        let _cadr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let mut act1 = DiaToTrs::new();
        act1.to = AddrOrPtr::from_ptr(1);
        act1.diamonds = DiamondNameListMax200::from_readable("KKKKVA,HYXYHY,UETWNK,WYUKKZ,EYWTUK").unwrap();
        
        let mut act2 = AssetFromTrs::new();
        act2.from = AddrOrPtr::from_ptr(1);
        act2.asset = AssetAmt::from(1, 5 * 1_0000_0000).unwrap();

        curl_trs_3(vec![Box::new(act1), Box::new(act2)], "12:244");



    }


    #[test]
    fn withdraw() {
        use protocol::action::*;
        
        let _addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        let _cadr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let mut act1 = AssetToTrs::new();
        act1.to = AddrOrPtr::from_ptr(1);
        act1.asset = AssetAmt::from(1, 3 * 1_0000_0000).unwrap();

        let mut act2 = DiaFromTrs::new();
        act2.from = AddrOrPtr::from_ptr(1);
        act2.diamonds = DiamondNameListMax200::from_readable("UETWNK,WYUKKZ,EYWTUK").unwrap();
        
        curl_trs_3(vec![Box::new(act1), Box::new(act2)], "12:244");



    }





    #[test]
    fn hip20() {

        use field::interface::*;
        // use protocol::action::*;
        // use mint::action::*;

        let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        let caddr = ContractAddress::calculate(&addr, &Uint4::default());

        println!("ContractAddress: {}", caddr.readable());

        let cadr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();
        assert!(caddr == ContractAddress::from_addr(cadr).unwrap());

        let mut act = mint::action::AssetCreate::new();
        act.metadata.issuer = cadr;
        act.metadata.serial = Fold64::from(1).unwrap();
        act.metadata.supply = Fold64::from(1800_0000_0000_0000).unwrap();
        act.metadata.decimal = Uint1::from(8);
        act.metadata.ticket = BytesW1::from(b"HACDS".to_vec()).unwrap();
        act.metadata.name = BytesW1::from(b"HACD Pieces".to_vec()).unwrap();
        act.protocol_fee = Amount::mei(1);
        curl_trs_1(vec![Box::new(act)]);

    }






}