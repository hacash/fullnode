
use std::time::SystemTime;
use super::rt::Bytecode::*;


fn test_codes() -> Vec<u8> {
    build_codes!(
        P1 PU8 9 ADD CU32 PBUF 2 56 56 DUP POP DUP DUP DUP DUP POPN 4 CU64 CBUF LEFT 1 CU8 CU32 MUL CBUF CU128 PBUF 3 3 4 5 CU128 ADD PU16 3 3 GT POP
    )
}

fn benchmark(appcodes: Vec<u8>) {

    // let codes = vec![appcodes].concat();
    let codes = vec![test_codes(), appcodes].concat();
    let cdlen = codes.len();

    let sy_time = SystemTime::now();
    let exec_res = execute_test_maincall(65535 / 3 * 499, codes);
    let us_time = SystemTime::now().duration_since(sy_time).unwrap().as_millis();

    println!("use time: {} millis, codes sizes: {}, exec res: {:?}", us_time, cdlen, exec_res);

}

#[allow(dead_code)]
pub fn benchmark1() {
    benchmark(build_codes!( RET ));
}

#[allow(dead_code)]
pub fn benchmark2() {
    benchmark(build_codes!(
        P1 P0 ADD DUP POP POP JMPL 0 0 RET
    ));
}


#[allow(dead_code)]
pub fn benchmark3() {

    let codes = lang_to_bytecode(r##"
        var foo = $0
        var fk = "foo"
        var vn = 100
        while true {
            global_put(fk, 1)
            foo = global_get(fk)
            foo = foo + vn
            foo = foo - vn
            foo *= 32
            foo /= 32
            foo = 1 * 2 * 3 / 4 + 5 - 6 + 7 + 8 - 9
            if foo > 1 {
                foo = global_get(fk)
            } else {
                foo = 1
            }
        }
    "##).unwrap();
    
    // run
    benchmark(codes)
}