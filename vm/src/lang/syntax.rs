


/*****************************************/


#[allow(dead_code)]
#[derive(Default)]
pub struct Syntax {
    tokens: Vec<Token>,
    idx: usize,
    locals: HashMap<String, u8>,
    bduses: HashMap<String, Address>,
    bdlibs: HashMap<String, (Address, u8)>,
    local_alloc: u8,
    // leftv: Box<dyn AST>,
    irnode: IRNodeBlock,
}


#[allow(dead_code)]
impl Syntax {


    pub fn bind_uses(&mut self, s: String, adr: Vec<u8>) -> Rerr {
        if let Some(..) = self.bduses.get(&s) {
            return errf!("<use> cannot repeat bind the symbol '{}'", s)
        }
        let addr = Address::from_vec(adr);
        addr.must_contract()?;
        self.bduses.insert(s, addr);
        Ok(())
    }

    pub fn link_use(&self, s: &String) -> Ret<Vec<u8>> {
        match self.bduses.get(s) {
            Some(i) => Ok(i.to_vec()),
            _ =>  errf!("cannot find any use bind '{}'", s)
        }
    }

    pub fn link_lib(&self, s: &String) -> Ret<u8> {
        match self.bdlibs.get(s).map(|d|d.1) {
            Some(i) => Ok(i),
            _ =>  errf!("cannot find any lib bind '{}'", s)
        }
    }

    pub fn bind_libs(&mut self, s: String, adr: Vec<u8>, idx: u8) -> Rerr {
        if let Some(..) = self.bdlibs.get(&s) {
            return errf!("<use> cannot repeat bind the symbol '{}'", s)
        }
        let addr = Address::from_vec(adr);
        addr.must_contract()?;
        self.bdlibs.insert(s, (addr, idx));
        Ok(())
    }

    pub fn bind_local(&mut self, s: String, idx: u8) -> Rerr {
        if let Some(..) = self.locals.get(&s) {
            return errf!("<let> cannot repeat bind the symbol '{}'", s)
        }
        self.locals.insert(s, idx);
        Ok(())
    }

    pub fn link_local(&self, s: &String) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        match self.locals.get(s) {
            None => return errf!("cannot find symbol '{}'", s),
            Some(i) => Ok(Box::new(IRNodeParam1{hrtv: true, inst: GETX, para: *i, text: s.clone() })),
        }
    }

    pub fn save_local(&self, s: &String, v: Box<dyn IRNode>) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        match self.locals.get(s) {
            None => return errf!("cannot find symbol '{}'", s),
            Some(i) => Ok(Box::new(IRNodeParam1Single{hrtv: false, inst: PUTX, para: *i, subx: v })),
        }
    }
    
    pub fn assign_local(&self, s: &String, v: Box<dyn IRNode>, op: &Token) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        use KwTy::*;
        match self.locals.get(s) {
            None => return errf!("cannot find symbol '{}'", s),
            Some(i) => {
                if *i < 64 {
                    let mark = *i & match op {
                        Keyword(AsgAdd) => 0b00111111,
                        Keyword(AsgSub) => 0b01111111,
                        Keyword(AsgMul) => 0b10111111,
                        Keyword(AsgDiv) => 0b11111111,
                        _ => unreachable!(),
                    };
                    return Ok(Box::new(IRNodeParam1Single{hrtv: false, inst: XOP, para: mark, subx: v }))
                }
                // $0 = $0 + 1
                let getv = Box::new(IRNodeParam1{hrtv: true, inst: GETX, para: *i, text: s!("")});
                let opsv = Box::new(IRNodeDouble{hrtv: true, inst: match op {
                    Keyword(AsgAdd) => ADD,
                    Keyword(AsgSub) => SUB,
                    Keyword(AsgMul) => MUL,
                    Keyword(AsgDiv) => DIV,
                    _ => unreachable!()
                }, subx: getv, suby: v});
                Ok(Box::new(IRNodeParam1Single{hrtv: false, inst: PUTX, para: *i, subx: opsv }))
            },
        }
    }
    

    pub fn new(mut tokens: Vec<Token>) -> Self {
        tokens.push(Token::Partition('}'));
        Self {
            tokens,
            irnode: IRNodeBlock::new(),
            ..Default::default()
        }
    }

    fn may_swap_op_level(lop: OpTy, mut left: Box<dyn IRNode>, mut right: Box<dyn IRNode>) -> Ret<(Bytecode, Box<dyn IRNode>, Box<dyn IRNode>)> {
        let mut inst = lop.bytecode();
        let mut change = false;
        if let Some(y) = right.as_any().downcast_ref::<IRNodeDouble>() {
            let rlv = OpTy::from_bytecode(y.inst)?.level();
            let llv = lop.level();
            change = llv > rlv;
            if change {
                left = Box::new(IRNodeDouble{ hrtv: true, inst, subx: left, suby: y.subx.clone() });
                inst = y.inst;
            }
        }
        if change {
            if let Some(y) = right.clone().as_any().downcast_ref::<IRNodeDouble>() {
                right = y.suby.clone();
            }else{
                unreachable!()
            }
        }
        Ok((inst, left, right))
    }


    pub fn item_with_left(&mut self, left: Box<dyn IRNode>) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        use KwTy::*;
        let max = self.tokens.len();
        let sfptr = self as *mut Syntax;
        if self.idx >= self.tokens.len() {
            return Ok(left) // end
        }
        macro_rules! next { () => {{
            if self.idx >= max {
                return errf!("item_with_left get next token error")
            }
            let nxt = &self.tokens[self.idx];
            self.idx += 1;
            nxt
        }}}
        let mut nxt = next!();
        Ok(match nxt {
            Keyword(Assign) 
            | Keyword(AsgAdd) 
            | Keyword(AsgSub)
            | Keyword(AsgMul)
            | Keyword(AsgDiv) => {
                let e = errf!("assign statement format error");
                let Some(ir) = left.as_any().downcast_ref::<IRNodeParam1>() else {
                    return e
                };
                let v = unsafe { (&mut *sfptr).item_must(0)? };
                v.checkretval()?; // must retv
                let id = ir.as_text();
                match nxt {
                    Keyword(Assign) => self.save_local(id, v)?,
                    _ => self.assign_local(id, v, nxt)?,
                } 
            }
            Keyword(As) => {
                let e = errf!("<as> express format error");
                nxt = next!();
                let mut obj = IRNodeSingle{hrtv: true, inst: CU8, subx: left};
                match nxt {
                    Keyword(U8)    => obj.inst = CU8   ,
                    Keyword(U16)   => obj.inst = CU16  ,
                    Keyword(U32)   => obj.inst = CU32  ,
                    Keyword(U64)   => obj.inst = CU64  ,
                    Keyword(U128)  => obj.inst = CU128 ,
                    Keyword(Bytes) => obj.inst = CBUF  ,
                    _ => return e
                };
                obj.checkretval()?; // must retv
                Box::new(obj)
            }
            Operator(op) => {
                let inst;
                let mut subx = left;
                let mut suby = unsafe { (&mut *sfptr).item_must(0)? };
                subx.checkretval()?; // must retv
                suby.checkretval()?; // must retv
                (inst, subx, suby) = Self::may_swap_op_level(*op, subx, suby)?;
                Box::new(IRNodeDouble{ hrtv: true, inst, subx, suby })
            }
            _ => { self.idx -= 1; left }
        })
    }

    pub fn item_must(&mut self, jp: usize) -> Ret<Box<dyn IRNode>> {
        self.idx += jp;
        match self.item_may()? {
            Some(n) => Ok(n),
            None => errf!("not match next Syntax node")
        }
    }

    pub fn item_may_list(&mut self) -> Ret<Box<dyn IRNode>> {
        let block = self.item_may_block()?;
        Ok(match block.len() {
            0 => IRNodeLeaf::nop_box(),
            1 => block.into_one(),
            _ => Box::new(block)
        })
    }
    
    pub fn item_may_block(&mut self) -> Ret<IRNodeBlock> {
        let mut block = IRNodeBlock::new();
        let max = self.tokens.len() - 1;
        let e =  errf!("block format error");
        if self.idx >= max {
            return e
        }
        let nxt = &self.tokens[self.idx];
        if let Partition('{')|Partition('(') = nxt {} else {
            return e
        };
        self.idx += 1;
        loop {
            if self.idx >= max { break }
            let nxt = &self.tokens[self.idx];
            if let Partition('}')|Partition(')') = nxt {
                self.idx += 1;
                break
            }
            let Some(li) = self.item_may()? else {
                break
            };
            block.push( li );
        }
        Ok(block)
    }

    pub fn item_identifier(&mut self, id: String) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        use KwTy::*;
        let max = self.tokens.len() - 1;
        let e1 = errf!("call express after identifier format error");
        macro_rules! next {() => {{
            self.idx += 1;
            if self.idx >= max {
                return e1
            }
            &self.tokens[self.idx]
        }}}
        if self.idx < max {
            let mut nxt = &self.tokens[self.idx];
            if let Partition('(') = nxt { // function call
                return self.item_func_call(id)
            } else if let Keyword(Dot) = nxt {
                nxt = next!();
                let Identifier(func) = nxt else {
                    return e1
                };
                self.idx += 1;
                let fnsg = calc_func_sign(func);
                let subx = self.must_get_func_argv()?;
                return Ok(match &id=="self" {
                    true => { // CALLLOC
                        let para: Vec<u8> = fnsg.to_vec();
                        Box::new(IRNodeParamsSingle{hrtv: true, inst: CALLLOC, para, subx})
                    },
                    false => { // CALL
                        let usadr = self.link_use(&id)?;
                        let para: Vec<u8> = iter::empty().chain(usadr).chain(fnsg).collect();
                        Box::new(IRNodeParamsSingle{hrtv: true, inst: CALL, para, subx})
                    },
                })
            }else if Keyword(Colon) == *nxt || Keyword(DColon) == *nxt {
                let is_static = Keyword(DColon) == *nxt;
                nxt = next!();
                let Identifier(func) = nxt else {
                    return e1
                };
                self.idx += 1;
                let fnsg = calc_func_sign(func);
                let subx = self.must_get_func_argv()?;
                let inst = maybe!(is_static, CALLSTATIC, CALLLIB);
                let libi = self.link_lib(&id)?;
                let para: Vec<u8> = iter::once(libi).chain(fnsg).collect();
                return Ok(Box::new(IRNodeParamsSingle{hrtv: true, inst, para, subx}))
            }
        }
        self.link_local(&id)
    }

    fn item_bytes(b: &Vec<u8>) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        let bl = b.len();
        if bl == 0 {
            return Ok(Box::new(IRNodeLeaf{hrtv: true, inst: PNBUF}))
        }
        if bl > u16::MAX as usize {
            return errf!("bytes data too long")
        }
        let isl = bl > u8::MAX as usize;
        let inst = maybe!(isl, PBUFL, PBUF);
        let size = maybe!(isl, 
            (bl as u16 - 1).to_be_bytes().to_vec(),
            vec![bl as u8 - 1]
        );
        let para = iter::empty().chain(size).chain(b.clone()).collect::<Vec<_>>();
        Ok(Box::new(IRNodeParams{hrtv: true, inst, para}))
    }

    pub fn item_may(&mut self) -> Ret<Option<Box<dyn IRNode>>> {
        use Bytecode::*;
        use KwTy::*;
        let max = self.tokens.len() - 1;
        if self.idx >= max {
            return Ok(None) // end
        }
        macro_rules! next { () => {{
            if self.idx >= max {
                return errf!("item_may get next token error")
            }
            let nxt = &self.tokens[self.idx];
            self.idx += 1;
            nxt
        }}}
        macro_rules! push_uint { ($n:expr, $t:expr) => {{
            let buf = buf_drop_left_zero(&$n.to_be_bytes(), 0);
            let numv = iter::once(buf.len() as u8 - 1).chain(buf).collect::<Vec<_>>();
            Box::new(IRNodeSingle{hrtv: true, inst: $t, subx: Box::new(IRNodeParams{
                hrtv: true, inst: PBUF, para: numv,
            })})
        }}}
        let mut nxt = next!();
        let mut item: Box<dyn IRNode> = match nxt {
            Identifier(id) => self.item_identifier(id.clone())?,
            Integer(n) => match n {
                0 => Box::new(IRNodeLeaf{hrtv: true, inst: P0}),
                1 => Box::new(IRNodeLeaf{hrtv: true, inst: P1}),
                2..256 => Box::new(IRNodeParam1{hrtv: true, inst: PU8, para: *n as u8, text: s!("")}),
                256..65536 => Box::new(IRNodeParam2{hrtv: true, inst: PU16, para: (*n as u16).to_be_bytes() }),
                65536..4294967296 => push_uint!(n, CU32),
                4294967296..18446744073709551616 => push_uint!(n, CU64),
                _ => push_uint!(n, CU128),
            }
            Token::Bytes(b) => Self::item_bytes(b)?,
            Partition('(') => {
                let exp = self.item_must(0)?;
                exp.checkretval()?; // must retv
                let e = errf!("(..) expression format error");
                nxt = next!();
                let Partition(')') = nxt else {
                    return e
                };
                maybe!(exp.subs() >= 2,
                    Box::new(IRNodeWrapOne{node: exp}),
                    exp
                )
            }
            Keyword(While) => {
                let exp = self.item_must(0)?;
                exp.checkretval()?; // must retv
                // let e = errf!("while statement format error");
                let suby = self.item_may_list()?;
                Box::new(IRNodeDouble{hrtv: false, inst: IRWHILE, subx: exp, suby})
            }
            Keyword(If) => {
                let exp = self.item_must(0)?;
                exp.checkretval()?; // must retv
                let list = self.item_may_list()?;
                let mut ifobj = IRNodeTriple{
                    hrtv: false, inst: IRIF, subx: exp, suby: list, subz: IRNodeLeaf::nop_box()
                };
                let nxt = &self.tokens[self.idx];
                let Keyword(Else) = nxt else {
                    // no else
                    return Ok(Some(Box::new(ifobj)))
                };
                self.idx += 1; // over else token
                let nxt = &self.tokens[self.idx];
                // else
                let Keyword(If) = nxt else {
                    let elseobj = self.item_may_list()?;
                    ifobj.subz = elseobj;
                    return Ok(Some(Box::new(ifobj)))
                };
                // else if
                let elseifobj = self.item_must(0)?;
                ifobj.subz = elseifobj;
                Box::new(ifobj)
            }
            Keyword(Let) => { // let foo = $0
                let e = errf!("let statement format error");
                nxt = next!();
                let Identifier(id) = nxt else {
                    return e
                };
                nxt = next!();
                let Keyword(Assign) = nxt else {
                    return e
                };
                nxt = next!();
                let Identifier(num) = nxt else {
                    return e
                };
                if '$' != num.as_bytes()[0] as char {
                    return e
                }
                let Ok(idx) = num.trim_start_matches('$').parse::<u8>() else {
                    return e
                };
                self.bind_local(id.clone(), idx)?;
                Box::new(IRNodeEmpty{})
            }
            Keyword(Use) => { // use AnySwap = VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa
                let e = errf!("use statement format error");
                nxt = next!();
                let Identifier(id) = nxt else {
                    return e
                };
                nxt = next!();
                let Keyword(Assign) = nxt else {
                    return e
                };
                nxt = next!();
                let Token::Bytes(addr) = nxt else {
                    return e
                };
                self.bind_uses(id.clone(), addr.clone())?;
                Box::new(IRNodeEmpty{})
            }
            Keyword(Lib) => { // lib AnySwap = VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa(1)
                let e = errf!("lib statement format error");
                nxt = next!();
                let Identifier(id) = nxt else {
                    return e
                };
                nxt = next!();
                let Keyword(Assign) = nxt else {
                    return e
                };
                nxt = next!();
                let Token::Bytes(addr) = nxt else {
                    return e
                };
                nxt = next!();
                let Partition('(') = nxt else {
                    return e
                };
                nxt = next!();
                let Integer(idx) = nxt else {
                    return e
                };
                nxt = next!();
                let Partition(')') = nxt else {
                    return e
                };
                if *idx > u8::MAX as u128 {
                    return errf!("<lib> statement link index overflow")
                }
                self.bind_libs(id.clone(), addr.clone(), *idx as u8)?;
                Box::new(IRNodeEmpty{})
            }
            Keyword(CallCode) => {
                let e = errf!("callcode statement format error");
                nxt = next!();
                let Identifier(id) = nxt else {
                    return e
                };
                nxt = next!();
                let Keyword(DColon) = nxt else {
                    return e
                };
                nxt = next!();
                let Identifier(func) = nxt else {
                    return e
                };
                let fnsg = calc_func_sign(func);
                let para: Vec<u8> = iter::once(self.link_lib(id)?).chain(fnsg).collect();
                Box::new(IRNodeParams{hrtv: false, inst: CALLCODE, para})
            }
            Keyword(ByteCode) => {
                let e = errf!("bytecode format error");
                nxt = next!();
                let Partition('{') = nxt else {
                    return e
                };
                let mut codes: Vec<u8> = Vec::new();
                loop {
                    let inst: u8;
                    match next!() {
                        Identifier(id) => {
                            let Some(t) = Bytecode::parse(id) else {
                                return errf!("bytecode {} not find", id)
                            };
                            inst = t as u8;
                        }
                        Integer(n) if *n <= u8::MAX as u128 => {
                            inst = *n as u8;
                        }
                        Partition('}') => break, // end
                        _ => return e
                    }
                    codes.push(inst as u8);
                }
                Box::new(IRNodeBytecodes{inst: IRCODE, codes})
            }
            Keyword(Abort)  => Box::new(IRNodeLeaf{hrtv: false, inst: ABT}),
            Keyword(Finish) => Box::new(IRNodeLeaf{hrtv: false, inst: END}),
            Keyword(Assert) => Box::new(IRNodeSingle{hrtv: false, inst: AST, subx: self.item_must(0)?}),
            Keyword(Throw)  => Box::new(IRNodeSingle{hrtv: false, inst: ERR, subx: self.item_must(0)?}),
            Keyword(Return) => Box::new(IRNodeSingle{hrtv: false, inst: RET, subx: self.item_must(0)?}),
            _ => return errf!("unsupport token '{:?}'", nxt),
        };
        item = self.item_with_left(item)?;
        Ok(Some(item))
    }


    pub fn parse(mut self) -> Ret<IRNodeBlock> {
        self.irnode.push(Box::new(IRNodeEmpty{}));
        while let Some(item) = self.item_may()? {
            if let Some(..) = item.as_any().downcast_ref::<IRNodeEmpty>() {} else {
                self.irnode.push(item);
            };
        }
        // local alloc
        if let Some(m) = self.locals.values().max() {
            let local_alloc = Box::new(IRNodeParam1{
                hrtv: false, inst: Bytecode::ALLOC, para: *m+1, text: s!("")
            });
            self.irnode.subs[0] = local_alloc;
        }
        Ok(self.irnode)
    }


}


