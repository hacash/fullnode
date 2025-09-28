



mod common;


#[cfg(test)]
mod amm {
    use field::*;
    use field::interface::*;
    use protocol::action::*;

    use vm::*;
    // use vm::ir::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::rt::Bytecode::*;
    use vm::rt::AbstCall::*;
    use vm::contract::*;

    #[test]
    fn op() {
        use vm::ir::IRNodePrint;

        println!("\n{}\n", lang_to_bytecode(r##"
            var foo = (1 + 2) * 3 * (4 * 5) / (6 / (7 + 8))
        "##).unwrap().bytecode_print(true).unwrap());

    }


    #[test]
    fn deploy() {
        use vm::ir::IRNodePrint;

        /*
            VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa


        */


        let payable_sat_fitsh = r##"
            var addr = $0
            var sat =  $1
            unpack_list(pick(0), 0)
            assert sat >= 1000
            var akey = $3
            akey = "addr"
            let adr = memory_get(akey)
            assert adr is nil
            memory_put(akey, addr)
            memory_put("sat", sat)
            return 0
        "##;

        let payable_sat = lang_to_ircode(&payable_sat_fitsh).unwrap();

        println!("\n{}\n", payable_sat.irnode_print(true).unwrap());


        let payable_hac_fitsh = r##"
            // HAC Pay
            //
            //
            /* 
                dddddd   /8 "" 
            */ 

            var addr = $0
            var amt  = $1
            unpack_list(pick(0), 0)
            var zhu $1 = hac_to_zhu(amt) as u128
            assert zhu >= 10000
            // SAT
            var sat = memory_get("sat") as u128
            assert sat is not nil

            let akey = "addr"
            let adr = memory_get(akey)
            assert adr == addr

            var zhu_key = "zhu"


            return 0


        "##;

        let payable_hac = lang_to_ircode(&payable_hac_fitsh).unwrap();

        println!("\n{}\n", payable_hac.irnode_print(true).unwrap());
        
        /* println!("payable_hac byte code len {} : {}\n\n{}\n\n{}", 
            payable_hac.len(), 
            payable_hac.to_hex(), 
            lang_to_bytecode(&payable_hac_fitsh).unwrap().bytecode_print(true).unwrap(),
            payable_hac.irnode_print(true).unwrap()
        ); */
        

        let deposit_codes = lang_to_bytecode(r##"
            // check param
            var sat  = $0
            var zhu  = $1
            var exp  = $2
            unpack_list(pick(0), 0)
            assert sat >= 1000 && zhu >= 10000
            assert block_height() < exp
            // 
            var tt_sk $2 = "total_sat"
            var tt_hk    = "total_hac"
            var tt_sat = storage_load(tt_sk)
            if tt_sat is nil {
                return zhu // first deposit
            }
            var tt_hac = storage_load(tt_hk)
            assert tt_hac is not nil

            let in_hac = (sat as u128) * tt_hac * 1000 / tt_sat / 997
            
            return in_hac
        "##).unwrap();


        println!("\n{}\n{}\n", deposit_codes.bytecode_print(true).unwrap(), deposit_codes.to_hex());



        Contract::new()
        // .call(Abst::new(PermitHAC).bytecode(permit_hac))
        .call(Abst::new(PayableSAT).ircode(payable_sat).unwrap())
        .call(Abst::new(PayableHAC).ircode(payable_hac).unwrap())
        .func(Func::new("deposit").bytecode(deposit_codes))
        .testnet_deploy_print("2:244");    

    }


    #[test]
    // fn call_recursion() {
    fn maincall() {

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
    fn transfer() {

        let adr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let mut act = HacToTrs::new();
        act.to = AddrOrPtr::from_addr(adr);
        act.hacash = Amount::mei(5);

        curl_trs_1(vec![Box::new(act.clone())]);

    }








}