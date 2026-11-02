//! FitSH source compiler builders (`Contract`/`Func`/`Abst`): tree-building
//! helpers consumed by the `vm::fitshc` frontend. Ported from fullnodedev
//! `contract/function.rs` + `contract/contract.rs` (builder subset), adapted to
//! this workspace's macro-generated contract wire structs (`Default` instead of
//! hand-written `new()`).

use super::{ContractAbstCall, ContractSto, ContractUserFunc};
use crate::ir::{IRNode, convert_ir_to_runtime_bytecode, drop_irblock_wrap};
use crate::lang::{Syntax, Tokenizer};
use crate::rt::{
    AbstCall, CodeConf, CodeType, FnConf, FuncArgvTypes, calc_func_sign, verify_bytecodes,
};
use crate::value::{ContractAddress, ValueTy};
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
            return errf!("userfunc name cannot be empty");
        };
        if c0.is_ascii_uppercase() {
            return errf!("userfunc name '{}' cannot start with uppercase", fname);
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

    /// Attach a packed calc container, admitted against the L1 default
    /// `SpaceCap`. A sidechain with raised caps uses
    /// [`Self::rv64im_container_with_cap`].
    pub fn rv64im_container(self, c: &crate::calc::container::CalcContainer) -> Ret<Self> {
        self.rv64im_container_with_cap(c, &crate::rt::SpaceCap::new(0))
    }

    /// [`Self::rv64im_container`] admitted against `cap` — the same caps the
    /// target chain will apply at deploy, so a sidechain tool does not
    /// reject kernels its chain accepts.
    pub fn rv64im_container_with_cap(
        self,
        c: &crate::calc::container::CalcContainer,
        cap: &crate::rt::SpaceCap,
    ) -> Ret<Self> {
        let bytes = c.encode().map_err(|e| e.to_string())?;
        self.rv64im_container_bytes_with_cap(bytes, cap)
    }

    /// Attach raw calc container bytes, admitted against the L1 default
    /// `SpaceCap`.
    pub fn rv64im_container_bytes(self, bytes: Vec<u8>) -> Ret<Self> {
        self.rv64im_container_bytes_with_cap(bytes, &crate::rt::SpaceCap::new(0))
    }

    /// [`Self::rv64im_container_bytes`] admitted against `cap`.
    pub fn rv64im_container_bytes_with_cap(
        mut self,
        bytes: Vec<u8>,
        cap: &crate::rt::SpaceCap,
    ) -> Ret<Self> {
        let (c, used) =
            crate::calc::container::CalcContainer::decode(&bytes).map_err(|e| e.to_string())?;
        if used != bytes.len() {
            return errf!("calc container trailing bytes");
        }
        if c.input_max as usize > cap.call_data_size {
            return errf!("calcfunc input_max exceeds call_data_size");
        }
        if c.output_max as usize > cap.value_size {
            return errf!("calcfunc output_max exceeds HVM value_size");
        }
        let cfg = crate::calc::CalcConfig::for_space_cap(cap);
        crate::calc::verify::verify_container_with(&c, &cfg).map_err(|e| e.to_string())?;
        self.func.code_stuff.conf = Uint1::from(CodeConf::from_type(CodeType::RV64IM).raw());
        self.func.code_stuff.data = BytesW2::from(bytes)?;
        Ok(self)
    }

    /// Pack a bare-metal RISC-V ELF into a calc container, admitted against
    /// the L1 default `SpaceCap`.
    pub fn rv64im_elf(self, elf: &[u8], mem_kib: u32, input_max: u16, output_max: u16) -> Ret<Self> {
        self.rv64im_elf_with_cap(elf, mem_kib, input_max, output_max, &crate::rt::SpaceCap::new(0))
    }

    /// [`Self::rv64im_elf`] admitted against `cap`.
    pub fn rv64im_elf_with_cap(
        self,
        elf: &[u8],
        mem_kib: u32,
        input_max: u16,
        output_max: u16,
        cap: &crate::rt::SpaceCap,
    ) -> Ret<Self> {
        let cfg = crate::calc::CalcConfig::for_space_cap(cap);
        let c = crate::calc::pack_elf_with_config(
            elf,
            crate::calc::container::CALC_PROFILE_RV64IM,
            mem_kib,
            input_max,
            output_max,
            &cfg,
        )
        .map_err(|e| e.to_string())?;
        self.rv64im_container_with_cap(&c, cap)
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
