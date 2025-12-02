
#[cfg(test)]
#[allow(unused)]
mod hrc20 {

    use field::*;
    use field::interface::*;

    use vm::ir::*;
    use vm::rt::*;
    use vm::lang::*;
    use vm::contract::*;

    fn addr(s: &str) -> Address {
        Address::from_readable(s).unwrap()
    }

    #[test]
    fn test1() {
        let sss = r##"
            var addr = tx_main_address()
            var cur_hei = block_height()
            // check time
            var phk = "phk"
            var prev_hei = storage_load(phk)
            if prev_hei is nil {
                prev_hei = 0 as u64
            }
            assert cur_hei > prev_hei
            // release tokens
            var bls = self.balance_of(addr)
            let bk = "b_" ++ addr
            storage_save(bk, bls + 1000000000) // give 10 token
            storage_save(phk, cur_hei)
            return 0
        "##;
        println!("{:?}", Tokenizer::new(sss.as_bytes()).parse())

    }

    #[test]
    fn deploy() {

        // VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        Contract::new()

        // info
        .func(Func::new("info").public().fitsh(r##"
            var ary1 = list[1, 2, 3, 4]
            var inf2 = map{
                "symbol":  "THT",
                "name":    "Test HRC20 Token",
                "decimals": 8 as u8,
            }
            return inf2
            /* 
                var infos = new_map()
                insert(infos, "name",     "Test HRC20 Token")
                insert(infos, "symbol",   "THT")
                insert(infos, "decimals", 8 as u8)
                return infos
            */
        "##).unwrap())

        // total_supply
        .func(Func::new("total_supply").public().fitsh(r##"
            let tk = "total"
            var total = storage_load(tk)
            if size(total) != 8 {
                total = 0 as u64
            }
            return total
        "##).unwrap())

        // balance_of
        .func(Func::new("balance_of").public().fitsh(r##"
            param { addr }
            assert addr is address
            let bk = "b_" ++ addr
            var balance = storage_load(bk)
            if balance is nil {
                balance = 0 as u64
            }
            return balance
        "##).unwrap())

        // transfer
        .func(Func::new("transfer").public().fitsh(r##"
            param { addr_to, amount }
            var addr_from = tx_main_address()
            // print addr_from
            return self.do_transfer(addr_from, addr_to, amount)
        "##).unwrap())

        // extend

        .func(Func::new("renewal").public().fitsh(r##"
            param{ period }
            assert period is u64
            
            let addr = tx_main_address()
            let bk = "b_" ++ addr
            let bkr = storage_rest(bk)
            assert bkr is not nil
            var new_bkr = bkr + period * 300
            storage_rent(bk, period)
            let tk = "total"
            storage_rent(tk, period)
            end
        "##).unwrap())

        .func(Func::new("offering").public().fitsh(r##"
            var addr = tx_main_address()
            var cur_hei = block_height()
            // check time
            var phk = "phk"
            var prev_hei = storage_load(phk)
            if prev_hei is nil {
                prev_hei = 0 as u64
            }
            assert cur_hei > prev_hei
            // release tokens
            var bls = self.balance_of(addr)
            let bk = "b_" ++ addr
            storage_save(bk, bls + 1000000000) // give 10 token
            storage_save(phk, cur_hei)
            end
        "##).unwrap())



        // private

        .func(Func::new("do_transfer").fitsh(r##"
            param { addr_from, addr_to, amount }

            // print 3

            assert addr_from is address
            assert addr_to is address
            assert amount is u64
            // print 4
            // check all addr is private key
            assert 0 == (buf_left(1, addr_from) + buf_left(1, addr_to))
            // print 5
            // load balance
            var bls_from = self.balance_of(addr_from)
            assert amount <= bls_from
            // update from
            bls_from -= amount
            var bk_from = "b_" ++ addr_from
            if bls_from > 0 {
                storage_save(bk_from, bls_from)
            } else {
                storage_del(bk_from)
            }
            // print 6
            // update to
            var bls_to = self.balance_of(addr_to)
            bls_to += amount
            var bk_to = "b_" ++ addr_to
            if bls_to > 0 {
                storage_save(bk_to, bls_to)
            } else {
                storage_del(bk_to)
            }
            // print 7
            // finish
            end
        "##).unwrap())
        .testnet_deploy_print_by_nonce("12:244", 0);
    


    }


    #[test]
    fn maincall() {


        // main call
        Maincall::new().fitsh(r##"
            lib C = 1
            // print 1
            C.offering()
            // print 2
            var a1 = 1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9
            var a2 = 18dekVcACnj6Tbd69SsexVMQ5KLBZZfn5K
            C.transfer(18dekVcACnj6Tbd69SsexVMQ5KLBZZfn5K, 10000 as u64)
            // print 100
            print C.balance_of(a1)
            // print 101
            print C.balance_of(a2)
            // print 102
            end
        "##).unwrap()
        .testnet_call_print("8:244");


        

    }

}

