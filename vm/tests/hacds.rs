


mod hacds {


    use field::*;
    use vm::*;
    use vm::lang::*;
    use vm::contract::*;





    #[test]
    fn deploy() {
        use vm::rt::AbstCall::*;

        let payable_hacd = lang_to_ircode(r##"
            param { addr, num, names }
            assert num <= 200

            






            return 0
        "##).unwrap();

        let payable_asset = lang_to_ircode(r##"
            return 0
        "##).unwrap();

        let permit_hacd = lang_to_ircode(r##"
            return 0
        "##).unwrap();

        let permit_asset = lang_to_ircode(r##"
            return 0
        "##).unwrap();

        let _deposit_codes = lang_to_bytecode(r##"
            return 0
        "##).unwrap();

        let _withdraw_codes = lang_to_bytecode(r##"
            return 0
        "##).unwrap();


        

        // use vm::value::ValueTy as VT;

        let contract = Contract::new()
        .syst(Abst::new(PayableHACD).ircode(payable_hacd).unwrap())
        .syst(Abst::new(PayableAsset).ircode(payable_asset).unwrap())
        .syst(Abst::new(PermitHACD).ircode(permit_hacd).unwrap())
        .syst(Abst::new(PermitAsset).ircode(permit_asset).unwrap())
        // .func(Func::new("deposit").public()
        //     .types(Some(VT::U64), vec![]).bytecode(deposit_codes))
        // .func(Func::new("withdraw").public()
        //     .types(Some(VT::Bytes), vec![VT::U64]).bytecode(withdraw_codes))
        ;
        // println!("\n{} bytes:\n{}\n\n", contract.serialize().len(), contract.serialize().to_hex());
        contract.testnet_deploy_print("8:244");    



    }



    #[test]
    fn deposit() {






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
        act.metadata.supply = Fold64::from(1800_0000_0000).unwrap();
        act.metadata.decimal = Uint1::from(4);
        act.metadata.ticket = BytesW1::from(b"HACDS".to_vec()).unwrap();
        act.metadata.name = BytesW1::from(b"HACD Pieces".to_vec()).unwrap();
        act.protocol_fee = Amount::mei(1);
        curl_trs_1(vec![Box::new(act)]);

    }






}