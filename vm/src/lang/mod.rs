use std::iter;
use std::any::*;
use std::collections::*;

use sys::*;

use super::*;
use super::rt::*;
use super::ir::*;
use super::value::*;

use super::rt::Token::*;
use super::rt::TokenType::*;

use super::native::*;


pub enum ArgvMode {
    Concat,
    PackList,
}


include!{"interface.rs"}
include!{"tokenizer.rs"}
// include!{"ast.rs"}
include!{"funcs.rs"}
include!{"syntax.rs"}
include!{"test.rs"}



pub fn lang_to_irnode(langscript: &str) -> Ret<IRNodeBlock> {
    let tkr = Tokenizer::new(langscript.as_bytes());
    let tks = tkr.parse()?;
    let syx = Syntax::new(tks);
    syx.parse()
}


pub fn lang_to_ircode(langscript: &str) -> Ret<Vec<u8>> {
    let ir = lang_to_irnode(langscript)?;
    Ok(ir.serialize().split_off(3))
}


pub fn lang_to_bytecode(langscript: &str) -> Ret<Vec<u8>> {
    let ir = lang_to_irnode(langscript)?;
    let codes = ir.codegen()?;
    Ok(codes)
}





