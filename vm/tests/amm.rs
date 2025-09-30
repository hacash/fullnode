



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
            let in_sat = memory_get("in_sat")
            assert sat == in_sat
            var akey = $3
            akey = "in_addr"
            let adr = memory_get(akey)
            assert adr is nil
            memory_put(akey, addr)
            return 0
        "##;

        let payable_sat = lang_to_ircode(&payable_sat_fitsh).unwrap();

        println!("\n{}\n", payable_sat.ircode_print(true).unwrap());


        let payable_hac_fitsh = r##"
            // HAC Pay
            var addr = $0
            var amt  = $1
            unpack_list(pick(0), 0)
            var zhu $1 = hac_to_zhu(amt) as u128
            assert zhu >= 10000
            let in_zhu = memory_get("in_zhu") as u128
            assert zhu == in_zhu
            let akey = "in_addr"
            let adr = memory_get(akey)
            assert adr == addr
            let sat = memory_get("in_sat")
            // do deposit
            memory_put("in_sat", nil)
            memory_put("in_zhu", nil)
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
            var sat      = $0
            var zhu      = $1
            var deadline = $2
            unpack_list(pick(0), 0)
            assert deadline <= block_height()
            assert sat >= 1000 && zhu >= 10000
            // 
            var k_in_sat = "in_sat"
            var k_in_zhu = "in_zhu"
            var tt_sk $2 = "total_sat"
            var tt_zk    = "total_zhu"
            var tt_sat = storage_load(tt_sk)
            if tt_sat is nil {
                memory_put(k_in_sat, sat)
                memory_put(k_in_zhu, zhu)
                return zhu // first deposit
            }
            var tt_zhu = storage_load(tt_zk)
            assert tt_zhu is not nil
            if tt_zhu == 0 {
                storage_del(tt_sk)
                storage_del(tt_zk)
                memory_put(k_in_sat, sat)
                memory_put(k_in_zhu, zhu)
                return zhu // first deposit
            }
            var in_zhu = (sat as u128) * tt_zhu / tt_sat
            assert in_zhu <= zhu
            memory_put(k_in_sat, sat)
            memory_put(k_in_zhu, in_zhu)
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
            // get total
            var tt_shares = $3
            var tt_sat    = $4
            var tt_zhu    = $5
            unpack_list(self.total(nil), 3)
            tt_shares += zhu as u128
            tt_sat    += sat as u128
            tt_zhu    += zhu as u128
            let tt_k = "total"
            storage_save(tt_k, tt_shares ++ tt_sat ++ tt_zhu)
            // 
            var lq_k $0 = addr ++ "_shares"
            var my_shares $4 = storage_load(lq_k)
            if my_shares is nil {
                my_shares = 0 as u128
            }
            my_shares += zhu as u128
            storage_save(lq_k, my_shares)
            end
        "##).unwrap();
        println!("deposit_codes:\n{}\n{}\n", deposit_codes.ircode_print(true).unwrap(), deposit_codes.to_hex());
        let deposit_codes = convert_ir_to_bytecode(&deposit_codes).unwrap();


        let withdraw_codes = lang_to_ircode(r##"
            // check param
            unpack_list(pick(0), 0)
            var addr   = $0
            var shares = $1
            var lq_k = addr ++ "_shares"
            var my_shares = storage_load(lq_k)
            assert shares <= my_shares
            // get total
            var tt_shares = $3
            var tt_sat    = $4
            var tt_zhu    = $5
            unpack_list(self.total(nil), 3)
            assert my_shares <= tt_shares
            var my_per = my_shares * 1000 / tt_shares
            var my_sat = my_per * tt_sat / 1000
            var my_zhu = my_per * tt_zhu / 1000
            // update
            tt_shares -= my_shares
            tt_sat    -= my_sat
            tt_zhu    -= my_zhu
            var tt_k = "total"
            if tt_shares > 0 {
                storage_save(tt_k, tt_shares ++ tt_sat ++ tt_zhu)
            } else {
                storage_del(tt_k)
            }
            // return
            var reslist = new_list()
            append(reslist, my_sat)
            append(reslist, my_zhu)
            return reslist
        "##).unwrap();
        println!("withdraw_codes:\n{}\n{}\n", withdraw_codes.ircode_print(true).unwrap(), withdraw_codes.to_hex());
        let withdraw_codes = convert_ir_to_bytecode(&withdraw_codes).unwrap();





        let buy_codes = lang_to_ircode(r##"
            // check param
            var zhu      = $0
            var min_sat  = $1
            var deadline = $2
            unpack_list(pick(0), 0)
            assert deadline <= block_height()
            // get total
            var tt_shares = $3
            var tt_sat    = $4
            var tt_zhu    = $5
            unpack_list(self.total(nil), 3)
            assert tt_shares>0 && tt_sat>0  && tt_zhu>0 
            // 0.3% fee
            var out_sat = (tt_zhu + zhu) * tt_sat * 997 / tt_zhu / 1000
            assert out_sat >= min_sat
            memory_put("out_sat", out_sat)
            return out_sat
        "##).unwrap();
        println!("buy_codes:\n{}\n{}\n", buy_codes.ircode_print(true).unwrap(), buy_codes.to_hex());
        let buy_codes = convert_ir_to_bytecode(&buy_codes).unwrap();


        let sell_codes = lang_to_ircode(r##"
            // check param
            var sat      = $0
            var min_zhu  = $1
            var deadline = $2
            unpack_list(pick(0), 0)
            assert deadline <= block_height()
            // get total
            var tt_shares = $3
            var tt_sat    = $4
            var tt_zhu    = $5
            unpack_list(self.total(nil), 3)
            assert tt_shares>0 && tt_sat>0 && tt_zhu>0 
            // 0.3% fee
            var out_zhu = tt_zhu * sat * 997 / (tt_sat + sat) / 1000
            memory_put("out_zhu", out_zhu)
            assert out_zhu >= min_zhu
            return out_zhu
        "##).unwrap();
        println!("sell_codes:\n{}\n{}\n", sell_codes.ircode_print(true).unwrap(), sell_codes.to_hex());
        let sell_codes = convert_ir_to_bytecode(&sell_codes).unwrap();



        let permit_sat = lang_to_bytecode(r##"
            // check param
            var addr = $0
            var sat  = $1
            unpack_list(pick(0), 0)
            var ot_k = "out_sat"
            var out_sat $0 = memory_get(ot_k)
            assert sat > 0 && sat == out_sat
            memory_put(ot_k, nil)
            // ok
            return 0
        "##).unwrap();


        let permit_hac = lang_to_bytecode(r##"
            // check param
            var addr = $0
            var hac  = $1
            unpack_list(pick(0), 0)
            var ot_k = "out_hac"
            var out_hac $0 = memory_get("out_hac")
            assert hac > 0 && hac == out_hac
            memory_put(ot_k, nil)
            // ok
            return 0
        
        "##).unwrap();



        let total_codes = lang_to_bytecode(r##"
            // get total
            var tt_k = "total"
            var total = storage_load(tt_k)
            var tt_shares = 0 as u128
            var tt_sat    = 0 as u128
            var tt_zhu    = 0 as u128
            if 3 * 64 == size(total) {
                tt_shares = buf_left(16, total)    as u128
                tt_sat    = buf_cut(total, 16, 16) as u128
                tt_zhu    = buf_right(16, total)   as u128
            }
            return [tt_shares, tt_sat, tt_zhu]
        "##).unwrap();


        println!("total_codes:\n{}\n{}\n", total_codes.bytecode_print(true).unwrap(), total_codes.to_hex());



        use vm::value::ValueTy as VT;

        let contract = Contract::new()
        .syst(Abst::new(PayableSAT).ircode(payable_sat).unwrap())
        .syst(Abst::new(PayableHAC).ircode(payable_hac).unwrap())
        .syst(Abst::new(PermitSAT).bytecode(permit_sat))
        .syst(Abst::new(PermitHAC).bytecode(permit_hac))
        .func(Func::new("prepare").public()
            .types(Some(VT::U64), vec![VT::U64, VT::U64, VT::U64]).bytecode(prepare_codes))
        .func(Func::new("deposit")
            .types(None, vec![VT::Addr, VT::U64, VT::U64]).bytecode(deposit_codes))
        .func(Func::new("withdraw").public()
            .types(None, vec![VT::Addr, VT::U128]).bytecode(withdraw_codes))
        .func(Func::new("buy").public()
            .types(Some(VT::U64), vec![VT::U64, VT::U64, VT::U64]).bytecode(buy_codes))
        .func(Func::new("sell").public()
            .types(Some(VT::U64), vec![VT::U64, VT::U64, VT::U64]).bytecode(sell_codes))
        .func(Func::new("total").public()
            .types(None, vec![]).bytecode(total_codes))
        ;
        println!("\n{} bytes:\n{}\n\n", contract.serialize().len(), contract.serialize().to_hex());
        contract.testnet_deploy_print("8:244");    

    }


    #[test]
    // fn call_recursion() {
    // 
    // function
    fn maincall_add() {

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