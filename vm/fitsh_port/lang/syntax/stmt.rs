use super::*;

impl Syntax {
    pub(super) fn parse_item(&mut self) -> Ret<Option<Box<dyn IRNode>>> {
        if self.cursor.at_end() {
            return Ok(None);
        }
        let item = match self.cursor.peek() {
            Some(Token::Keyword(KwTy::Const)) => {
                self.cursor.next()?;
                self.parse_const_stmt()?
            }
            Some(Token::Keyword(KwTy::Var)) => {
                self.cursor.next()?;
                self.parse_local_stmt(SlotKind::Var)?
            }
            Some(Token::Keyword(KwTy::Let)) => {
                self.cursor.next()?;
                self.parse_local_stmt(SlotKind::Let)?
            }
            Some(Token::Keyword(KwTy::Bind)) => {
                self.cursor.next()?;
                self.parse_bind_stmt()?
            }
            Some(Token::Keyword(KwTy::Lib)) => {
                self.cursor.next()?;
                self.parse_lib_stmt()?
            }
            Some(Token::Keyword(KwTy::Param)) => {
                self.cursor.next()?;
                self.parse_param_stmt()?
            }
            Some(Token::Keyword(KwTy::ByteCode)) => {
                self.cursor.next()?;
                self.parse_bytecode_stmt()?
            }
            Some(Token::Keyword(KwTy::Log)) => {
                self.cursor.next()?;
                self.parse_log_stmt()?
            }
            Some(Token::Keyword(KwTy::Print)) => {
                self.cursor.next()?;
                self.parse_single_value_stmt(Bytecode::PRT, "print arguments must be expressions with return values; do not use bind/var declarations directly")?
            }
            Some(Token::Keyword(KwTy::Assert)) => {
                self.cursor.next()?;
                self.parse_single_value_stmt(
                    Bytecode::AST,
                    "assert arguments must be expressions with return values",
                )?
            }
            Some(Token::Keyword(KwTy::Throw)) => {
                self.cursor.next()?;
                self.parse_single_value_stmt(
                    Bytecode::ERR,
                    "throw arguments must be expressions with return values",
                )?
            }
            Some(Token::Keyword(KwTy::Return)) => {
                self.cursor.next()?;
                self.parse_single_value_stmt(
                    Bytecode::RET,
                    "return arguments must be expressions with return values",
                )?
            }
            Some(Token::Keyword(KwTy::Abort)) => {
                self.cursor.next()?;
                push_inst_noret(Bytecode::ABT)
            }
            Some(Token::Keyword(KwTy::End)) => {
                self.cursor.next()?;
                push_inst_noret(Bytecode::END)
            }
            Some(Token::Keyword(KwTy::Break)) => {
                self.cursor.next()?;
                if self.mode.expect_retval {
                    return errf!("break statement cannot be used as expression");
                }
                if self.mode.loop_depth == 0 {
                    return errf!("break can only be used inside while loop");
                }
                push_inst_noret(Bytecode::IRBREAK)
            }
            Some(Token::Keyword(KwTy::Continue)) => {
                self.cursor.next()?;
                if self.mode.expect_retval {
                    return errf!("continue statement cannot be used as expression");
                }
                if self.mode.loop_depth == 0 {
                    return errf!("continue can only be used inside while loop");
                }
                push_inst_noret(Bytecode::IRCONTINUE)
            }
            _ => self.parse_expr_bp(0)?,
        };
        Ok(Some(item))
    }

    pub(super) fn parse_param_stmt(&mut self) -> Ret<Box<dyn IRNode>> {
        self.cursor.expect_partition('{', "param format invalid")?;
        let mut params = 0u8;
        let mut names = Vec::new();
        loop {
            self.cursor.skip_soft_separators();
            match self.cursor.next()? {
                Token::Partition('}') => break,
                Token::Identifier(id) => {
                    if params == u8::MAX {
                        return errf!("param index overflow");
                    }
                    names.push(id.clone());
                    self.bind_slot(id, params, SlotKind::Param)?;
                    params += 1;
                }
                _ => return errf!("param format invalid"),
            }
        }
        if params == 0 {
            return errf!("at least one param required");
        }
        self.emit.source_map.register_param_names(names)?;
        self.emit.source_map.register_param_prelude_count(params)?;
        Self::build_param_prelude(params as usize, false)
    }

    fn parse_local_stmt(&mut self, kind: SlotKind) -> Ret<Box<dyn IRNode>> {
        let Token::Identifier(name) = self.cursor.next()? else {
            return errf!(
                "{} statement format invalid",
                maybe!(matches!(kind, SlotKind::Let), "let", "var")
            );
        };
        let explicit_idx = match self.cursor.peek() {
            Some(Token::Identifier(alias)) => Self::parse_slot_alias(alias),
            _ => None,
        };
        if explicit_idx.is_some() {
            self.cursor.next()?;
        }
        let value = if self.cursor.eat_keyword(KwTy::Assign) {
            let expr = self.parse_required_item()?;
            if !expr.hasretval() {
                return errf!(
                    "{} initializer must be expressions with return values; do not use bind/var/let declarations directly",
                    maybe!(matches!(kind, SlotKind::Let), "let", "var")
                );
            }
            Some(expr)
        } else {
            None
        };
        match (explicit_idx, value) {
            (idx, Some(value)) => self.bind_slot_with_value(name, idx, value, kind),
            (Some(idx), None) => self.bind_slot(name, idx, kind),
            _ => errf!(
                "{} statement format invalid",
                maybe!(matches!(kind, SlotKind::Let), "let", "var")
            ),
        }
    }

    fn parse_bind_stmt(&mut self) -> Ret<Box<dyn IRNode>> {
        let Token::Identifier(name) = self.cursor.next()? else {
            return errf!("bind statement format invalid");
        };
        self.cursor
            .expect_keyword(KwTy::Assign, "bind statement format invalid")?;
        let expr = self.parse_required_item()?;
        expr.checkretval()?;
        self.bind_macro(name, expr)?;
        Ok(push_empty())
    }

    fn parse_const_stmt(&mut self) -> Ret<Box<dyn IRNode>> {
        let Token::Identifier(name) = self.cursor.next()? else {
            return errf!("const statement format invalid");
        };
        let explicit_ty = if self.cursor.eat_keyword(KwTy::Colon) {
            let ty_token = self.cursor.next()?;
            let Some(ty) = crate::lang::parse_const_value_ty(&ty_token) else {
                return errf!("const statement type invalid");
            };
            Some(ty)
        } else {
            None
        };
        self.cursor
            .expect_keyword(KwTy::Assign, "const statement format invalid")?;
        let token = self.cursor.next()?;
        let literal = crate::lang::parse_const_literal(token, explicit_ty)?;
        self.register_const_symbol(name.clone(), clone_box(literal.node.as_ref()))?;
        self.emit.source_map.register_const(name, literal.display)?;
        Ok(push_empty())
    }

    fn parse_lib_stmt(&mut self) -> Ret<Box<dyn IRNode>> {
        let Token::Identifier(name) = self.cursor.next()? else {
            return errf!("lib statement format invalid");
        };
        self.cursor
            .expect_keyword(KwTy::Assign, "lib statement format invalid")?;
        let Token::Integer(idx) = self.cursor.next()? else {
            return errf!("lib statement format invalid");
        };
        if idx > u8::MAX as u128 {
            return errf!("lib statement link index overflow");
        }
        let addr = if self.cursor.eat_keyword(KwTy::Colon) {
            let Token::Address(addr) = self.cursor.next()? else {
                return errf!("lib statement format invalid");
            };
            Some(addr as FieldAddress)
        } else {
            None
        };
        self.bind_lib(name, idx as u8, addr)?;
        Ok(push_empty())
    }

    fn parse_log_stmt(&mut self) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        let (open, close) = match self.cursor.peek() {
            Some(Token::Partition('(')) => ('(', ')'),
            Some(Token::Partition('{')) => ('{', '}'),
            Some(Token::Partition('[')) => ('[', ']'),
            _ => return errf!("log argv number invalid"),
        };
        let subs = self.parse_value_container(open, close, "log argv number invalid")?;
        let inst = match subs.len() {
            2 => LOG1,
            3 => LOG2,
            4 => LOG3,
            5 => LOG4,
            _ => return errf!("log argv number invalid"),
        };
        super::call::build_log_irnode(inst, subs)
    }

    fn parse_single_value_stmt(
        &mut self,
        inst: Bytecode,
        err_msg: &'static str,
    ) -> Ret<Box<dyn IRNode>> {
        let expr = self.parse_required_item()?;
        if !expr.hasretval() {
            return errf!("{}", err_msg);
        }
        Ok(push_single_noret(inst, expr))
    }

    fn parse_bytecode_stmt(&mut self) -> Ret<Box<dyn IRNode>> {
        self.cursor
            .expect_partition('{', "bytecode format invalid")?;
        let mut codes = Vec::new();
        loop {
            match self.cursor.next()? {
                Token::Partition('}') => break,
                Token::Identifier(id) => {
                    let Some(code) = Bytecode::parse(&id) else {
                        return errf!("bytecode {} not found", id);
                    };
                    codes.push(code as u8);
                }
                Token::Bytes(bytes) => codes.extend(bytes),
                Token::Integer(n) if n <= u8::MAX as u128 => codes.push(n as u8),
                _ => return errf!("bytecode format invalid"),
            }
        }
        // Surface-level `bytecode { ... }` must produce a runtime-safe
        // fragment: no IR-only opcodes, no absolute jumps, no misaligned
        // params. Routing through `IRNodeBytecodes::new` is the single
        // construction-time gate, matching the serialized-IR parser entry.
        let node = IRNodeBytecodes::new(codes).map_err(|e| e.to_string())?;
        Ok(Box::new(node))
    }
}
