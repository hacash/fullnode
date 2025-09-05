
use std::time::SystemTime;
use super::rt::Bytecode::*;


fn test_codes() -> Vec<u8> {
    build_codes!(
        P1 PU8 9 ADD CU32 PBUF 1 56 56 DUP POP DUP DUP DUP DUP POPX 4 CU64 CBUF LEFT 1 CU8 CU32 MUL CBUF CU128 PBUF 2 3 4 5 CU128 SUB PU16 3 3 GT POP
    )
}

fn benchmark(appcodes: Vec<u8>) {

    let codes = vec![test_codes(), appcodes].concat();
    let cdlen = codes.len() * 1725;

    let sy_time = SystemTime::now();
    let exec_res = execute_test_maincall(65535, codes);
    let us_time = SystemTime::now().duration_since(sy_time).unwrap().as_millis();

    println!("use time: {} millis, codes sizes: {}, exec res: {:?}", us_time, cdlen, exec_res);

}

#[allow(dead_code)]
fn benchmark1() {
    benchmark(build_codes!( RET ));
}

#[allow(dead_code)]
fn benchmark2() {
    benchmark(build_codes!(
        POP JMPL 0 0 RET
    ));
}