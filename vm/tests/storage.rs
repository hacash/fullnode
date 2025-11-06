
#[cfg(test)]
#[allow(unused)]
mod storage {

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
    fn sto1() {

        let f1 = r##"
            param { rent, num }
            assert num is not nil
            var k = "num"
            storage_save(k, num)
            storage_rent(k, rent)
            end
        "##;

        let f3 = r##"
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
            var ctxadr = buf_left_drop(4, balance(context_address()))
            let tt_sat = buf_left(8, ctxadr) as u64
            let tt_zhu = hac_to_zhu(buf_left_drop(8, ctxadr))
            storage_save(tt_k,   2000000)
            return [tt_shares + 1000000, tt_sat + 100000000, tt_zhu + 10000000000]
        "##;

        let f2 = r##"
            param { addr, shares }
            // get total
            var tt_shares $2
            var tt_sat    $3
            var tt_zhu    $4
            unpack_list(self.total(), 2)
            var lq_k = addr ++ "_shares"
            var my_shares = storage_load(lq_k)
            if  my_shares is nil {
                my_shares = 50000 as u64
                storage_save(lq_k, my_shares)
            }
            assert shares <= my_shares
            assert tt_shares>0 && my_shares <= tt_shares
            var spx = 100000000 as u128
            var my_per = (shares as u128) * spx / tt_shares
            var my_sat = my_per * tt_sat / spx
            var my_zhu = my_per * tt_zhu / spx
            // print shares
            // print my_shares
            // print tt_shares
            // print my_per
            // print my_sat
            // print my_zhu
            assert my_sat>0 && my_zhu>0
            memory_put("out_sat", my_sat)
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
        "##;

        // VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        Contract::new()
        .func(Func::new("total").public().fitsh(f3).unwrap())
        .func(Func::new("sto1").public().fitsh(f2).unwrap())
        .testnet_deploy_print_by_nonce("12:244", 0);
    













    }

}