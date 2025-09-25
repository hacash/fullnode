



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
    fn verify_codes() {

        verify_bytecodes(&build_codes!(
            PU8 1 JMPL 0 8 JMPL 0 2 RET
        )).unwrap()

    }


    #[test]
    fn deploy() {
        use vm::ir::IRNodePrint;

        /*
            VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa


        */



        let payable_hac = lang_to_ircode(r##"
            local_move(0)
            var argv = $0
            var addr = $1 
            var zhu =  $2
            addr = argv[0]
            let mei = hac_to_mei(argv[1])
            assert mei < 100
            zhu = hac_to_zhu(argv[1])
            assert zhu > 0

            var akey = $3
            var hkey = $4
            hkey = "zhu"
            akey = "addr"

            let hamt = memory_get(hkey)
            assert hamt is nil
            memory_put(hkey, zhu)

            var hadr = $5
            hadr = memory_get(akey)
            if hadr is nil {
                memory_put(akey, addr)
            } else {
                assert hadr == addr
            }

            return 0

        "##).unwrap();


        println!("payable_hac ir code len {} : {}\n{}", 
            payable_hac.len(), 
            payable_hac.to_hex(), 
            payable_hac.irnode_print(true).unwrap(),
        );




        let deposit_codes = lang_to_bytecode(r##"
            local_move(0)
            var param = $0
            var addr  = $1
            var res   = $2
            assert type_id(param) == 15
            assert type_is_list(param)
            addr = item_get(0, param)
            addr = param[3]
            assert type_is(12, addr)

            let bdt = param + addr
            res = 1 + 2
            assert bdt

            return res
        "##).unwrap();





        Contract::new()
        // .call(Abst::new(PermitHAC).bytecode(permit_hac))
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