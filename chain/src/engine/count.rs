

#[allow(dead_code)]
fn dev_count_switch_print(idx: usize, db: &DiskKV) {
    if idx == 1 { count_all_address_balance(db) }
}





/************************************/





#[allow(dead_code)]
fn count_all_address_balance(db: &DiskKV) {

    let mut blsnum: usize = 0;
    let mut hacnum: usize = 0;
    let mut satnum: usize = 0;
    let mut dianum: usize = 0;

    db.for_each(&mut |k, v|{
        if k[0] != 11 {
            return true
        }
        let _adr = Address::must(&k[1..]);
        let bls = Balance::must(&v);
        let havhac = bls.hacash.not_zero();
        let havsat = bls.satoshi.not_zero();
        let havdia = bls.diamond.not_zero();
        let hav = havhac || havsat || havdia;
        if !hav {
            return true
        }
        if havhac { hacnum += 1 }
        if havsat { satnum += 1 }
        if havdia { dianum += 1 }

        blsnum += 1;
        true
    });

    println!("--------\n---- count_all_address_balance Total Address: {}, HAC: {}, BTC: {}, HACD: {}\n--------", 
        blsnum, hacnum, satnum, dianum
    );

}
