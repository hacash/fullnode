
#[cfg(test)]
#[allow(unused)]
mod astact {

    use field::*;
    use field::interface::*;
    use protocol::action::*;
    use vm::action::*;
    use vm::contract::curl_trs_1;

    fn addr(s: &str) -> Address {
        Address::from_readable(s).unwrap()
    }
    
    #[test]
    fn t1() {

        let adr1 = addr("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9");
        let adr2 = addr("18dekVcACnj6Tbd69SsexVMQ5KLBZZfn5K");

        let act = AstSelect::create_by(1, 3, vec![
            Box::new(HacFromTrs::create_by(adr2, Amount::mei(45))),
            Box::new(HacToTrs::create_by(adr2, Amount::mei(20))),
            Box::new(HacFromTrs::create_by(adr2, Amount::mei(15))),
            Box::new(HacFromTrs::create_by(adr2, Amount::mei(15))),
        ]);

        // let act = HacFromTrs::create_by(adr2, Amount::mei(45));

        curl_trs_1(vec![Box::new(act)]);
    }



    #[test]
    fn t_if() {

        let adr1 = addr("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9");
        let adr2 = addr("18dekVcACnj6Tbd69SsexVMQ5KLBZZfn5K");

        let cond = AstSelect::create_list(vec![
            Box::new(HacFromTrs::create_by(adr2, Amount::mei(5)))
        ]);

        let br_if = AstSelect::create_list(vec![
            Box::new(SatToTrs::create_by(adr2, Satoshi::from(100)))
        ]);

        let br_else = AstSelect::create_list(vec![
            Box::new(HacToTrs::create_by(adr2, Amount::mei(20)))
        ]);

        let act = AstIf::create_by(cond, br_if, br_else);

        curl_trs_1(vec![Box::new(act)]);


    }



}