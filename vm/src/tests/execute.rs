
#[allow(dead_code)]
pub fn execute1() {
    /*

    */
    let irnds = build_codes!(
        ALLOC 2
        PUT 0 P0
        IRIF NEQ P1 GET 0
            PUT 1 P0
            IRBLOCK 0 2
                PUT 0 P1
                PUT 1 P1
        RET GET 0
    );
    let codes = convert_irs_to_bytecodes(&irnds).unwrap();
    println!("{}", codes.bytecode_print(true).unwrap());
    let exec_res = execute_test_maincall(65535, codes);
    println!("exec res: {:?}", exec_res);

}


#[allow(dead_code)]
pub fn execute2() {
    /*

    */
    let irnds = build_codes!(
        ALLOC 1
        PUT 0 P0
        IRWHILE GT PU8 50 GET 0
            PUT 0 ADD P1 GET 0
        RET GET 0
    );
    let codes = convert_irs_to_bytecodes(&irnds).unwrap();
    println!("{}", codes.bytecode_print(true).unwrap());
    let exec_res = execute_test_maincall(65535, codes);
    println!("exec res: {:?}", exec_res);

}


#[allow(dead_code)]
pub fn execute3() {

    let permithac_codes = lang_to_bytecodes(r##"
        local_move(0)
        var argv = $0
        var mei  = $1
        argv = buf_left_drop(21, argv)
        mei = amount_to_mei(argv)
        return choise(mei<=4, true, false)
    "##).unwrap();

    let argv = Value::Compo(CompoItem::list(VecDeque::from([
        Value::Addr(Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap()),
        Value::Bytes(Amount::from("6:248").unwrap().serialize()),
    ])));

    println!("{}", permithac_codes.bytecode_print(true).unwrap());
    let exec_res = execute_test_with_argv(65535, permithac_codes, Some(argv));
    println!("exec res: {:?}", exec_res);

}


#[allow(dead_code)]
pub fn execute4() {
    let codes = lang_to_bytecodes(r##"
        throw "1"
    "##).unwrap();

    println!("{}", codes.bytecode_print(true).unwrap());
    let exec_res = execute_test_maincall(65535, codes);
    println!("exec res: {:?}", exec_res);

}

#[allow(dead_code)]
pub fn execute5() {

    let permithac_codes = lang_to_bytecodes(r##"
        local_move(0)
        var argv = $0
        var mei  = $1
        mei = hac_to_mei(argv[1])
        if mei > 5 {
            return 0
        } else {
            return mei
        }
        return choise(true, false, mei<=4)
    "##).unwrap();

    let argv = Value::Compo(CompoItem::list(VecDeque::from([
        Value::Addr(Address::from_readable("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").unwrap()),
        Value::Bytes(Amount::from("6:248").unwrap().serialize()),
    ])));

    println!("{}", permithac_codes.bytecode_print(true).unwrap());
    let exec_res = execute_test_with_argv(65535, permithac_codes, Some(argv));
    println!("exec res: {:?}", exec_res);

}
