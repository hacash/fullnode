use std::any::Any;
use std::iter;

use dyn_clone::{DynClone, clone_trait_object};
use field::{Address, Encode};
use sys::{Rerr, Ret, errf};

use crate::rt::Bytecode::*;
use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::*;

pub trait IRNode: DynClone {
    fn bytecode(&self) -> u8;
    fn hasretval(&self) -> bool;
    fn subs(&self) -> usize {
        0
    }
    fn level(&self) -> u8 {
        0
    }

    #[cfg(feature = "execute")]
    fn checkretval(&self) -> Rerr {
        if self.hasretval() {
            return Ok(());
        }
        let c = Bytecode::try_from_u8(self.bytecode())?;
        errf!(
            "IR build error: Inst {:?} ({}) does not have a return value",
            c,
            c.metadata().intro
        )
    }
    fn print(&self) -> String {
        match Bytecode::try_from_u8(self.bytecode()) {
            Ok(c) => format!("{:?}", c),
            Err(_) => format!("bytecode({})", self.bytecode()),
        }
    }

    #[cfg(feature = "execute")]
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        let mut buf = Vec::new();
        self.codegen_into(&mut buf)?;
        Ok(buf)
    }

    #[cfg(feature = "execute")]
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        buf.push(self.bytecode());
        Ok(())
    }

    #[cfg(feature = "execute")]
    fn serialize(&self) -> Vec<u8> {
        vec![]
    }

    #[cfg(feature = "execute")]
    fn is_serialization_elided(&self) -> bool {
        false
    }
    fn as_any(&self) -> &dyn Any {
        unimplemented!()
    }

    #[cfg(feature = "execute")]
    fn as_any_mut(&mut self) -> &mut dyn Any {
        unimplemented!()
    }
}

clone_trait_object!(IRNode);

impl std::fmt::Debug for dyn IRNode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{{IRNode}}")
    }
}

include!("node.rs");
include!("parse.rs");
#[cfg(feature = "execute")]
include!("compile.rs");
#[cfg(feature = "execute")]
include!("build.rs");

// Helper functions - moved from helper.rs for public export
pub fn push_empty() -> Box<dyn IRNode> {
    mk_empty()
}

pub fn push_nil() -> Box<dyn IRNode> {
    use Bytecode::*;
    push_inst(PNIL)
}

fn mk_empty() -> Box<dyn IRNode> {
    Box::new(IRNodeEmpty {})
}

fn mk_leaf(hrtv: bool, inst: Bytecode, text: String) -> Box<dyn IRNode> {
    Box::new(IRNodeLeaf { hrtv, inst, text })
}

fn mk_p1(hrtv: bool, inst: Bytecode, para: u8, text: String) -> Box<dyn IRNode> {
    Box::new(IRNodeParam1 {
        hrtv,
        inst,
        para,
        text,
    })
}

fn mk_p2(hrtv: bool, inst: Bytecode, para: [u8; 2]) -> Box<dyn IRNode> {
    Box::new(IRNodeParam2 { hrtv, inst, para })
}

fn mk_ps(hrtv: bool, inst: Bytecode, para: Vec<u8>) -> Box<dyn IRNode> {
    Box::new(IRNodeParams { hrtv, inst, para })
}

fn mk_1(hrtv: bool, inst: Bytecode, subx: Box<dyn IRNode>) -> Box<dyn IRNode> {
    Box::new(IRNodeSingle { hrtv, inst, subx })
}

fn mk_1p(hrtv: bool, inst: Bytecode, para: u8, subx: Box<dyn IRNode>) -> Box<dyn IRNode> {
    Box::new(IRNodeParam1Single {
        hrtv,
        inst,
        para,
        subx,
    })
}

fn mk_ps1(hrtv: bool, inst: Bytecode, para: Vec<u8>, subx: Box<dyn IRNode>) -> Box<dyn IRNode> {
    Box::new(IRNodeParamsSingle {
        hrtv,
        inst,
        para,
        subx,
    })
}

fn mk_2(
    hrtv: bool,
    inst: Bytecode,
    subx: Box<dyn IRNode>,
    suby: Box<dyn IRNode>,
) -> Box<dyn IRNode> {
    Box::new(IRNodeDouble {
        hrtv,
        inst,
        subx,
        suby,
    })
}

fn mk_4(
    hrtv: bool,
    inst: Bytecode,
    subx: Box<dyn IRNode>,
    suby: Box<dyn IRNode>,
    subz: Box<dyn IRNode>,
    subw: Box<dyn IRNode>,
) -> Box<dyn IRNode> {
    Box::new(IRNodeQuad {
        hrtv,
        inst,
        subx,
        suby,
        subz,
        subw,
    })
}

pub fn push_inst_noret(inst: Bytecode) -> Box<dyn IRNode> {
    mk_leaf(false, inst, s!(""))
}

pub fn push_inst(inst: Bytecode) -> Box<dyn IRNode> {
    mk_leaf(true, inst, s!(""))
}

pub fn push_single(inst: Bytecode, subx: Box<dyn IRNode>) -> Box<dyn IRNode> {
    mk_1(true, inst, subx)
}

pub fn push_single_noret(inst: Bytecode, subx: Box<dyn IRNode>) -> Box<dyn IRNode> {
    mk_1(false, inst, subx)
}

pub fn push_single_p1(inst: Bytecode, para: u8, subx: Box<dyn IRNode>) -> Box<dyn IRNode> {
    mk_1p(false, inst, para, subx)
}

pub fn push_single_p1_hr(
    hrtv: bool,
    inst: Bytecode,
    para: u8,
    subx: Box<dyn IRNode>,
) -> Box<dyn IRNode> {
    mk_1p(hrtv, inst, para, subx)
}

pub fn push_double(inst: Bytecode, subx_inst: Bytecode, suby_inst: Bytecode) -> Box<dyn IRNode> {
    mk_2(false, inst, push_inst(subx_inst), push_inst(suby_inst))
}

pub fn push_double_box(
    inst: Bytecode,
    subx: Box<dyn IRNode>,
    suby: Box<dyn IRNode>,
) -> Box<dyn IRNode> {
    mk_2(false, inst, subx, suby)
}

pub fn push_quad_box(
    inst: Bytecode,
    subx: Box<dyn IRNode>,
    suby: Box<dyn IRNode>,
    subz: Box<dyn IRNode>,
    subw: Box<dyn IRNode>,
) -> Box<dyn IRNode> {
    mk_4(false, inst, subx, suby, subz, subw)
}

pub fn push_local_get(i: u8, text: String) -> Box<dyn IRNode> {
    use Bytecode::*;
    match i {
        0 => mk_leaf(true, GET0, text),
        1 => mk_leaf(true, GET1, text),
        2 => mk_leaf(true, GET2, text),
        3 => mk_leaf(true, GET3, text),
        _ => mk_p1(true, GET, i, text),
    }
}

pub fn push_num(n: u128) -> Box<dyn IRNode> {
    use Bytecode::*;
    macro_rules! push_uint {
        ($n:expr, $t:expr) => {{
            let buf = buf_drop_left_zero(&$n.to_be_bytes(), 0);
            let numv = iter::once(buf.len() as u8).chain(buf).collect::<Vec<_>>();
            mk_1(true, $t, mk_ps(true, PBUF, numv))
        }};
    }

    match n {
        0 => push_inst(P0),
        1 => push_inst(P1),
        2 => push_inst(P2),
        3 => push_inst(P3),
        4..=255 => mk_p1(true, PU8, n as u8, s!("")),
        256..=65535 => mk_p2(true, PU16, (n as u16).to_be_bytes()),
        65536..=4294967295 => push_uint!(n, CU32),
        4294967296..=18446744073709551615 => push_uint!(n, CU64),
        _ => push_uint!(n, CU128),
    }
}

pub fn push_addr(a: Address) -> Box<dyn IRNode> {
    use Bytecode::*;
    let mut para = Vec::with_capacity(1 + a.size());
    para.push(Address::SIZE as u8);
    a.encode_to(&mut para);
    push_single_p1_hr(true, CTO, ValueTy::Address as u8, mk_ps(true, PBUF, para))
}

pub fn push_bytes(b: &Vec<u8>) -> Ret<Box<dyn IRNode>> {
    use Bytecode::*;
    let bl = b.len();
    if bl == 0 {
        return Ok(push_inst(PNBUF));
    }
    if bl > u16::MAX as usize {
        return errf!("bytes data too long");
    }
    let isl = bl > u8::MAX as usize;
    let inst = maybe!(isl, PBUFL, PBUF);
    let size = maybe!(isl, (bl as u16).to_be_bytes().to_vec(), vec![bl as u8]);
    let para = std::iter::empty()
        .chain(size)
        .chain(b.clone())
        .collect::<Vec<_>>();
    Ok(mk_ps(true, inst, para))
}

pub fn push_user_invoke(call: CallSpec, subx: Box<dyn IRNode>) -> Ret<Box<dyn IRNode>> {
    if !matches!(call, CallSpec::Invoke { .. }) {
        return errf!("invoke call spec required");
    }
    let (inst, para) = encode_user_call_site(call);
    Ok(mk_ps1(true, inst, para, subx))
}

pub fn push_user_splice(call: CallSpec, subx: Box<dyn IRNode>) -> Ret<Box<dyn IRNode>> {
    if !matches!(call, CallSpec::Splice { .. }) {
        return errf!("splice call spec required");
    }
    let (inst, para) = encode_user_call_site(call);
    Ok(mk_ps1(false, inst, para, subx))
}

pub fn drop_irblock_wrap(mut serialized: Vec<u8>) -> Ret<Vec<u8>> {
    if serialized.len() < 3 {
        return errf!("invalid serialized IR: length {} < 3", serialized.len());
    }
    let op = serialized[0];
    if op != Bytecode::IRBLOCK as u8 && op != Bytecode::IRBLOCKR as u8 {
        return errf!(
            "invalid serialized IR: expected IRBLOCK/IRBLOCKR header, got opcode {}",
            op
        );
    }
    Ok(serialized.split_off(3))
}
