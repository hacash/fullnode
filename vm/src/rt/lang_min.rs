use super::Bytecode;
use sys::{Ret, errf};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[repr(u8)]
pub enum OpTy {
    NOT,
    POW,
    MUL,
    DIV,
    MOD,
    ADD,
    SUB,
    BSHL,
    BSHR,
    GE,
    LE,
    GT,
    LT,
    EQ,
    NEQ,
    BAND,
    BXOR,
    BOR,
    AND,
    OR,
    CAT,
}

impl OpTy {
    pub fn level(&self) -> u8 {
        use OpTy::*;
        match self {
            NOT => 13,
            POW => 12,
            MUL | DIV | MOD => 11,
            ADD | SUB => 10,
            BSHL | BSHR => 9,
            GE | LE | GT | LT => 8,
            EQ | NEQ => 7,
            BAND => 6,
            BXOR => 5,
            BOR => 4,
            AND => 3,
            OR => 2,
            CAT => 1,
        }
    }

    pub fn from_bytecode(code: Bytecode) -> Ret<OpTy> {
        use OpTy::*;
        Ok(match code {
            Bytecode::NOT => NOT,
            Bytecode::POW => POW,
            Bytecode::MUL => MUL,
            Bytecode::DIV => DIV,
            Bytecode::MOD => MOD,
            Bytecode::ADD => ADD,
            Bytecode::SUB => SUB,
            Bytecode::BSHL => BSHL,
            Bytecode::BSHR => BSHR,
            Bytecode::GE => GE,
            Bytecode::LE => LE,
            Bytecode::GT => GT,
            Bytecode::LT => LT,
            Bytecode::EQ => EQ,
            Bytecode::NEQ => NEQ,
            Bytecode::BAND => BAND,
            Bytecode::BXOR => BXOR,
            Bytecode::BOR => BOR,
            Bytecode::AND => AND,
            Bytecode::OR => OR,
            Bytecode::CAT => CAT,
            _ => return errf!("cannot find OpTy {:?}", code),
        })
    }
}
