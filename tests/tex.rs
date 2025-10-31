

#[cfg(test)]
#[allow(unused)]
mod tex {

    use mint::action::AssetCreate;
    use sys::*;
    use field::*;
    use field::interface::*;
    use protocol::{action::*, tex::*};
    use vm::contract::*;

    /*
        1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9   123456
        18dekVcACnj6Tbd69SsexVMQ5KLBZZfn5K   123457
        VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
        
        
    */

    #[test]
    fn trs1() {

        let acc1 = Account::create_by_password("123456").unwrap();
        let acc2 = Account::create_by_password("123457").unwrap();
        let adr1 = Address::from(*acc1.address());
        let adr2 = Address::from(*acc2.address());

        let mut tex1 = TexCellAct::create_by(adr1);
        let mut tex2 = TexCellAct::create_by(adr2);

        tex1.add_cell(Box::new(CellTrsZhuGet::new(Fold64::from(100000000).unwrap()))).unwrap();
        tex1.add_cell(Box::new(CellTrsSatPay::new(Fold64::from(2).unwrap()))).unwrap();
        tex1.add_cell(Box::new(CellTrsDiaPay::new(DiamondNameListMax200::from_readable("KKKKVA,HYXYHY,UETWNK").unwrap()))).unwrap();
        tex1.add_cell(Box::new(CellTrsAssetGet::new(AssetAmt::from(5, 100).unwrap()))).unwrap();
        tex1.do_sign(&acc1).unwrap();

        tex2.add_cell(Box::new(CellTrsZhuPay::new(Fold64::from(100000000).unwrap()))).unwrap();
        tex2.add_cell(Box::new(CellTrsSatGet::new(Fold64::from(2).unwrap()))).unwrap();
        tex2.add_cell(Box::new(CellTrsDiaGet::new(DiamondNumber::from(3)))).unwrap();
        tex2.add_cell(Box::new(CellTrsAssetPay::new(AssetAmt::from(5, 100).unwrap()))).unwrap();
        tex2.do_sign(&acc2).unwrap();
        // tex2.sign.signature[1] = 1; // like sign bug

        let mut act1 = HacToTrs::new();
        act1.to = AddrOrPtr::from_addr(adr2);
        act1.hacash = Amount::mei(2);

        let mut act2 = AssetCreate::new();
        act2.metadata = AssetSmelt { 
            serial: Fold64::from(5).unwrap(), 
            supply: Fold64::from(10000).unwrap(),
            decimal: Uint1::from(2), 
            issuer: adr2, 
            ticket: BytesW1::from_str("USDT").unwrap(), 
            name: BytesW1::from_str("Teather").unwrap(), 
        };
        act2.protocol_fee = Amount::mei(1);



        // 
        curl_trs_1(vec![Box::new(act1), Box::new(act2), Box::new(tex1), Box::new(tex2)]);



    }





}