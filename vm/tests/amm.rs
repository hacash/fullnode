



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
            1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9
            18dekVcACnj6Tbd69SsexVMQ5KLBZZfn5K
            VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa

        */


        let payable_sat_fitsh = r##"
            var addr = $0
            var sat =  $1
            unpack_list(pick(0), 0)
            if sat == memory_get("sell_sat") {
                return 0 // ok for sell sat
            }
            assert sat >= 1000
            var in_sat = memory_get("in_sat")
            assert sat == in_sat
            var akey = "in_addr"
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
            if amt == memory_get("buy_hac") {
                return 0 // ok for buy sat
            }
            // deal deposit
            var zhu $1 = hac_to_zhu(amt) as u128
            assert zhu >= 10000
            let in_zhu = memory_get("in_zhu") as u128
            assert zhu == in_zhu
            let akey = "in_addr"
            let adr = memory_get(akey)
            assert adr == addr
            var sat = memory_get("in_sat")
            assert sat >= 1000
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
            assert deadline >= block_height()
            assert sat >= 1000 && zhu >= 10000
            // get total
            var tt_shares = $3
            var tt_sat    = $4
            var tt_zhu    = $5
            unpack_list(self.total(nil), 3)
            // check
            var k_in_sat = "in_sat"
            var k_in_zhu = "in_zhu"
            if tt_shares == 0 ||  tt_sat == 0 || tt_zhu == 0  {
                storage_del("total")
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
            tt_shares += zhu as u64
            let tt_k = "total_shares"
            storage_save(tt_k, tt_shares)
            // 
            var lq_k $0 = addr ++ "_shares"
            var my_shares $4 = storage_load(lq_k)
            if my_shares is nil {
                my_shares = 0 as u64
            }
            my_shares += zhu as u64
            storage_save(lq_k, my_shares)
            end
        "##).unwrap();
        println!("deposit_codes:\n{}\n{}\n", deposit_codes.ircode_print(true).unwrap(), deposit_codes.to_hex());
        let deposit_codes = convert_ir_to_bytecode(&deposit_codes).unwrap();


        let withdraw_codes = lang_to_ircode(r##"
            // check param
            var addr   = $0
            var shares = $1
            unpack_list(pick(0), 0)
            // get total
            var tt_shares = $2
            var tt_sat    = $3
            var tt_zhu    = $4
            unpack_list(self.total(nil), 2)
            var lq_k = addr ++ "_shares"
            var my_shares = storage_load(lq_k)
            assert shares <= my_shares
            assert tt_shares>0 && my_shares <= tt_shares
            var my_per = (shares as u128) * 1000 / tt_shares
            var my_sat = my_per * tt_sat / 1000
            var my_zhu = my_per * tt_zhu / 1000
            assert my_sat>0 && my_zhu>0
            memory_put("out_sat", my_sat)
            // print my_zhu
            // print zhu_to_hac(my_zhu)
            memory_put("out_hac", zhu_to_hac(my_zhu))
            // update total
            tt_shares -= shares
            var tt_k = "total_shares"
            if tt_shares > 0 {
                storage_save(tt_k, tt_shares as u64)
            } else {
                storage_del(tt_k)
            }
            // update my shares
            my_shares -= shares
            if my_shares > 0 {
                storage_save(lq_k, my_shares as u64)
            } else {
                storage_del(lq_k)
            }
            // return
            var reslist = new_list()
            append(reslist, my_sat as u64)
            append(reslist, my_zhu as u64)
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
            assert deadline >= block_height()
            assert zhu>0 && min_sat>0
            // get total
            var tt_shares = $3
            var tt_sat    = $4
            var tt_zhu    = $5
            unpack_list(self.total(nil), 3)
            assert tt_shares>0 && tt_sat>0 && tt_zhu>0 
            // 0.3% fee
            var out_sat = ((tt_sat as u128) * zhu * 997 / (tt_zhu + zhu) / 1000) as u64
            // print out_sat
            assert out_sat >= min_sat
            memory_put("buy_hac", zhu_to_hac(zhu))
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
            assert deadline >= block_height()
            // get total
            var tt_shares = $3
            var tt_sat    = $4
            var tt_zhu    = $5
            unpack_list(self.total(nil), 3)
            assert tt_shares>0 && tt_sat>0 && tt_zhu>0 
            // 0.3% fee
            var out_zhu = ((tt_zhu as u128) * sat * 997 / (tt_sat + sat) / 1000) as u64
            memory_put("out_hac", zhu_to_hac(out_zhu))
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
            var out_hac $0 = memory_get(ot_k)
            // print hac
            // print out_hac
            // print hac ++ out_hac
            assert hac_to_zhu(hac) > 0 && hac == out_hac
            memory_put(ot_k, nil)
            // ok
            return 0
        
        "##).unwrap();



        let total_codes = lang_to_bytecode(r##"
            // get total
            var tt_k = "total_shares"
            var total = storage_load(tt_k)
            if total is nil {
                var exp = storage_time(tt_k)
                if exp is not nil {
                    if exp < block_height() {
                        throw "storage expire"
                    }
                }
            }
            var tt_shares = 0 as u64
            if 8 == size(total) {
                tt_shares = total as u64
            }
            var ctxadr = buf_left_drop(4, balance(context_address(nil)))
            let tt_sat = buf_left(8, ctxadr) as u64
            let tt_zhu = hac_to_zhu(buf_left_drop(8, ctxadr))
            return [tt_shares, tt_sat, tt_zhu]
        "##).unwrap();


        let shares_codes = lang_to_bytecode(r##"
            // get shares
            var lq_k = pick(0) ++ "_shares"
            var my_shares = storage_load(lq_k)
            if my_shares is nil {
                return nil
            }
            return my_shares as u128
        "##).unwrap();


        println!("shares_codes:\n{}\n{}\n", shares_codes.bytecode_print(true).unwrap(), shares_codes.to_hex());



        use vm::value::ValueTy as VT;

        let contract = Contract::new()
        .syst(Abst::new(PayableSAT).ircode(payable_sat).unwrap())
        .syst(Abst::new(PayableHAC).ircode(payable_hac).unwrap())
        .syst(Abst::new(PermitSAT).bytecode(permit_sat))
        .syst(Abst::new(PermitHAC).bytecode(permit_hac))
        .func(Func::new("prepare").public()
            .types(Some(VT::U64), vec![VT::U64, VT::U64, VT::U64]).bytecode(prepare_codes))
        .func(Func::new("deposit")
            .types(None, vec![VT::Address, VT::U64, VT::U64]).bytecode(deposit_codes))
        .func(Func::new("withdraw").public()
            .types(None, vec![VT::Address, VT::U128]).bytecode(withdraw_codes))
        .func(Func::new("buy").public()
            .types(Some(VT::U64), vec![VT::U64, VT::U64, VT::U64]).bytecode(buy_codes))
        .func(Func::new("sell").public()
            .types(Some(VT::U64), vec![VT::U64, VT::U64, VT::U64]).bytecode(sell_codes))
        .func(Func::new("total").public()
            .types(None, vec![]).bytecode(total_codes))
        .func(Func::new("shares").public()
            .types(Some(VT::U128), vec![VT::Address]).bytecode(shares_codes))
        ;
        println!("\n{} bytes:\n{}\n\n", contract.serialize().len(), contract.serialize().to_hex());
        contract.testnet_deploy_print("8:244");    


        let acc = sys::Account::create_by("123457").unwrap();
        println!("\n{}", acc.readable());

    }


    #[test]
    // fn call_recursion() {
    // 
    // function
    fn maincall_add() {

        use vm::action::*;
        
        let maincodes = lang_to_bytecode(r##"
            lib HacSwap = 1: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            var sat = 100000000 as u64 // 1 BTC
            var zhu = HacSwap.prepare(sat, 100000000000, 50) // 1k HAC
            var adr = address_ptr(1)
            // throw concat(adr, sat)
            transfer_sat_to(adr, sat)
            transfer_hac_to(adr, zhu_to_hac(zhu))
            end
        "##).unwrap();

        println!("{}\n", maincodes.bytecode_print(true).unwrap());
        println!("{}\n", maincodes.to_hex());

        let mut act = ContractMainCall::new();
        act.ctype = Uint1::from(0);
        act.codes = BytesW2::from(maincodes).unwrap();

        curl_trs_3(vec![Box::new(act)], "22:244");

    }


    #[test]
    fn maincall_remove() {

        use vm::action::*;
        
        let maincodes = lang_to_bytecode(r##"
            lib HacSwap = 1: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            var shares = 50000000000 as u64 // 500HAC
            var coins = HacSwap.withdraw(tx_main_address(), shares) // 1k HAC
            let sat = item_get(coins, 0)
            let zhu = item_get(coins, 1)
            var adr = address_ptr(1)
            transfer_sat_from(adr, sat)
            transfer_hac_from(adr, zhu_to_hac(zhu))
            end
        "##).unwrap();

        println!("{}\n", maincodes.bytecode_print(true).unwrap());
        println!("{}\n", maincodes.to_hex());

        let mut act = ContractMainCall::new();
        act.ctype = Uint1::from(0);
        act.codes = BytesW2::from(maincodes).unwrap();

        curl_trs_3(vec![Box::new(act)], "22:244");

    }


    #[test]
    fn maincall_buy() {

        use vm::action::*;
        
        let maincodes = lang_to_bytecode(r##"
            lib HacSwap = 1: VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
            var zhu = 5000000000 as u64 // 50HAC
            var sat = HacSwap.buy(zhu, 1000000, 300)
            var adr = address_ptr(1)
            transfer_sat_from(adr, sat)
            transfer_hac_to(adr, zhu_to_hac(zhu))
            end
        "##).unwrap();

        println!("{}\n", maincodes.bytecode_print(true).unwrap());
        println!("{}\n", maincodes.to_hex());

        let mut act = ContractMainCall::new();
        act.ctype = Uint1::from(0);
        act.codes = BytesW2::from(maincodes).unwrap();

        curl_trs_3(vec![Box::new(act)], "22:244");

    }



    #[test]
    fn transfer1() {

        let adr = Address::from_readable("VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa").unwrap();

        let mut act = HacToTrs::new();
        act.to = AddrOrPtr::from_addr(adr);
        act.hacash = Amount::mei(5);

        curl_trs_1(vec![Box::new(act.clone())]);

    }



    #[test]
    fn transfer2() {

        let adr = Address::from_readable("18dekVcACnj6Tbd69SsexVMQ5KLBZZfn5K").unwrap();

        let mut act1 = HacToTrs::new();
        act1.to = AddrOrPtr::from_addr(adr);
        act1.hacash = Amount::mei(15000);
        
        let mut act2 = SatToTrs::new();
        act2.to = AddrOrPtr::from_addr(adr);
        act2.satoshi = Satoshi::from(500000000);
        

        curl_trs_1(vec![Box::new(act1.clone()), Box::new(act2.clone())]);

    }








}