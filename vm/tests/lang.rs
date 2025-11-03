// use sys::*;
// use vm::IRNode;
// use vm::rt::BytecodePrint;
// use vm::ir::IRCodePrint;
// use vm::lang::{Tokenizer, Syntax};

use vm::lang::*;
use vm::rt::*;
use vm::ir::*;



#[test]
fn t1(){
    
    // lang_to_bytecode("return 0").unwrap();

    let payable_hac_fitsh = r##"
        // var addr = 1
        self.deposit(1)
        end
    "##;

    let ircodes = lang_to_ircode(&payable_hac_fitsh).unwrap();

    println!("\n{}\n", ircodes.bytecode_print(false).unwrap());

    let bytecodes = convert_ir_to_bytecode(&ircodes).unwrap();

    println!("\n{}\n", bytecodes.bytecode_print(false).unwrap());

        
}
