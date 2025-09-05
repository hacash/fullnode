
#[allow(dead_code)]
fn execute1() {
    /*

    */
    let irnds = build_codes!(
        ALLOC 2
        PUTX 0 P0
        IRIF NEQ P1 GETX 0
            PUTX 1 P0
            IRBLOCK 0 2
                PUTX 0 P1
                PUTX 1 P1
        RET GETX 0
    );
    let codes = compile_irs_to_bytecodes(&irnds).unwrap();
    println!("{}", codes.bytecode_print(true).unwrap());
    let exec_res = execute_test_maincall(65535, codes);
    println!("exec res: {:?}", exec_res);

}


#[allow(dead_code)]
fn execute2() {
    /*

    */
    let irnds = build_codes!(
        ALLOC 1
        PUTX 0 P0
        IRWHILE GT PU8 50 GETX 0
            PUTX 0 ADD P1 GETX 0
        RET GETX 0
    );
    let codes = compile_irs_to_bytecodes(&irnds).unwrap();
    println!("{}", codes.bytecode_print(true).unwrap());
    let exec_res = execute_test_maincall(65535, codes);
    println!("exec res: {:?}", exec_res);

}