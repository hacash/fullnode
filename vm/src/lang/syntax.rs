


/*****************************************/


#[allow(dead_code)]
#[derive(Default)]
pub struct Syntax {
    tokens: Vec<Token>,
    idx: usize,
    locals: HashMap<String, u8>,
    bdlets: HashMap<String, Box<dyn IRNode>>,
    bdlibs: HashMap<String, (u8, Option<field::Address>)>,
    local_alloc: u8,
    check_op: bool,
    // leftv: Box<dyn AST>,
    irnode: IRNodeBlock,
}


#[allow(dead_code)]
impl Syntax {


    /*
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
    */
    

    fn next(&mut self) -> Ret<Token> {
        if self.idx >= self.tokens.len() {
            return errf!("item_with_left get next token error")
        }
        let nxt = &self.tokens[self.idx];
        self.idx += 1;
        Ok(nxt.clone())
    }

    pub fn link_lib(&self, s: &String) -> Ret<u8> {
        match self.bdlibs.get(s).map(|d|d.0) {
            Some(i) => Ok(i),
            _ =>  errf!("cannot find any lib bind '{}'", s)
        }
    }

    pub fn bind_lib(&mut self, s: String, idx: u8, adr: Option<field::Address>) -> Rerr {
        if let Some(..) = self.bdlibs.get(&s) {
            return errf!("<use> cannot repeat bind the symbol '{}'", s)
        }
        if let Some(adr) = adr {
            adr.must_contract()?;
        }
        self.bdlibs.insert(s, (idx, adr));
        Ok(())
    }

    pub fn bind_local_assign(&mut self, s: String, v: Box<dyn IRNode>) -> Ret<Box<dyn IRNode>> {
        let idx = self.local_alloc;
        self.bind_local_assign_replace(s, idx, v)
    }

    pub fn bind_local_assign_replace(&mut self, s: String, idx: u8, v: Box<dyn IRNode>) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        self.bind_local(s, idx)?;
        Ok(Box::new(IRNodeParam1Single{hrtv: false, inst: PUT, para: idx, subx: v}))
    }

    pub fn bind_local(&mut self, s: String, idx: u8) -> Rerr {
        if idx >= self.local_alloc {
            self.local_alloc = idx + 1;
        }
        /* if let Some(..) = self.locals.get(&s) {
            return errf!("<let> cannot repeat bind the symbol '{}'", s)
        } */
        self.locals.insert(s, idx);
        Ok(())
    }

    pub fn bind_let(&mut self, s: String, v: Box<dyn IRNode>) -> Rerr {
        if let Some(..) = self.bdlets.get(&s) {
            return errf!("<let> cannot repeat bind the symbol '{}'", s)
        }
        self.bdlets.insert(s, v);
        Ok(())
    }

    pub fn link_local(&self, s: &String) -> Ret<Box<dyn IRNode>> {
        let text = s.clone();
        match self.locals.get(s) {
            None => return errf!("cannot find symbol '{}'", s),
            Some(i) => Ok(Self::push_local_get(*i, text)),
        }
    }

    pub fn link_let(&mut self, s: &String) -> Ret<Box<dyn IRNode>> {
        match self.bdlets.remove(s) {
            None => return errf!("cannot find or relink symbol '{}'", s),
            Some(d) => Ok(d),
        }
    }

    pub fn link_symbol(&mut self, s: &String) -> Ret<Box<dyn IRNode>> {
        if let Ok(d) = self.link_let(s) {
            return Ok(d)
        }
        self.link_local(s)
    }

    pub fn save_local(&self, s: &String, v: Box<dyn IRNode>) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        match self.locals.get(s) {
            None => return errf!("cannot find symbol '{}'", s),
            Some(i) => Ok(Box::new(IRNodeParam1Single{hrtv: false, inst: PUT, para: *i, subx: v })),
        }
    }
    
    pub fn assign_local(&self, s: &String, v: Box<dyn IRNode>, op: &Token) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        use KwTy::*;
        match self.locals.get(s) {
            None => return errf!("cannot find symbol '{}'", s),
            Some(i) => {
                let i = *i;
                if i < 64 {
                    let mark = i | match op {
                        Keyword(AsgAdd) => 0b00000000,
                        Keyword(AsgSub) => 0b01000000,
                        Keyword(AsgMul) => 0b10000000,
                        Keyword(AsgDiv) => 0b11000000,
                        _ => unreachable!(),
                    };
                    // debug_println!("------------XOP {:?} assign_local: i={}, mark={}, v={:?}", op, i, mark, v);
                    return Ok(Box::new(IRNodeParam1Single{hrtv: false, inst: XOP, para: mark, subx: v }))
                }
                // $0 = $0 + 1
                let getv = Self::push_local_get(i, s!(""));
                let opsv = Box::new(IRNodeDouble{hrtv: true, inst: match op {
                    Keyword(AsgAdd) => ADD,
                    Keyword(AsgSub) => SUB,
                    Keyword(AsgMul) => MUL,
                    Keyword(AsgDiv) => DIV,
                    _ => unreachable!()
                }, subx: getv, suby: v});
                Ok(Box::new(IRNodeParam1Single{hrtv: false, inst: PUT, para: i, subx: opsv }))
            },
        }
    }
    

    pub fn new(mut tokens: Vec<Token>) -> Self {
        tokens.push(Token::Partition('}'));
        Self {
            tokens,
            irnode: IRNodeBlock::new(),
            check_op: true,
            ..Default::default()
        }
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
                return errf!("item with left get next token error")
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
                let mut id: Option<String> = None; 
                if let Some(ir) = left.as_any().downcast_ref::<IRNodeParam1>() {
                    id = Some(ir.as_text().clone());
                };
                if let Some(ir) = left.as_any().downcast_ref::<IRNodeLeaf>() {
                    id = Some(ir.as_text().clone());
                };
                let Some(id) = id else {
                    return e
                };
                let v = unsafe { (&mut *sfptr).item_must(0)? };
                v.checkretval()?; // must retv
                match nxt {
                    Keyword(Assign) => self.save_local(&id, v)?,
                    _ => self.assign_local(&id, v, nxt)?,
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
            Keyword(Is) => {
                let e = errf!("<is> express format error");
                nxt = next!();
                let mut is_not = false;
                if let Keyword(Not) = nxt {
                    is_not = true;
                    nxt = next!();
                }
                let hrtv = true;
                let subx = left;
                let mut res: Box<dyn IRNode> = match nxt {
                    Keyword(Nil)     => Box::new(IRNodeSingle{hrtv, subx, inst: TNIL   }),
                    Keyword(List)    => Box::new(IRNodeSingle{hrtv, subx, inst: TLIST  }),
                    Keyword(Map)     => Box::new(IRNodeSingle{hrtv, subx, inst: TMAP   }),
                    Keyword(Bool)    => Box::new(IRNodeParam1Single{para: ValueTy::Bool    as u8, hrtv, subx, inst: TIS}),
                    Keyword(U8)      => Box::new(IRNodeParam1Single{para: ValueTy::U8      as u8, hrtv, subx, inst: TIS}),
                    Keyword(U16)     => Box::new(IRNodeParam1Single{para: ValueTy::U16     as u8, hrtv, subx, inst: TIS}),
                    Keyword(U32)     => Box::new(IRNodeParam1Single{para: ValueTy::U32     as u8, hrtv, subx, inst: TIS}),
                    Keyword(U64)     => Box::new(IRNodeParam1Single{para: ValueTy::U64     as u8, hrtv, subx, inst: TIS}),
                    Keyword(U128)    => Box::new(IRNodeParam1Single{para: ValueTy::U128    as u8, hrtv, subx, inst: TIS}),
                    Keyword(Bytes)   => Box::new(IRNodeParam1Single{para: ValueTy::Bytes   as u8, hrtv, subx, inst: TIS}),
                    Keyword(Address) => Box::new(IRNodeParam1Single{para: ValueTy::Address as u8, hrtv, subx, inst: TIS}),
                    _ => return e
                };
                if is_not {
                    res = Box::new(IRNodeSingle{hrtv: true, inst: NOT, subx: res})
                }
                res
            }
            Operator(op) if self.check_op => {
                self.check_op = false;
                let res = self.parse_next_op(left, *op)?;
                self.check_op = true;
                res
            }
            _ => { self.idx -= 1; left }
        })
    }

    fn parse_next_op(&mut self, mut left: Box<dyn IRNode>, op: OpTy) -> Ret<Box<dyn IRNode>> {
        let mut right = self.item_must(0)?;
        let c_node = |op: OpTy, l, r| { Box::new(IRNodeDouble{hrtv: true, inst: op.bytecode(), subx: l, suby: r}) };
        if let Ok(nxt) = self.next() {
            if let Operator(op2) = nxt {
                if op.level() >= op2.level() {
                    left = c_node(op, left, right);
                    return self.parse_next_op(left, op2)
                }else{
                    right = self.parse_next_op(right, op2)?;
                }
            }else{
                self.idx -= 1; // back
            }
        }
        Ok(c_node(op, left, right))
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
            0 => Box::new(IRNodeEmpty{}),
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
            } else if let Partition('[') = nxt { // item get
                // println!("---------item_identifier---------- self.tokens[self.idx]= {:?}", nxt); 
                return self.item_get(id)
            } else if let Keyword(Dot) = nxt {
                nxt = next!();
                let Identifier(func) = nxt else {
                    return e1
                };
                self.idx += 1;
                let fnsg = calc_func_sign(func);
                let (args, mut subx) = self.must_get_func_argv(ArgvMode::PackList)?;
                if 0 == args {
                    subx = Self::push_nil()
                }
                return Ok(match &id=="self" {
                    true => { // CALLINR
                        let para: Vec<u8> = fnsg.to_vec(); // fnsig
                        Box::new(IRNodeParamsSingle{hrtv: true, inst: CALLINR, para, subx})
                    },
                    false => { // CALL
                        let lib = self.link_lib(&id)?;
                        let para: Vec<u8> = iter::once(lib).chain(fnsg).collect();
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
                let (_, subx) = self.must_get_func_argv(ArgvMode::PackList)?;
                let inst = maybe!(is_static, CALLSTATIC, CALLLIB);
                let libi = self.link_lib(&id)?;
                let para: Vec<u8> = iter::once(libi).chain(fnsg).collect();
                return Ok(Box::new(IRNodeParamsSingle{hrtv: true, inst, para, subx}))
            }
        }
        self.link_symbol(&id)
    }

    fn item_bytes(b: &Vec<u8>) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        let bl = b.len();
        if bl == 0 {
            return Ok(Self::push_inst(PNBUF))
        }
        if bl > u16::MAX as usize {
            return errf!("bytes data too long")
        }
        let isl = bl > u8::MAX as usize;
        let inst = maybe!(isl, PBUFL, PBUF);
        let size = maybe!(isl, 
            (bl as u16).to_be_bytes().to_vec(),
            vec![bl as u8]
        );
        let para = iter::empty().chain(size).chain(b.clone()).collect::<Vec<_>>();
        Ok(Box::new(IRNodeParams{hrtv: true, inst, para}))
    }

    pub fn push_nil() -> Box<dyn IRNode> {
        use Bytecode::*;
        Self::push_inst(PNIL)
    }

    pub fn push_local_get(i: u8, text: String) -> Box<dyn IRNode> {
        use Bytecode::*;
        match i {
            0 => Box::new(IRNodeLeaf{  hrtv: true, inst: GET0, text }),
            1 => Box::new(IRNodeLeaf{  hrtv: true, inst: GET1, text }),
            2 => Box::new(IRNodeLeaf{  hrtv: true, inst: GET2, text }),
            3 => Box::new(IRNodeLeaf{  hrtv: true, inst: GET3, text }),
            _ => Box::new(IRNodeParam1{hrtv: true, inst: GET,  text, para: i })
        }
    }

    pub fn push_inst_noret(inst: Bytecode) -> Box<dyn IRNode> {
        Box::new(IRNodeLeaf::notext(false, inst))
    }

    pub fn push_inst(inst: Bytecode) -> Box<dyn IRNode> {
        Box::new(IRNodeLeaf::notext(true, inst))
    }

    pub fn push_num(n: u128) -> Box<dyn IRNode> {
        use Bytecode::*;
        macro_rules! push_uint { ($n:expr, $t:expr) => {{
            let buf = buf_drop_left_zero(&$n.to_be_bytes(), 0);
            let numv = iter::once(buf.len() as u8).chain(buf).collect::<Vec<_>>();
            Box::new(IRNodeSingle{hrtv: true, inst: $t, subx: Box::new(IRNodeParams{
                hrtv: true, inst: PBUF, para: numv,
            })})
        }}}
        match n {
            0 => Self::push_inst(P0),
            1 => Self::push_inst(P1),
            2 => Self::push_inst(P2),
            3 => Self::push_inst(P3),
            4..256 => Box::new(IRNodeParam1{hrtv: true, inst: PU8, para: n as u8, text: s!("")}),
            256..65536 => Box::new(IRNodeParam2{hrtv: true, inst: PU16, para: (n as u16).to_be_bytes() }),
            65536..4294967296 => push_uint!(n, CU32),
            4294967296..18446744073709551616 => push_uint!(n, CU64),
            _ => push_uint!(n, CU128),
        }
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
        macro_rules! back { () => {
            self.idx -= 1;
        }}
        let mut nxt = next!();
        let mut item: Box<dyn IRNode> = match nxt {
            Identifier(id) => self.item_identifier(id.clone())?,
            Integer(n) => Self::push_num(*n),
            Token::Bytes(b) => Self::item_bytes(b)?,
            Partition('(') => {
                let ckop = self.check_op;
                self.check_op = true;
                let exp = self.item_must(0)?;
                self.check_op = ckop; // recover
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
            Partition('[') => { // pack_list
                let mut subs = vec![];
                loop {
                    nxt = next!();
                    if let Partition(']') = nxt {
                        break
                    };
                    self.idx -= 1;
                    let item = self.item_must(0)?;
                    item.checkretval()?; // must retv
                    subs.push(item);
                }
                let num = subs.len();
                let mut list = IRNodeList{subs};
                list.push(Self::push_num(num as u128));
                list.push(Self::push_inst(PACKLIST));
                Box::new(list)
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
            Keyword(Var) => { // let foo = $0
                let e = errf!("var statement format error");
                let gidx = |nxt: &Token| {
                    let mut lcalc: Option<u8> = None;
                    if let Identifier(num) = nxt.clone() {
                        if '$' == num.as_bytes()[0] as char {
                            if let Ok(idx) = num.trim_start_matches('$').parse::<u8>() {
                                lcalc = Some(idx);
                            };
                        }
                    }
                    lcalc
                };
                nxt = next!();
                let Identifier(id) = nxt else {
                    return e
                };
                nxt = next!();
                let mut reset_idx = None;
                if let Some(idx) = gidx(nxt) {
                    reset_idx = Some(idx);
                    nxt = next!();
                }
                let Keyword(Assign) = nxt else {
                    return e
                };
                nxt = next!();
                let vk = id.clone();
                match gidx(nxt) {
                    Some(idx) => {
                        self.bind_local(vk, idx)?;
                        Box::new(IRNodeEmpty{})
                    }
                    _ => {
                        self.idx -= 1;
                        let v = self.item_must(0)?;
                        // debug_println!("------------ var {} {:?} = {:?}", vk, reset_idx, v);
                        match reset_idx {
                            Some(idx) => self.bind_local_assign_replace(vk, idx, v)?,
                            _ => self.bind_local_assign(vk, v)?,
                        }
                    }
                }
            }
            Keyword(Let) => { // let foo = $0
                let e = errf!("let statement format error");
                nxt = next!();
                let Identifier(id) = nxt else {
                    return e
                };
                let id = id.clone();
                nxt = next!();
                let Keyword(Assign) = nxt else {
                    return e
                };
                let exp = self.item_must(0)?;
                exp.checkretval()?; // must retv
                self.bind_let(id, exp)?;
                Box::new(IRNodeEmpty{})
            }
            /*
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
            */
            Keyword(Lib) => { // lib AnySwap = VFE6Zu4Wwee1vjEkQLxgVbv3c6Ju9iTaa(1)
                let e = errf!("lib statement format error");
                nxt = next!();
                let Identifier(id) = nxt else { return e };
                nxt = next!();
                let Keyword(Assign) = nxt else { return e };
                nxt = next!();
                let Integer(idx) = nxt else { return e };
                nxt = next!();
                let mut adr = None;
                if let Keyword(Colon) = nxt {
                    nxt = next!();
                    let Token::Address(a) = nxt else { return e };
                    adr = Some(*a as field::Address);
                }else{
                    back!();
                }
                if *idx > u8::MAX as u128 {
                    return errf!("lib statement link index overflow")
                }
                // debug_println!("lib statement {}, {:?}", *idx, adr);
                self.bind_lib(id.clone(), *idx as u8, adr)?;
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
                Box::new(IRNodeBytecodes{codes})
            }
            Keyword(Nil)    => Self::push_nil(),
            Keyword(True)   => Self::push_inst(P1),
            Keyword(False)  => Self::push_inst(P0),
            Keyword(Abort)  => Self::push_inst_noret(ABT),
            Keyword(End)    => Self::push_inst_noret(END),
            Keyword(Print)  => Box::new(IRNodeSingle{hrtv: false, inst: PRT, subx: self.item_must(0)?}),
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
            let allocs = Box::new(IRNodeParam1{
                hrtv: false, inst: Bytecode::ALLOC, para: *m+1, text: s!("")
            });
            self.irnode.subs[0] = allocs;
        }
        Ok(self.irnode)
    }


}


