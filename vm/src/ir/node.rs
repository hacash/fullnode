use crate::rt::Bytecode;

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeEmpty {}

impl IRNode for IRNodeEmpty {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn hasretval(&self) -> bool {
        false
    }
    fn bytecode(&self) -> u8 {
        0
    }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        Ok(vec![])
    }
    fn codegen_into(&self, _buf: &mut Vec<u8>) -> VmrtRes<()> {
        Ok(())
    }
    fn is_serialization_elided(&self) -> bool {
        true
    }
    fn print(&self) -> String {
        String::new()
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeTopStackValue {}

impl IRNode for IRNodeTopStackValue {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn hasretval(&self) -> bool {
        true
    }

    // This node is an internal, codegen-only placeholder that represents: “a value is already on the top of the stack”. It must never be serialized into ircode; if it ever leaks into serialization, it would create ambiguous/shifted IR streams.
    fn bytecode(&self) -> u8 {
        0
    }

    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        Ok(vec![])
    }
    fn codegen_into(&self, _buf: &mut Vec<u8>) -> VmrtRes<()> {
        Ok(())
    }

    fn serialize(&self) -> Vec<u8> {
        panic!("IRNodeTopStackValue is codegen-only and cannot be serialized")
    }

    fn print(&self) -> String {
        "<TopStackValue>".to_string()
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeText {
    pub text: String,
}

impl IRNode for IRNodeText {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn hasretval(&self) -> bool {
        false
    }
    fn bytecode(&self) -> u8 {
        0
    }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        Ok(vec![])
    }
    fn codegen_into(&self, _buf: &mut Vec<u8>) -> VmrtRes<()> {
        Ok(())
    }
}

impl IRNodeText {
    pub fn into_text(self) -> String {
        self.text
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeLeaf {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub text: String,
}

impl IRNode for IRNodeLeaf {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        // IRBREAK / IRCONTINUE must be lowered to JMPSL by the while-loop `LoopPatch`
        // sink (compile.rs); reaching here means they leaked outside a while body — fail loudly.
        if matches!(self.inst, Bytecode::IRBREAK | Bytecode::IRCONTINUE) {
            return itr_err_fmt!(
                CompileError,
                "{:?} appeared outside a while-loop body",
                self.inst
            );
        }
        buf.push(self.bytecode());
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        // Serialized IR keeps IRBREAK/IRCONTINUE as 1-byte leaves; they are lowered to JMPSL
        // in while-loop codegen and never appear in runtime bytecode.
        vec![self.inst as u8]
    }
    fn print(&self) -> String {
        format!("{:?}", self.inst)
    }
}

impl IRNodeLeaf {
    pub fn nop() -> Self {
        Self {
            hrtv: false,
            inst: Bytecode::NOP,
            text: "".to_owned(),
        }
    }
    pub fn notext(hrtv: bool, inst: Bytecode) -> Self {
        Self {
            hrtv,
            inst,
            text: "".to_owned(),
        }
    }
    pub fn nop_box() -> Box<dyn IRNode> {
        Box::new(Self::nop())
    }
    pub fn as_text(&self) -> &String {
        &self.text
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeParam1 {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: u8,
    pub text: String,
}

impl IRNode for IRNodeParam1 {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        buf.push(self.bytecode());
        buf.push(self.para);
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        // IR serialized form is prefix-coded: [opcode][param...]
        vec![self.bytecode(), self.para]
    }
    fn print(&self) -> String {
        format!("{:?} {}", self.inst, self.para)
    }
}

impl IRNodeParam1 {
    pub fn into_text(self) -> String {
        self.text
    }
    pub fn to_text(&self) -> String {
        self.text.clone()
    }
    pub fn as_text(&self) -> &String {
        &self.text
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeParam2 {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: [u8; 2],
}

impl IRNode for IRNodeParam2 {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        buf.push(self.bytecode());
        buf.extend_from_slice(&self.para);
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        // IR serialized form is prefix-coded: [opcode][param...]
        iter::once(self.bytecode()).chain(self.para).collect()
    }
    fn print(&self) -> String {
        format!("{:?} {} {}", self.inst, self.para[0], self.para[1])
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeParams {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: Vec<u8>,
}

impl IRNode for IRNodeParams {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        buf.push(self.bytecode());
        buf.extend_from_slice(&self.para);
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        // IR serialized form is prefix-coded: [opcode][param...]
        iter::once(self.bytecode())
            .chain(self.para.iter().copied())
            .collect()
    }
    fn print(&self) -> String {
        let parastr = hex::encode(&self.para);
        format!("{:?} 0x{}", self.inst, parastr)
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeParamsSingle {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: Vec<u8>,
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeParamsSingle {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        1
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        self.subx.codegen_into(buf)?;
        buf.push(self.bytecode());
        buf.extend_from_slice(&self.para);
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain(self.para.clone())
            .chain(self.subx.serialize())
            .collect()
    }
    fn print(&self) -> String {
        let parastr = hex::encode(&self.para);
        let substr = self.subx.print();
        format!("{:?} 0x{} {}", self.inst, parastr, substr)
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeWrapOne {
    pub node: Box<dyn IRNode>,
}

impl std::ops::Deref for IRNodeWrapOne {
    type Target = Box<dyn IRNode>;
    fn deref(&self) -> &Box<dyn IRNode> {
        &self.node
    }
}

impl std::ops::DerefMut for IRNodeWrapOne {
    fn deref_mut(&mut self) -> &mut Box<dyn IRNode> {
        &mut self.node
    }
}

impl IRNode for IRNodeWrapOne {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        1
    }
    fn hasretval(&self) -> bool {
        self.node.hasretval()
    }
    fn bytecode(&self) -> u8 {
        self.node.bytecode()
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        self.node.codegen_into(buf)
    }
    fn serialize(&self) -> Vec<u8> {
        self.node.serialize()
    }
    fn print(&self) -> String {
        format!("({})", self.node.print())
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeSingle {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeSingle {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        1
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain(self.subx.serialize())
            .collect()
    }
    fn print(&self) -> String {
        let substr = self.subx.print();
        format!("{:?} {}", self.inst, substr)
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        self.subx.codegen_into(buf)?;
        buf.push(self.bytecode());
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeDouble {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
}

impl IRNode for IRNodeDouble {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn subs(&self) -> usize {
        2
    }
    fn level(&self) -> u8 {
        match OpTy::from_bytecode(self.inst) {
            Ok(t) => t.level(),
            _ => 0,
        }
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        if let Some(c) = compile_double(self.inst, &self.subx, &self.suby)? {
            buf.extend(c);
            return Ok(());
        }
        self.subx.codegen_into(buf)?;
        self.suby.codegen_into(buf)?;
        buf.push(self.bytecode());
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain(self.subx.serialize())
            .chain(self.suby.serialize())
            .collect::<Vec<u8>>()
    }
    fn print(&self) -> String {
        let subxstr = self.subx.print();
        let subystr = self.suby.print();
        format!("{:?} {} {}", self.inst, subxstr, subystr)
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeTriple {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
    pub subz: Box<dyn IRNode>,
}

impl IRNode for IRNodeTriple {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        3
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        if let Some(c) = compile_triple(self.inst, &self.subx, &self.suby, &self.subz)? {
            buf.extend(c);
            return Ok(());
        }
        self.subx.codegen_into(buf)?;
        self.suby.codegen_into(buf)?;
        self.subz.codegen_into(buf)?;
        buf.push(self.bytecode());
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain(self.subx.serialize())
            .chain(self.suby.serialize())
            .chain(self.subz.serialize())
            .collect()
    }
    fn print(&self) -> String {
        let subxstr = self.subx.print();
        let subystr = self.suby.print();
        let subzstr = self.subz.print();
        format!("{:?} {} {} {}", self.inst, subxstr, subystr, subzstr)
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeQuad {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
    pub subz: Box<dyn IRNode>,
    pub subw: Box<dyn IRNode>,
}

impl IRNode for IRNodeQuad {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        4
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        self.subx.codegen_into(buf)?;
        self.suby.codegen_into(buf)?;
        self.subz.codegen_into(buf)?;
        self.subw.codegen_into(buf)?;
        buf.push(self.bytecode());
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain(self.subx.serialize())
            .chain(self.suby.serialize())
            .chain(self.subz.serialize())
            .chain(self.subw.serialize())
            .collect()
    }
    fn print(&self) -> String {
        let subxstr = self.subx.print();
        let subystr = self.suby.print();
        let subzstr = self.subz.print();
        let subwstr = self.subw.print();
        format!(
            "{:?} {} {} {} {}",
            self.inst, subxstr, subystr, subzstr, subwstr
        )
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeQuint {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub suba: Box<dyn IRNode>,
    pub subb: Box<dyn IRNode>,
    pub subc: Box<dyn IRNode>,
    pub subd: Box<dyn IRNode>,
    pub sube: Box<dyn IRNode>,
}

impl IRNode for IRNodeQuint {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        5
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        self.suba.codegen_into(buf)?;
        self.subb.codegen_into(buf)?;
        self.subc.codegen_into(buf)?;
        self.subd.codegen_into(buf)?;
        self.sube.codegen_into(buf)?;
        buf.push(self.bytecode());
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain(self.suba.serialize())
            .chain(self.subb.serialize())
            .chain(self.subc.serialize())
            .chain(self.subd.serialize())
            .chain(self.sube.serialize())
            .collect()
    }
    fn print(&self) -> String {
        let substrs = [
            self.suba.print(),
            self.subb.print(),
            self.subc.print(),
            self.subd.print(),
            self.sube.print(),
        ];
        format!(
            "{:?} {} {} {} {} {}",
            self.inst, substrs[0], substrs[1], substrs[2], substrs[3], substrs[4]
        )
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeParam1Single {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: u8,
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeParam1Single {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        1
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        self.subx.codegen_into(buf)?;
        buf.push(self.bytecode());
        buf.push(self.para);
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain([self.para])
            .chain(self.subx.serialize())
            .collect()
    }
    fn print(&self) -> String {
        let substr = self.subx.print();
        format!("{:?} {} {}", self.inst, self.para, substr)
    }
}

pub(crate) fn validate_param1_multi_arity(
    inst: Bytecode,
    para: u8,
    hrtv: bool,
    argc: usize,
) -> VmrtErr {
    if !is_fin_family(inst) {
        return Ok(());
    }
    let spec = fin_spec_lookup(inst, para)?;
    let expected_argc = spec.argc()? as usize;
    if expected_argc != argc {
        return itr_err_fmt!(
            InstInvalid,
            "FIN IR argc mismatch: expected {} got {} for {:?}",
            expected_argc,
            argc,
            inst
        );
    }
    if !hrtv {
        return itr_err_fmt!(
            InstInvalid,
            "FIN IR return flag mismatch: expected hrtv=true for {:?}",
            inst
        );
    }
    Ok(())
}

fn validate_param1_multi_node(
    inst: Bytecode,
    para: u8,
    hrtv: bool,
    args: &[&dyn IRNode],
) -> VmrtErr {
    validate_param1_multi_arity(inst, para, hrtv, args.len())?;
    if !is_fin_family(inst) {
        return Ok(());
    }
    for arg in args {
        if arg.as_any().downcast_ref::<IRNodeTopStackValue>().is_some() {
            return itr_err_fmt!(
                InstInvalid,
                "FIN IR arg cannot be codegen-only TopStackValue"
            );
        }
        if !arg.hasretval() {
            return itr_err_fmt!(InstInvalid, "FIN IR arg must have a return value");
        }
    }
    Ok(())
}

fn codegen_param1_multi_into(
    buf: &mut Vec<u8>,
    inst: Bytecode,
    para: u8,
    hrtv: bool,
    args: &[&dyn IRNode],
) -> VmrtRes<()> {
    validate_param1_multi_node(inst, para, hrtv, args)?;
    for arg in args {
        arg.codegen_into(buf)?;
    }
    buf.push(inst as u8);
    buf.push(para);
    Ok(())
}

fn serialize_param1_multi(
    inst: Bytecode,
    para: u8,
    hrtv: bool,
    args: &[&dyn IRNode],
    invariant_name: &str,
) -> Vec<u8> {
    if let Err(e) = validate_param1_multi_node(inst, para, hrtv, args) {
        panic!("{} serialize invariant: {}", invariant_name, e.1);
    }
    let mut out = Vec::new();
    out.push(inst as u8);
    out.push(para);
    for arg in args {
        out.extend(arg.serialize());
    }
    out
}

fn print_param1_multi(inst: Bytecode, para: u8, args: &[&dyn IRNode]) -> String {
    let mut out = format!("{:?} {}", inst, para);
    for arg in args {
        out.push(' ');
        out.push_str(&arg.print());
    }
    out
}

pub(crate) struct Param1MultiParts<'a> {
    pub inst: Bytecode,
    pub para: u8,
    pub args: Vec<&'a dyn IRNode>,
}

pub(crate) fn param1_multi_parts<'a>(node: &'a dyn IRNode) -> Option<Param1MultiParts<'a>> {
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1Double>() {
        return Some(Param1MultiParts {
            inst: n.inst,
            para: n.para,
            args: vec![&*n.subx, &*n.suby],
        });
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1Triple>() {
        return Some(Param1MultiParts {
            inst: n.inst,
            para: n.para,
            args: vec![&*n.subx, &*n.suby, &*n.subz],
        });
    }
    if let Some(n) = node.as_any().downcast_ref::<IRNodeParam1Quad>() {
        return Some(Param1MultiParts {
            inst: n.inst,
            para: n.para,
            args: vec![&*n.subx, &*n.suby, &*n.subz, &*n.subw],
        });
    }
    None
}

pub(crate) fn build_param1_multi_node(
    hrtv: bool,
    inst: Bytecode,
    para: u8,
    argvs: Vec<Box<dyn IRNode>>,
) -> VmrtRes<Box<dyn IRNode>> {
    {
        let args = argvs
            .iter()
            .map(|n| &**n as &dyn IRNode)
            .collect::<Vec<_>>();
        validate_param1_multi_node(inst, para, hrtv, &args)?;
    }
    let argc = argvs.len();
    let mut argvs = argvs.into_iter();
    let node: Box<dyn IRNode> = match argc {
        2 => Box::new(IRNodeParam1Double {
            hrtv,
            inst,
            para,
            subx: argvs.next().unwrap(),
            suby: argvs.next().unwrap(),
        }),
        3 => Box::new(IRNodeParam1Triple {
            hrtv,
            inst,
            para,
            subx: argvs.next().unwrap(),
            suby: argvs.next().unwrap(),
            subz: argvs.next().unwrap(),
        }),
        4 => Box::new(IRNodeParam1Quad {
            hrtv,
            inst,
            para,
            subx: argvs.next().unwrap(),
            suby: argvs.next().unwrap(),
            subz: argvs.next().unwrap(),
            subw: argvs.next().unwrap(),
        }),
        _ => {
            return itr_err_fmt!(
                InstInvalid,
                "invalid param1 multi IR node argc {} for {:?}",
                argc,
                inst
            )
        }
    };
    Ok(node)
}

#[derive(Debug, Clone)]
pub struct IRNodeParam1Double {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: u8,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
}

impl IRNode for IRNodeParam1Double {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        2
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        codegen_param1_multi_into(
            buf,
            self.inst,
            self.para,
            self.hrtv,
            &[&*self.subx, &*self.suby],
        )
    }
    fn serialize(&self) -> Vec<u8> {
        serialize_param1_multi(
            self.inst,
            self.para,
            self.hrtv,
            &[&*self.subx, &*self.suby],
            "IRNodeParam1Double",
        )
    }
    fn print(&self) -> String {
        print_param1_multi(self.inst, self.para, &[&*self.subx, &*self.suby])
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeParam1Triple {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: u8,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
    pub subz: Box<dyn IRNode>,
}

impl IRNode for IRNodeParam1Triple {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        3
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        codegen_param1_multi_into(
            buf,
            self.inst,
            self.para,
            self.hrtv,
            &[&*self.subx, &*self.suby, &*self.subz],
        )
    }
    fn serialize(&self) -> Vec<u8> {
        serialize_param1_multi(
            self.inst,
            self.para,
            self.hrtv,
            &[&*self.subx, &*self.suby, &*self.subz],
            "IRNodeParam1Triple",
        )
    }
    fn print(&self) -> String {
        print_param1_multi(
            self.inst,
            self.para,
            &[&*self.subx, &*self.suby, &*self.subz],
        )
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeParam1Quad {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: u8,
    pub subx: Box<dyn IRNode>,
    pub suby: Box<dyn IRNode>,
    pub subz: Box<dyn IRNode>,
    pub subw: Box<dyn IRNode>,
}

impl IRNode for IRNodeParam1Quad {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        4
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        codegen_param1_multi_into(
            buf,
            self.inst,
            self.para,
            self.hrtv,
            &[&*self.subx, &*self.suby, &*self.subz, &*self.subw],
        )
    }
    fn serialize(&self) -> Vec<u8> {
        serialize_param1_multi(
            self.inst,
            self.para,
            self.hrtv,
            &[&*self.subx, &*self.suby, &*self.subz, &*self.subw],
            "IRNodeParam1Quad",
        )
    }
    fn print(&self) -> String {
        print_param1_multi(
            self.inst,
            self.para,
            &[&*self.subx, &*self.suby, &*self.subz, &*self.subw],
        )
    }
}

#[derive(Debug, Clone)]
pub struct IRNodeParam2Single {
    pub hrtv: bool,
    pub inst: Bytecode,
    pub para: [u8; 2],
    pub subx: Box<dyn IRNode>,
}

impl IRNode for IRNodeParam2Single {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        1
    }
    fn hasretval(&self) -> bool {
        self.hrtv
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        self.subx.codegen_into(buf)?;
        buf.push(self.bytecode());
        buf.extend_from_slice(&self.para);
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        iter::once(self.bytecode())
            .chain(self.para)
            .chain(self.subx.serialize())
            .collect()
    }
    fn print(&self) -> String {
        let parastr = hex::encode(&self.para);
        let substr = self.subx.print();
        format!("{:?} 0x{} {}", self.inst, parastr, substr)
    }
}

/*************************************/

#[derive(Default, Debug, Clone)]
pub struct IRNodeBytecodes {
    pub codes: Vec<u8>,
}

impl IRNode for IRNodeBytecodes {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        0
    }
    fn hasretval(&self) -> bool {
        false
    }
    fn bytecode(&self) -> u8 {
        IRBYTECODE as u8
    }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        // Codegen is a pure splice; `verify_ir_runtime_safe_bytecodes` / `IRNodeBytecodes::new`
        // gate the final composed stream, keeping unchecked conversion distinct from runtime rejection.
        Ok(self.codes.clone())
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        buf.extend_from_slice(&self.codes);
        Ok(())
    }
    fn serialize(&self) -> Vec<u8> {
        // Deliberate panic: a truncated serialize would silently drop trailing instructions and
        // change program semantics. All construction paths enforce the length before this point.
        if self.codes.len() > u16::MAX as usize {
            panic!("IRNodeBytecodes payload too long ({} bytes)", self.codes.len());
        }
        iter::once(IRBYTECODE as u8)
            .chain((self.codes.len() as u16).to_be_bytes())
            .chain(self.codes.clone())
            .collect::<Vec<_>>()
    }
    fn print(&self) -> String {
        let codes = format!("0x{}", hex::encode(&self.codes));
        format!("bytecode {{ {} }}", codes)
    }
}

impl IRNodeBytecodes {
    /// Build a new `IRNodeBytecodes` from a verified runtime-shaped payload; rejects oversized or
    /// IR-only / absolute-jump / misaligned payloads. Recommended for in-process builders (parser uses `Default`).
    pub fn new(codes: Vec<u8>) -> VmrtRes<Self> {
        if codes.len() > u16::MAX as usize {
            return itr_err_fmt!(CompileError, "IRNodeBytecodes payload too long");
        }
        verify_ir_bytecode_stream_fragment(&codes)?;
        Ok(Self { codes })
    }
}

/*************************************/

#[derive(Debug, Clone)]
pub struct IRNodeArray {
    pub subs: Vec<Box<dyn IRNode>>,
    pub inst: Bytecode,
}

impl Default for IRNodeArray {
    fn default() -> Self {
        Self::new_list()
    }
}

impl std::ops::Deref for IRNodeArray {
    type Target = Vec<Box<dyn IRNode>>;
    fn deref(&self) -> &Vec<Box<dyn IRNode>> {
        &self.subs
    }
}

impl std::ops::DerefMut for IRNodeArray {
    fn deref_mut(&mut self) -> &mut Vec<Box<dyn IRNode>> {
        &mut self.subs
    }
}

impl IRNode for IRNodeArray {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn subs(&self) -> usize {
        self.subs.len()
    }
    fn hasretval(&self) -> bool {
        // I-4 return-value contract: IRBLOCK never yields a value (pops children), IRBLOCKR
        // yields the last child's value, IRLIST yields its last child's value if it has one.
        match self.inst {
            Bytecode::IRBLOCK => false,
            _ => match self.subs.last() {
                None => false,
                Some(s) => s.hasretval(),
            },
        }
    }
    fn bytecode(&self) -> u8 {
        self.inst as u8
    }
    fn codegen(&self) -> VmrtRes<Vec<u8>> {
        match self.inst {
            Bytecode::IRLIST => compile_list(&self.subs),
            Bytecode::IRBLOCK | Bytecode::IRBLOCKR => compile_block(self.inst, &self.subs),
            _ => errf!("IRNodeArray: invalid opcode {:?}", self.inst).map_ire(InstInvalid),
        }
    }
    fn codegen_into(&self, buf: &mut Vec<u8>) -> VmrtRes<()> {
        match self.inst {
            Bytecode::IRLIST => compile_list_into(&self.subs, buf),
            Bytecode::IRBLOCK | Bytecode::IRBLOCKR => {
                compile_block_into(self.inst, &self.subs, buf)
            }
            _ => errf!("IRNodeArray: invalid opcode {:?}", self.inst).map_ire(InstInvalid),
        }
    }
    fn serialize(&self) -> Vec<u8> {
        // I-5: serialization is not a 1:1 mirror of construction — elided children (IRNodeEmpty) vanish and
        // `count` is recomputed. Visible count must fit u16; panic rather than emit a truncated prefix.
        let count = self
            .subs
            .iter()
            .filter(|a| !a.is_serialization_elided())
            .count();
        if count > u16::MAX as usize {
            panic!("IRNodeArray length overflow: {} children serialize to {} stripped", self.subs.len(), count);
        }
        let mut children_bytes: Vec<u8> = Vec::new();
        for a in &self.subs {
            if a.is_serialization_elided() {
                continue;
            }
            let mut b = a.serialize();
            if b.is_empty() {
                panic!(
                    "IRNodeArray child {:?} serialized to empty bytes without explicit elision",
                    a.bytecode()
                );
            }
            children_bytes.append(&mut b);
        }
        let mut bytes = iter::once(self.inst as u8)
            .chain((count as u16).to_be_bytes())
            .collect::<Vec<_>>();
        bytes.append(&mut children_bytes);
        bytes
    }
    fn print(&self) -> String {
        let num = self.subs.len();
        if num == 0 {
            return format!("{:?} {}:", self.inst, num);
        }
        let mut buf = format!("{:?} {}:\n", self.inst, num);
        for a in &self.subs {
            buf.push_str(&a.print());
            buf.push('\n');
        }
        buf.pop();
        buf
    }
}

impl IRNodeArray {
    pub fn new_list() -> Self {
        Self {
            subs: vec![],
            inst: Bytecode::IRLIST,
        }
    }
    pub fn new_block() -> Self {
        Self {
            subs: vec![],
            inst: Bytecode::IRBLOCK,
        }
    }
    pub fn new_block_expr() -> Self {
        Self {
            subs: vec![],
            inst: Bytecode::IRBLOCKR,
        }
    }
    pub fn with_opcode(inst: Bytecode) -> Self {
        Self { subs: vec![], inst }
    }
    pub fn with_capacity(n: usize, inst: Bytecode) -> Ret<Self> {
        if n > u16::MAX as usize {
            return errf!("IRNodeArray length cannot exceed {}", u16::MAX);
        }
        Ok(Self {
            subs: Vec::with_capacity(n),
            inst,
        })
    }
    pub fn from_vec(subs: Vec<Box<dyn IRNode>>, inst: Bytecode) -> Ret<Self> {
        if subs.len() > u16::MAX as usize {
            return errf!("IRNodeArray length cannot exceed {}", u16::MAX);
        }
        Ok(Self { subs, inst })
    }
    pub fn into_vec(self) -> Vec<Box<dyn IRNode>> {
        self.subs
    }
    pub fn into_one(mut self) -> Box<dyn IRNode> {
        self.subs.pop().unwrap()
    }
    pub fn set_inst(&mut self, inst: Bytecode) {
        self.inst = inst;
    }
}

