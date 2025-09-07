
// use super::rt::Bytecode::*;
use super::ir::*;


fn test_irnds() -> Vec<u8> {
    /*
    local_var_alloc(1);
    local_var_set(0, 1u8);
    return local_var_get(0);
    */
    build_codes!(
        ALLOC 2
        PUTX 0 P0
        IRWHILE GT PU8 100 GETX 0
            PUTX 0 ADD P1 GETX 0 
        PUTX 0
            CALLINR 0 0 0 0 GET 
                EXTACTION 1 GETX 0
        PUTX 1
            EXTACTION 1 GETX 0
        IRIF EQ P1 GETX 0
            PUTX 1 P0
            IRBLOCK 0 2
                PUTX 0 P1
                PUTX 1 P1
        GETX 0
        GETX 0
        GETX 0
        GETX 0
        RET 
            GETX 1
    )
}


#[allow(dead_code)]
fn codegen1() {
    let irbts = test_irnds();
    println!("irbtx len = {}", irbts.len());
    let irnds = parse_ir_block(&irbts, &mut 0).unwrap();
    println!("{:?}", irnds);
    println!("```ir byte codes:\n{}\n```", irnds.print("    ", 0, false));
    println!("```ir desc codes:\n{}\n```", irnds.print("    ", 0, true));
    let codes = irnds.codegen().unwrap();
    println!("codes len = {}, {}", codes.len(), codes.bytecode_print(true).unwrap());
}


#[allow(dead_code)]
fn codegen2() {
    let irbts = test_irnds();
    let sy_time = SystemTime::now();
    let mut res: u8 = 0;
    let mut num = 0;
    while num < 65535 / irbts.len() {
        num += 1;
        let irnds = parse_ir_block(&irbts, &mut 0).unwrap();
        let codes = irnds.codegen().unwrap();
        res = *codes.last().unwrap();
    }
    let us_time = SystemTime::now().duration_since(sy_time).unwrap().as_millis();
    println!("res codes last {}, use time {} millis", res, us_time);
}


