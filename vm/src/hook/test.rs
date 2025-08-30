






#[cfg(test)]
mod maincall {
    use vm::action::*;
    use ::field::interface::*;
    use field::*;
    use protocol::action::*;
    use protocol::interface::*;
    use protocol::transaction::*;
    use mint::action::*;
    use sys::*;
    use vm::*;
    use vm::rt::*;


    fn curl_trs(acts: Vec<Box<dyn Action>>) {

        let acc = Account::create_by_password("123456").unwrap();
        let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        let fee = Amount::small(2, 245);

        let mut trs = TransactionType3::new_by(addr, fee, curtimes());
        
        for act in acts {
            trs.push_action(act).unwrap();
        }

        trs.gas_max = Uint1::from(4);
        trs.fill_sign(&acc).unwrap();

        // print
        println!("\n\n");
        println!(r#"curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "{}""#, trs.serialize().hex());
        println!("\n");
    }



    #[test]
    fn test1() {

        let mut act2 = ContractMainCall::new();
        act2.ctype = Uint1::from(1);
        act2.codes = BytesW2::from(hex::decode("4e010101414b8059f00000").unwrap()).unwrap();
        // print
        // curl_trs(vec![Box::new(act2)]);

    
    }



    #[test]
    fn test2() {

        use Bytecode::*;

        let mut syscal1 = ContractAbstCall::new();
        syscal1.sign = Fixed1::from([5]); // PermitHAC   : 5
        syscal1.cdty = Fixed1::from([0]);
        let codes = vec![hex::decode("0601434e03").unwrap(), 
            150000000u32.to_be_bytes().to_vec(),
            hex::decode("437cec").unwrap()
        ].concat();
        syscal1.code = BytesW2::from(codes).unwrap(); // return true
        // amt < 1 zhu
        let mut syscal2 = ContractAbstCall::new();
        syscal2.sign = Fixed1::from([15]); // PayableHAC   : 15
        syscal2.cdty = Fixed1::from([0]);
        syscal2.code = BytesW2::from(hex::decode("070143480c437bEC").unwrap()).unwrap(); // return true
        // height > 12
        let mut usrfun1 = ContractClientFunc::new();
        usrfun1.sign = Fixed4::from(calc_func_sign("testadd"));
        usrfun1.cdty = Fixed1::from([0b10000000]);
        usrfun1.code = BytesW2::from(build_codes!(
            CU16 DUP ADD RET
        )).unwrap(); /* a = a + a; return a */
        let mut csto = ContractSto::new();
        csto.sytmcalls.push(syscal1).unwrap();
        csto.sytmcalls.push(syscal2).unwrap();
        csto.userfuncs.push(usrfun1).unwrap();
        let mut act2 = ContractDeploy::new();
        act2.contract = csto;
        act2.protocol_fee = Amount::coin(6, 245);

        // print
        curl_trs(vec![Box::new(act2)]);

    
    }

    #[test]
    fn test3() {

        let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        let caddr = ContractAddress::calculate(&addr, &Uint4::default());

        println!("ContractAddress: {}", caddr.readable());

        let cadr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();
        assert!(caddr == ContractAddress::from_addr(cadr).unwrap());


        let mut act = AssetCreate::new();
        act.metadata.issuer = cadr;
        act.metadata.serial = Fold64::from(32).unwrap();
        act.metadata.supply = Fold64::from(20_00000000_0000).unwrap();
        act.metadata.decimal = Uint1::from(4);
        act.metadata.ticket = BytesW1::from(b"USDT".to_vec()).unwrap();
        act.metadata.name = BytesW1::from(b"USD Tether".to_vec()).unwrap();
        curl_trs(vec![Box::new(act)]);

        let mut act = HacToTrs::new();
        act.to = AddrOrPtr::from_addr(cadr);
        act.hacash = Amount::mei(2);
        curl_trs(vec![Box::new(act)]);
        
        let mut act = HacFromTrs::new();
        act.from = AddrOrPtr::from_addr(cadr);
        act.hacash = Amount::mei(2);
        curl_trs(vec![Box::new(act)]);
        
    
        let mut act = HacFromTrs::new();
        act.from = AddrOrPtr::from_addr(cadr);
        act.hacash = Amount::mei(1);
        curl_trs(vec![Box::new(act)]);
        
    
    }



    #[test]
    fn test4() {

        /*
            123456789ABCDEFGHJKLMNP QRSTUVWXYZ abcdefghijk mno pqrstuvwxyz
        */

        for i in 0..=255 {
            let mut adr = [i; 21];
            adr[0] = 1;
            // println!("- ---------  addr: {}", Account::to_readable(&adr));
        }
        let addr = Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap();
        println!("++++++++ addr: {}", ContractAddress::calculate(&addr, &Uint4::from(1)).readable());

    }


    #[test]
    fn test5() {

        let adr = Address::from_readable("1EuGe2GU8tDKnHLNfBsgyffx66buK7PP6g").unwrap();

        let mut act = HacToTrs::new();
        act.to = AddrOrPtr::from_addr(adr);
        act.hacash = Amount::mei(4);

        
        curl_trs(vec![Box::new(act.clone())]);


    }


}


/*
http://127.0.0.1:8088/query/contract/sandboxcall?contract=VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa&funcname=testadd&param=0005&retabi=[res:U2]



curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006763a6a400e63c33a796b3032ce6b856f68fccf06608d9ed18f501020001007a000000000000000000000000000000020501000c0601434e0308f0d180437cec0f010008070143480c437bec0001e3b674a0000004415080ec0000000000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263ce353a2453613ae12a8ccf01699dc96194b3c095a2951b80e4ffb8ebe322a711206ed8661e9bb6655d40faed636b87bc2de0638f1612d94a091a188ae8e4592200400"



curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "030067b7227000e63c33a796b3032ce6b856f68fccf06608d9ed18f501020001000100987c51c474678706bf5320bb2e22207a1ebf3f29f8010400010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c926ab4ea4ebc23b9a3e56c59af593c50ab5ae8313ec5a1f17deda0ffe31a36b94b5fda6e5696d1734a9250fc65fc2d5896e31c2b1b2e1403f62ca00a2cc328280400"
curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "030067b7227a00e63c33a796b3032ce6b856f68fccf06608d9ed18f501020001000100987c51c474678706bf5320bb2e22207a1ebf3f29f8010400010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263cb74c618ed25773f08dab1012ada3708cbde64c56ffd4566f1ac37d62b98483b2623e344e2e8e4655380384d47f8f795112dea47b5a25c3a81c2781a8f722b4710400"
curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "030067b7228500e63c33a796b3032ce6b856f68fccf06608d9ed18f501020001000100987c51c474678706bf5320bb2e22207a1ebf3f29f8010400010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c7730242d72cc646272fae2bd21150c1ac59e82f4052285521ddca981b376498b52edfcea116af2d9680755b8728172ea946fd87d459cc525c3d5a47fcbe45c7a0400"




*/