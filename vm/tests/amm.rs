



mod common;


#[cfg(test)]
#[allow(unused)]
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
        use vm::ir::*;

        println!("\n{}\n", lang_to_bytecode(r##"
            var foo = (1 + 2) * 3 * (4 * 5) / (6 / (7 + 8))
        "##).unwrap().bytecode_print(true).unwrap());

    }


    #[test]
    fn deploy() {
        use vm::ir::*;

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

        println!("\n{}\n", payable_sat.ircode_print(true).unwrap());


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
            var in_zhu = memory_get("zhu") as u128
            assert in_zhu == zhu
            // do deposit
            self.deposit(addr, sat, zhu)
            return 0

        "##;

        let payable_hac = lang_to_ircode(&payable_hac_fitsh).unwrap();

        println!("\n{}\n", lang_to_ircode(&payable_hac_fitsh).unwrap().ircode_print(true).unwrap());
        
        /* println!("payable_hac byte code len {} : {}\n\n{}\n\n{}", 
            payable_hac.len(), 
            payable_hac.to_hex(), 
            lang_to_bytecode(&payable_hac_fitsh).unwrap().bytecode_print(true).unwrap(),
            payable_hac.ircode_print(true).unwrap()
        ); */
        

        let prepare_codes = lang_to_ircode(r##"
            // check param
            var sat  = $0
            var zhu  = $1
            var exp  = $2
            unpack_list(pick(0), 0)
            assert sat >= 1000 && zhu >= 10000
            assert block_height() < exp
            // 
            var tt_sk $2 = "total_sat"
            var tt_zk    = "total_zhu"
            var tt_sat = storage_load(tt_sk)
            if tt_sat is nil {
                return zhu // first deposit
            }
            var tt_zhu = storage_load(tt_zk)
            assert tt_zhu is not nil

            var  in_zhu = (sat as u128) * tt_zhu * 1000 / tt_sat / 997
            assert in_zhu <= zhu

            memory_put("zhu", in_zhu)

            return in_zhu
        "##).unwrap();
        println!("prepare_codes:\n{}\n{}\n", prepare_codes.ircode_print(true).unwrap(), prepare_codes.to_hex());
        let prepare_codes = convert_ir_to_bytecode(&prepare_codes).unwrap();



        let deposit_codes = lang_to_ircode(r##"
            // check param
            var addr = $0
            var sat  = $1
            var zhu  = $2
            unpack_list(pick(0), 0)
            var tt_k = "total"
            var total = storage_load(tt_k)
            var tt_shares = 0 as u128
            var tt_sat    = 0 as u128
            var tt_zhu    = 0 as u128
            if total is not nil {
                tt_shares = buf_left(16, total) as u128
                tt_sat    = buf_cut(total, 16, 16) as u128
                tt_zhu    = buf_right(16, total) as u128
            }
            tt_shares += sat as u128
            tt_sat += sat as u128
            tt_zhu += zhu as u128
            storage_save(tt_k, tt_shares ++ tt_sat ++ tt_zhu)
            // 
            var lq_k $0 = addr ++ "_shares"
            var my_shares $4 = storage_load(lq_k)
            if my_shares is nil {
                my_shares = 0 as u128
            }
            my_shares += sat as u128
            storage_save(lq_k, my_shares)
            end
        "##).unwrap();
        println!("deposit_codes:\n{}\n{}\n", deposit_codes.ircode_print(true).unwrap(), deposit_codes.to_hex());
        let deposit_codes = convert_ir_to_bytecode(&deposit_codes).unwrap();




        use vm::value::ValueTy as VT;

        let contract = Contract::new()
        // .call(Abst::new(PermitHAC).bytecode(permit_hac))
        .syst(Abst::new(PayableSAT).ircode(payable_sat).unwrap())
        .syst(Abst::new(PayableHAC).ircode(payable_hac).unwrap())
        .func(Func::new("prepare").public()
            .types(Some(VT::U64), vec![VT::U64, VT::U64, VT::U64]).bytecode(prepare_codes))
        .func(Func::new("deposit")
            .types(None, vec![VT::Addr, VT::U64, VT::U64]).bytecode(deposit_codes))
        ;
        println!("\n{} bytes:\n{}\n\n", contract.serialize().len(), contract.serialize().to_hex());
        contract.testnet_deploy_print("4:244");    

    }


    #[test]
    // fn call_recursion() {
    fn maincall() {

        use vm::action::*;
        
        let maincodes = lang_to_bytecode(r##"
            lib HacSwap = 1: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            var sat = 50000 as u64
            var zhu = HacSwap.prepare(sat, 100000, 15)
            var adr = address_ptr(1)
            transfer_sat_to(adr, sat)
            transfer_hac_to(adr, zhu_to_hac(zhu))
            end
        "##).unwrap();

        println!("{}\n", maincodes.bytecode_print(true).unwrap());
        println!("{}\n", maincodes.to_hex());

        let mut act = ContractMainCall::new();
        act.ctype = Uint1::from(0);
        act.codes = BytesW2::from(maincodes).unwrap();

        // print
        curl_trs_3(vec![Box::new(act)], "22:244");

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