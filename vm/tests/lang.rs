// use sys::*;
// use vm::IRNode;
// use vm::rt::BytecodePrint;
// use vm::ir::IRCodePrint;
// use vm::lang::{Tokenizer, Syntax};

use vm::lang::*;



#[test]
fn t1(){
    
    // lang_to_bytecode("return 0").unwrap();

    println!("{:?}", Tokenizer::new("return 0".as_bytes()).parse());

}
