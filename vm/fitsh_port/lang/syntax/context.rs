use super::*;

#[derive(Clone, Copy)]
enum SequenceClose {
    TopLevel,
    Partition(char),
}

#[derive(Clone, Copy)]
enum SequenceMode {
    Values,
    Statements { keep_retval: bool },
}

enum ResolvedSymbol {
    Slot { idx: u8, state: SlotStateV2 },
    Expr(Box<dyn IRNode>),
}

impl Syntax {
    pub(super) fn with_expect_retval<R>(
        &mut self,
        expect: bool,
        f: impl FnOnce(&mut Self) -> Ret<R>,
    ) -> Ret<R> {
        let prev = self.mode.expect_retval;
        self.mode.expect_retval = expect;
        let res = f(self);
        self.mode.expect_retval = prev;
        res
    }

    pub(super) fn with_loop_scope<R>(&mut self, f: impl FnOnce(&mut Self) -> Ret<R>) -> Ret<R> {
        self.mode.loop_depth += 1;
        let res = f(self);
        self.mode.loop_depth -= 1;
        res
    }

    pub(super) fn build_irlist(subs: Vec<Box<dyn IRNode>>) -> Ret<IRNodeArray> {
        IRNodeArray::from_vec(subs, Bytecode::IRLIST)
    }

    pub(super) fn build_list_node(
        &mut self,
        mut subs: Vec<Box<dyn IRNode>>,
    ) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        let len = subs.len();
        if len == 0 {
            return Ok(push_inst(NEWLIST));
        }
        subs.push(push_num(len as u128));
        subs.push(push_inst(PACKLIST));
        Ok(Box::new(Self::build_irlist(subs)?))
    }

    pub(super) fn build_param_prelude(params: usize, allow_zero: bool) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        match params {
            0 if allow_zero => Ok(push_inst_noret(POP)),
            1 => Ok(push_single_p1(PUT, 0, push_inst(ROLL0))),
            2.. => Ok(push_double(UNPACK, ROLL0, P0)),
            _ => errf!("at least one param required"),
        }
    }

    pub(super) fn parse_slot_alias(name: &str) -> Option<u8> {
        if !start_with_char(name, '$') {
            return None;
        }
        name.trim_start_matches('$').parse::<u8>().ok()
    }

    fn slot_meta(&self, idx: u8) -> Ret<SlotStateV2> {
        if let Some(state) = self.slots.get(&idx) {
            return Ok(*state);
        }
        // Decompiled source without source-map metadata can refer to raw `$n` slots.
        // Those slots may appear without an explicit local declaration and must stay writable.
        let _ = idx;
        Ok(SlotStateV2 { mutable: true })
    }

    pub(super) fn bind_slot(
        &mut self,
        name: String,
        idx: u8,
        kind: SlotKind,
    ) -> Ret<Box<dyn IRNode>> {
        self.reserve_slot(idx)?;
        if idx >= self.local_alloc {
            if idx == u8::MAX {
                return errf!("slot {} exceeds limit", idx);
            }
            self.local_alloc = idx + 1;
        }
        self.register_slot_symbol(name.clone(), idx)?;
        self.slots.insert(
            idx,
            SlotStateV2 {
                mutable: !matches!(kind, SlotKind::Let),
            },
        );
        self.emit.source_map.register_slot(idx, name)?;
        Ok(push_empty())
    }

    pub(super) fn bind_slot_with_value(
        &mut self,
        name: String,
        idx: Option<u8>,
        value: Box<dyn IRNode>,
        kind: SlotKind,
    ) -> Ret<Box<dyn IRNode>> {
        let idx = idx.unwrap_or(self.local_alloc);
        self.bind_slot(name, idx, kind)?;
        Ok(push_single_p1(Bytecode::PUT, idx, value))
    }

    fn register_slot_symbol(&mut self, name: String, idx: u8) -> Rerr {
        if self.symbols.contains_key(&name) {
            return errf!("symbol '{}' already bound", name);
        }
        self.symbols.insert(name, SymbolEntryV2::Slot(idx));
        Ok(())
    }

    fn register_bind_symbol(&mut self, name: String, entry: SymbolEntryV2) -> Rerr {
        if self.symbols.contains_key(&name) {
            return errf!("symbol '{}' already bound", name);
        }
        self.symbols.insert(name, entry);
        Ok(())
    }

    pub(super) fn register_const_symbol(&mut self, name: String, node: Box<dyn IRNode>) -> Rerr {
        self.register_bind_symbol(name, SymbolEntryV2::Const(node))
    }

    pub(super) fn bind_macro(&mut self, name: String, node: Box<dyn IRNode>) -> Rerr {
        self.register_bind_symbol(name, SymbolEntryV2::Bind(node))
    }

    fn reserve_slot(&mut self, idx: u8) -> Rerr {
        if self.slot_used.contains(&idx) {
            return errf!("slot {} already bound", idx);
        }
        self.slot_used.insert(idx);
        Ok(())
    }

    fn resolve_symbol(&self, name: &str) -> Ret<ResolvedSymbol> {
        if let Some(idx) = Self::parse_slot_alias(name) {
            return Ok(ResolvedSymbol::Slot {
                idx,
                state: SlotStateV2 { mutable: true },
            });
        }
        match self.symbols.get(name) {
            Some(SymbolEntryV2::Slot(idx)) => Ok(ResolvedSymbol::Slot {
                idx: *idx,
                state: self.slot_meta(*idx)?,
            }),
            Some(SymbolEntryV2::Bind(node)) | Some(SymbolEntryV2::Const(node)) => {
                Ok(ResolvedSymbol::Expr(clone_box(node.as_ref())))
            }
            None => errf!("cannot find symbol '{}'", name),
        }
    }

    pub(super) fn link_symbol(&self, name: &str) -> Ret<Box<dyn IRNode>> {
        match self.resolve_symbol(name)? {
            ResolvedSymbol::Slot { idx, .. } => Ok(push_local_get(idx, name.to_string())),
            ResolvedSymbol::Expr(node) => Ok(node),
        }
    }

    fn resolve_lvalue_slot(&self, name: &str) -> Ret<(u8, bool)> {
        match self.resolve_symbol(name)? {
            ResolvedSymbol::Slot { idx, state } => Ok((idx, state.mutable)),
            ResolvedSymbol::Expr(_) => errf!("cannot assign to non-slot symbol '{}'", name),
        }
    }

    pub(super) fn save_local(
        &mut self,
        name: &str,
        value: Box<dyn IRNode>,
    ) -> Ret<Box<dyn IRNode>> {
        let (idx, mutable) = self.resolve_lvalue_slot(name)?;
        if !mutable {
            return errf!("cannot assign to immutable symbol '{}'", name);
        }
        self.emit.source_map.mark_slot_mutated(idx);
        Ok(push_single_p1(Bytecode::PUT, idx, value))
    }

    pub(super) fn assign_local(
        &mut self,
        name: &str,
        value: Box<dyn IRNode>,
        op: Token,
    ) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        use KwTy::*;
        let (idx, mutable) = self.resolve_lvalue_slot(name)?;
        if !mutable {
            return errf!("cannot assign to immutable symbol '{}'", name);
        }
        self.emit.source_map.mark_slot_mutated(idx);
        let (lxop, inst) = match op {
            Token::Keyword(AsgAdd) => (LxOp::Add, ADD),
            Token::Keyword(AsgSub) => (LxOp::Sub, SUB),
            Token::Keyword(AsgMul) => (LxOp::Mul, MUL),
            Token::Keyword(AsgDiv) => (LxOp::Div, DIV),
            _ => return errf!("assign statement format invalid"),
        };
        if idx < 64 {
            let mark = encode_local_operand_mark(lxop, idx).map_err(Error::from)?;
            return Ok(push_single_p1(XOP, mark, value));
        }
        let getv = push_local_get(idx, s!(""));
        let expr = Box::new(IRNodeDouble {
            hrtv: true,
            inst,
            subx: getv,
            suby: value,
        });
        Ok(push_single_p1(PUT, idx, expr))
    }

    pub(super) fn bind_lib(&mut self, name: String, idx: u8, addr: Option<FieldAddress>) -> Rerr {
        if self.libs.contains_key(&name) {
            return errf!("lib '{}' already bound", name);
        }
        if self.libs.values().any(|(bound, _)| *bound == idx) {
            return errf!("lib index {} binding already exists", idx);
        }
        if let Some(addr) = addr {
            addr.must_contract()?;
        }
        self.libs.insert(name.clone(), (idx, addr.clone()));
        self.emit.source_map.register_lib(idx, name, addr)?;
        Ok(())
    }

    pub(super) fn link_lib(&self, name: &str) -> Ret<u8> {
        self.libs
            .get(name)
            .map(|v| v.0)
            .ok_or_else(|| format!("no lib binding '{}' found", name).into())
    }

    fn sequence_expect_retval(mode: SequenceMode) -> bool {
        match mode {
            SequenceMode::Values => true,
            SequenceMode::Statements { keep_retval } => keep_retval,
        }
    }

    fn parse_sequence(
        &mut self,
        close: SequenceClose,
        mode: SequenceMode,
        err_msg: &'static str,
    ) -> Ret<Vec<Box<dyn IRNode>>> {
        self.with_expect_retval(Self::sequence_expect_retval(mode), |s| {
            let mut items = Vec::new();
            let mut terminated = false;
            loop {
                // Optional soft separators: comma and normalized semicolon.
                // They only terminate/separate expressions/statements, and carry no count semantics.
                s.cursor.skip_soft_separators();
                match close {
                    SequenceClose::TopLevel => {
                        if s.cursor.at_end() {
                            break;
                        }
                    }
                    SequenceClose::Partition(part) => match s.cursor.peek() {
                        Some(Token::Partition(got)) if *got == part => {
                            s.cursor.next()?;
                            break;
                        }
                        Some(Token::Partition(got)) if matches!(got, '}' | ')' | ']') => {
                            return errf!("{}", err_msg);
                        }
                        Some(_) => {}
                        None => return errf!("{}", err_msg),
                    },
                }
                match mode {
                    SequenceMode::Values => {
                        let Some(item) = s.parse_item()? else {
                            return errf!("{}", err_msg);
                        };
                        item.checkretval()?;
                        items.push(item);
                    }
                    SequenceMode::Statements { .. } => {
                        if s.try_skip_redundant_terminal_end(terminated) {
                            continue;
                        }
                        if terminated {
                            return errf!("unreachable code after terminal statement");
                        }
                        let Some(item) = s.parse_item()? else {
                            return errf!("{}", err_msg);
                        };
                        terminated = Self::is_strong_terminator(&*item);
                        if item.as_any().downcast_ref::<IRNodeEmpty>().is_none() {
                            items.push(item);
                        }
                    }
                }
            }
            if let SequenceMode::Statements { keep_retval: true } = mode {
                match items.last() {
                    Some(last) if last.hasretval() => {}
                    Some(_) => return errf!("block expression must return a value"),
                    None => return errf!("block expression cannot be empty"),
                }
            }
            Ok(items)
        })
    }

    pub(super) fn parse_top_level_items(&mut self) -> Ret<Vec<Box<dyn IRNode>>> {
        self.parse_sequence(
            SequenceClose::TopLevel,
            SequenceMode::Statements { keep_retval: false },
            "top-level format invalid",
        )
    }

    pub(super) fn parse_value_container(
        &mut self,
        open: char,
        close: char,
        err_msg: &'static str,
    ) -> Ret<Vec<Box<dyn IRNode>>> {
        self.cursor.expect_partition(open, err_msg)?;
        self.parse_sequence(
            SequenceClose::Partition(close),
            SequenceMode::Values,
            err_msg,
        )
    }

    pub(super) fn parse_opened_value_container(
        &mut self,
        close: char,
        err_msg: &'static str,
    ) -> Ret<Vec<Box<dyn IRNode>>> {
        self.parse_sequence(
            SequenceClose::Partition(close),
            SequenceMode::Values,
            err_msg,
        )
    }

    pub(super) fn parse_key_value_container(
        &mut self,
        open: char,
        close: char,
        err_msg: &'static str,
    ) -> Ret<Vec<(Box<dyn IRNode>, Box<dyn IRNode>)>> {
        self.cursor.expect_partition(open, err_msg)?;
        self.with_expect_retval(true, |s| {
            let mut pairs = Vec::new();
            loop {
                s.cursor.skip_soft_separators();
                match s.cursor.peek() {
                    Some(Token::Partition(got)) if *got == close => {
                        s.cursor.next()?;
                        break;
                    }
                    Some(Token::Partition(got)) if matches!(got, '}' | ')' | ']') => {
                        return errf!("{}", err_msg);
                    }
                    Some(_) => {}
                    None => return errf!("{}", err_msg),
                }
                let Some(key) = s.parse_item()? else {
                    return errf!("{}", err_msg);
                };
                key.checkretval()?;
                s.cursor.expect_keyword(KwTy::Colon, err_msg)?;
                let Some(value) = s.parse_item()? else {
                    return errf!("{}", err_msg);
                };
                value.checkretval()?;
                pairs.push((key, value));
            }
            Ok(pairs)
        })
    }

    pub(super) fn parse_group_block(&mut self, keep_retval: bool) -> Ret<IRNodeArray> {
        let close = match self.cursor.next()? {
            Token::Partition('{') => '}',
            Token::Partition('(') => ')',
            Token::Partition('[') => ']',
            _ => return errf!("block format invalid"),
        };
        let subs = self.parse_sequence(
            SequenceClose::Partition(close),
            SequenceMode::Statements { keep_retval },
            "block format invalid",
        )?;
        IRNodeArray::from_vec(
            subs,
            maybe!(keep_retval, Bytecode::IRBLOCKR, Bytecode::IRBLOCK),
        )
    }

    pub(super) fn parse_required_item(&mut self) -> Ret<Box<dyn IRNode>> {
        self.with_expect_retval(true, |s| match s.parse_item()? {
            Some(item) => Ok(item),
            None => errf!("does not match next syntax node"),
        })
    }

    pub(super) fn try_skip_redundant_terminal_end(&mut self, terminated: bool) -> bool {
        if !terminated {
            return false;
        }
        self.cursor.eat_keyword(KwTy::End)
    }

    pub(super) fn is_strong_terminator(node: &dyn IRNode) -> bool {
        use Bytecode::*;
        if let Some(wrap) = node.as_any().downcast_ref::<IRNodeWrapOne>() {
            return Self::is_strong_terminator(&*wrap.node);
        }
        let op: Bytecode = std_mem_transmute!(node.bytecode());
        match op {
            RET | END | ERR | ABT | CODECALL => true,
            IRBLOCK | IRBLOCKR | IRLIST => node
                .as_any()
                .downcast_ref::<IRNodeArray>()
                .and_then(|arr| arr.subs.last())
                .is_some_and(|last| Self::is_strong_terminator(&**last)),
            IRIF | IRIFR => node
                .as_any()
                .downcast_ref::<IRNodeTriple>()
                .is_some_and(|ifn| {
                    Self::is_strong_terminator(&*ifn.suby) && Self::is_strong_terminator(&*ifn.subz)
                }),
            _ => false,
        }
    }
}
