//! FitSH source compiler builders (`Contract`/`Func`/`Abst`): tree-building
//! helpers consumed by the `vm::fitshc` frontend. Ported from fullnodedev
//! `contract/function.rs` + `contract/contract.rs` (builder subset), adapted to
//! this workspace's macro-generated contract wire structs (`Default` instead of
//! hand-written `new()`).

use crate::ir::{IRNode, convert_ir_to_runtime_bytecode, drop_irblock_wrap};
use crate::lang::{Syntax, Tokenizer};
use crate::rt::{
    AbstCall, CodeConf, CodeType, FuncArgvTypes, calc_func_sign, verify_bytecodes, FnConf,
};
use crate::value::{ContractAddress, ValueTy};
use super::{ContractAbstCall, ContractSto, ContractUserFunc};
use field::{Address, BytesW2, Fixed1, Fixed4, Uint1};
use sys::{Ret, errf};

macro_rules! define_func_codes {
    () => {
        pub fn fitsh(self, irs: &str) -> Ret<Self> {
            let tks = Tokenizer::new(irs.as_bytes());
            let sytax = Syntax::new(tks.parse()?);
            let (irnodes, _) = sytax.parse()?;
            self.irnode(irnodes)
        }

        pub fn irnode(self, irnodes: crate::IRNodeArray) -> Ret<Self> {
            // Store raw block content (without IRBLOCK/IRBLOCKR wrapper)
            let ircodes = drop_irblock_wrap(irnodes.serialize())?;
            self.ircode(ircodes)
        }

        pub fn ircode(mut self, ircodes: Vec<u8>) -> Ret<Self> {
            // ircodes should be raw block content (without IRBLOCK header)
            let cds = convert_ir_to_runtime_bytecode(&ircodes)?;
            verify_bytecodes(&cds)?;
            self.func.code_stuff.conf = Uint1::from(CodeConf::from_type(CodeType::IRNode).raw());
            self.func.code_stuff.data = BytesW2::from(ircodes)?;
            Ok(self)
        }

        pub fn bytecode(mut self, cds: Vec<u8>) -> Ret<Self> {
            verify_bytecodes(&cds)?;
            self.func.code_stuff.conf = Uint1::from(CodeConf::from_type(CodeType::Bytecode).raw());
            self.func.code_stuff.data = BytesW2::from(cds).unwrap();
            Ok(self)
        }
    };
}

#[allow(dead_code)]
pub struct Abst {
    func: ContractAbstCall,
}

#[allow(dead_code)]
impl Abst {
    pub fn new(fnsg: AbstCall) -> Self {
        let mut func = ContractAbstCall::default();
        func.sign = Fixed1::from([fnsg.uint()]);
        Self { func }
    }

    define_func_codes! {}
}

#[allow(dead_code)]
pub struct Func {
    func: ContractUserFunc,
}

#[allow(dead_code)]
impl Func {
    pub fn new(fname: &str) -> Ret<Self> {
        let Some(c0) = fname.as_bytes().first().copied() else {
            return errf!("userfunc name cannot be empty")
        };
        if c0.is_ascii_uppercase() {
            return errf!("userfunc name '{}' cannot start with uppercase", fname)
        }
        let mut func = ContractUserFunc::default();
        func.sign = Fixed4::from(calc_func_sign(fname));
        Ok(Self { func })
    }

    define_func_codes! {}

    pub fn external(mut self) -> Self {
        let fc1 = [self.func.fncnf[0] | FnConf::External as u8];
        self.func.fncnf = Fixed1::from(fc1);
        self
    }

    pub fn types(mut self, ret: Option<ValueTy>, params: Vec<ValueTy>) -> Self {
        self.func.pmdf = FuncArgvTypes::from_types(ret, params).unwrap();
        self
    }
}

/// Contract tree being assembled by the FitSH compiler frontend.
#[derive(Clone)]
pub struct Contract {
    argv: BytesW2,
    ctrt: ContractSto,
}

impl Contract {
    pub fn new() -> Self {
        Self {
            argv: BytesW2::default(),
            ctrt: ContractSto::default(),
        }
    }

    pub fn lib(mut self, a: Address) -> Self {
        let adr = ContractAddress::from_addr(a).unwrap();
        self.ctrt.library.push(adr).unwrap();
        self
    }

    pub fn inh(mut self, a: Address) -> Self {
        let adr = ContractAddress::from_addr(a).unwrap();
        self.ctrt.inherit.push(adr).unwrap();
        self
    }

    pub fn syst(mut self, a: Abst) -> Self {
        self.ctrt.abstcalls.push(a.func).unwrap();
        self
    }

    pub fn func(mut self, a: Func) -> Self {
        self.ctrt.userfuncs.push(a.func).unwrap();
        self
    }

    pub fn argv(mut self, a: Vec<u8>) -> Self {
        self.argv = BytesW2::from(a).unwrap();
        self
    }

    pub fn into_sto(self) -> ContractSto {
        self.ctrt
    }
}
