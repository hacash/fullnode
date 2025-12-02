
#[allow(unused)]
pub fn codes_verify_1() {

    let _codes = build_codes!{
        PU8 1 PBUF 4 0 0 0 0 JMPS 1 PBUF 2 0 0 END
    };


    println!("{:?}", verify_bytecodes(&_codes))

}