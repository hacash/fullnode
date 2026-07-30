use super::*;

impl Syntax {
    pub(super) fn parse_expr_bp(&mut self, min_prec: u8) -> Ret<Box<dyn IRNode>> {
        let mut left = self.parse_prefix_expr()?;
        loop {
            if self.try_parse_postfix(&mut left)? {
                continue;
            }
            let Some(op) = self.peek_binary_operator() else {
                break;
            };
            if op.level() < min_prec {
                break;
            }
            self.cursor.next()?;
            let right = self.parse_expr_bp(op.next_min_prec())?;
            left.checkretval()?;
            right.checkretval()?;
            left = Box::new(IRNodeDouble {
                hrtv: true,
                inst: op.bytecode(),
                subx: left,
                suby: right,
            });
        }
        Ok(left)
    }

    fn parse_prefix_expr(&mut self) -> Ret<Box<dyn IRNode>> {
        let token = self.cursor.next()?;
        match token {
            Token::Identifier(id) => self.parse_identifier_expr(id),
            Token::Keyword(KwTy::This) => self.parse_identifier_expr("this".to_string()),
            Token::Keyword(KwTy::Self_) => self.parse_identifier_expr("self".to_string()),
            Token::Keyword(KwTy::Upper) => self.parse_identifier_expr("upper".to_string()),
            Token::Keyword(KwTy::Super) => self.parse_identifier_expr("super".to_string()),
            Token::Keyword(KwTy::Tuple) => self.parse_identifier_expr("tuple".to_string()),
            Token::Integer(n) => Ok(push_num(n)),
            Token::IntegerWithSuffix(n, kw) => self.parse_integer_with_suffix_literal(n, kw),
            Token::Character(b) => Ok(push_num(b as u128)),
            Token::Address(addr) => Ok(push_addr(addr)),
            Token::Bytes(bytes) => push_bytes(&bytes),
            Token::Partition('(') => self.parse_group_expr(),
            Token::Partition('[') => self.parse_list_literal(),
            Token::Partition('{') => {
                self.cursor.rewind_one();
                Ok(Box::new(self.parse_group_block(self.mode.expect_retval)?))
            }
            Token::Operator(OpTy::NOT) | Token::Keyword(KwTy::Not) => {
                let expr = self.parse_expr_bp(OpTy::NOT.next_min_prec())?;
                expr.checkretval()?;
                Ok(push_single(Bytecode::NOT, expr))
            }
            Token::Keyword(KwTy::If) => self.parse_if_expr(),
            Token::Keyword(KwTy::While) => self.parse_while_expr(),
            Token::Keyword(KwTy::Ext) => {
                self.parse_ext_receiver_call("ext(index) call format invalid")
            }
            Token::Keyword(KwTy::Call) => self.parse_generic_call_expr(),
            Token::Keyword(KwTy::CodeCall) => self.parse_codecall_expr(),
            Token::Keyword(KwTy::CallExt) => self.parse_short_lib_call_invoke(
                Bytecode::CALLEXT,
                "callext body",
                "callext target format invalid",
                CallSpec::callext,
            ),
            Token::Keyword(KwTy::CallExtView) => self.parse_short_lib_call_invoke(
                Bytecode::CALLEXTVIEW,
                "callextview body",
                "callextview target format invalid",
                CallSpec::callextview,
            ),
            Token::Keyword(KwTy::CallUseView) => self.parse_short_lib_call_invoke(
                Bytecode::CALLUSEVIEW,
                "calluseview body",
                "calluseview target format invalid",
                CallSpec::calluseview,
            ),
            Token::Keyword(KwTy::CallUsePure) => self.parse_short_lib_call_invoke(
                Bytecode::CALLUSEPURE,
                "callusepure body",
                "callusepure target format invalid",
                CallSpec::callusepure,
            ),
            Token::Keyword(KwTy::CallThis) => self.parse_short_local_call_invoke(
                Bytecode::CALLTHIS,
                "callthis body",
                "callthis target format invalid",
                CallSpec::callthis,
            ),
            Token::Keyword(KwTy::CallSelf) => self.parse_short_local_call_invoke(
                Bytecode::CALLSELF,
                "callself body",
                "callself target format invalid",
                CallSpec::callself,
            ),
            Token::Keyword(KwTy::CallSuper) => self.parse_short_local_call_invoke(
                Bytecode::CALLSUPER,
                "callsuper body",
                "callsuper target format invalid",
                CallSpec::callsuper,
            ),
            Token::Keyword(KwTy::CallSelfView) => self.parse_short_local_call_invoke(
                Bytecode::CALLSELFVIEW,
                "callselfview body",
                "callselfview target format invalid",
                CallSpec::callselfview,
            ),
            Token::Keyword(KwTy::CallSelfPure) => self.parse_short_local_call_invoke(
                Bytecode::CALLSELFPURE,
                "callselfpure body",
                "callselfpure target format invalid",
                CallSpec::callselfpure,
            ),
            Token::Keyword(KwTy::List) => self.parse_keyword_list_literal(),
            Token::Keyword(KwTy::Map) => self.parse_map_literal(),
            Token::Keyword(KwTy::Nil) => Ok(push_nil()),
            Token::Keyword(KwTy::True) => Ok(push_inst(Bytecode::PTRUE)),
            Token::Keyword(KwTy::False) => Ok(push_inst(Bytecode::PFALSE)),
            Token::Operator(op) => errf!("operator {:?} cannot start expression", op),
            other => errf!("unsupported token '{:?}'", other),
        }
    }

    fn parse_identifier_expr(&mut self, id: String) -> Ret<Box<dyn IRNode>> {
        if start_with_char(&id, '$') {
            let stripped = id.trim_start_matches('$');
            if stripped == "param" {
                return self.parse_param_stmt();
            }
        }
        match self.cursor.peek() {
            Some(Token::Partition('(')) => self.parse_free_call(id),
            Some(Token::Keyword(sep @ (KwTy::Dot | KwTy::Colon | KwTy::DColon))) => {
                let sep = *sep;
                self.cursor.next()?;
                self.parse_identifier_receiver_call(id, sep)
            }
            _ => self.link_symbol(&id),
        }
    }

    fn parse_integer_with_suffix_literal(&mut self, n: u128, kw: KwTy) -> Ret<Box<dyn IRNode>> {
        let num_node = push_num(n);
        let token = Token::Keyword(kw);
        let Some((ty, inst)) = Self::parse_uint_suffix_cast(&token) else {
            return Ok(num_node);
        };
        Self::check_uint_literal_overflow(n, ty)?;
        Ok(push_single(inst, num_node))
    }

    fn parse_group_expr(&mut self) -> Ret<Box<dyn IRNode>> {
        let expr = self.with_expect_retval(true, |s| s.parse_required_item())?;
        expr.checkretval()?;
        self.cursor
            .expect_partition(')', "(..) expression format invalid")?;
        Ok(Box::new(IRNodeWrapOne { node: expr }))
    }

    fn parse_list_container(
        &mut self,
        open: char,
        close: char,
        err_msg: &'static str,
    ) -> Ret<Box<dyn IRNode>> {
        let subs = self.parse_value_container(open, close, err_msg)?;
        self.build_list_node(subs)
    }

    fn parse_list_literal(&mut self) -> Ret<Box<dyn IRNode>> {
        let subs = self.parse_opened_value_container(']', "list literal format invalid")?;
        self.build_list_node(subs)
    }

    fn parse_keyword_list_literal(&mut self) -> Ret<Box<dyn IRNode>> {
        self.parse_list_container('{', '}', "list statement format invalid")
    }

    fn parse_map_literal(&mut self) -> Ret<Box<dyn IRNode>> {
        use Bytecode::*;
        let pairs = self.parse_key_value_container('{', '}', "map format invalid")?;
        let mut subs = Vec::with_capacity(pairs.len() * 2 + 2);
        for (key, value) in pairs {
            subs.push(key);
            subs.push(value);
        }
        if subs.is_empty() {
            return Ok(push_inst(NEWMAP));
        }
        let count = subs.len();
        subs.push(push_num(count as u128));
        subs.push(push_inst(PACKMAP));
        Ok(Box::new(Self::build_irlist(subs)?))
    }

    fn parse_if_expr(&mut self) -> Ret<Box<dyn IRNode>> {
        let cond = self.parse_required_item()?;
        cond.checkretval()?;
        let keep_retval = self.mode.expect_retval;
        let then_block = Box::new(self.parse_group_block(keep_retval)?);
        let mut node = IRNodeTriple {
            hrtv: keep_retval,
            inst: maybe!(keep_retval, Bytecode::IRIFR, Bytecode::IRIF),
            subx: cond,
            suby: then_block,
            subz: IRNodeLeaf::nop_box(),
        };
        if !self.cursor.eat_keyword(KwTy::Else) {
            if keep_retval {
                return errf!("if expression must have else branch");
            }
            return Ok(Box::new(node));
        }
        node.subz = if self.cursor.eat_keyword(KwTy::If) {
            self.with_expect_retval(keep_retval, |s| s.parse_if_expr())?
        } else {
            Box::new(self.parse_group_block(keep_retval)?)
        };
        Ok(Box::new(node))
    }

    fn parse_while_expr(&mut self) -> Ret<Box<dyn IRNode>> {
        let cond = self.parse_required_item()?;
        cond.checkretval()?;
        let body = self.with_loop_scope(|s| Ok(Box::new(s.parse_group_block(false)?)))?;
        Ok(push_double_box(Bytecode::IRWHILE, cond, body))
    }

    fn try_parse_postfix(&mut self, left: &mut Box<dyn IRNode>) -> Ret<bool> {
        match self.cursor.peek() {
            Some(Token::Partition('[')) => {
                self.cursor.next()?;
                left.checkretval()?;
                let key = self.parse_required_item()?;
                key.checkretval()?;
                self.cursor
                    .expect_partition(']', "item get statement format invalid")?;
                let receiver = std::mem::replace(left, push_empty());
                *left = Box::new(IRNodeDouble {
                    hrtv: true,
                    inst: Bytecode::ITEMGET,
                    subx: receiver,
                    suby: key,
                });
                return Ok(true);
            }
            Some(Token::Keyword(
                kw @ (KwTy::Assign | KwTy::AsgAdd | KwTy::AsgSub | KwTy::AsgMul | KwTy::AsgDiv),
            )) => {
                let token = Token::Keyword(*kw);
                self.cursor.next()?;
                let Some(name) = Self::assign_target_name(left.as_ref()) else {
                    return errf!("assign statement format invalid");
                };
                let value = self.parse_required_item()?;
                value.checkretval()?;
                *left = match token {
                    Token::Keyword(KwTy::Assign) => self.save_local(&name, value)?,
                    _ => self.assign_local(&name, value, token)?,
                };
                return Ok(true);
            }
            Some(Token::Keyword(KwTy::As)) => {
                self.cursor.next()?;
                left.checkretval()?;
                let token = self.cursor.next()?;
                let Some(target_ty) = Self::parse_scalar_value_ty(&token) else {
                    return errf!("<as> expression format invalid");
                };
                Self::check_literal_as_cast(left.as_ref(), target_ty)?;
                let same_ty =
                    crate::lang::ir_literal_ty(left.as_ref()).is_some_and(|ty| ty == target_ty);
                if !same_ty {
                    let node = std::mem::replace(left, push_empty());
                    *left = Self::build_cast_node(node, target_ty);
                }
                return Ok(true);
            }
            Some(Token::Keyword(KwTy::Is)) => {
                self.cursor.next()?;
                left.checkretval()?;
                let mut is_not = false;
                let mut token = self.cursor.next()?;
                if token == Token::Keyword(KwTy::Not) {
                    is_not = true;
                    token = self.cursor.next()?;
                }
                let source = std::mem::replace(left, push_empty());
                let mut node = match &token {
                    Token::Keyword(KwTy::List) => push_single(Bytecode::TLIST, source),
                    Token::Keyword(KwTy::Map) => push_single(Bytecode::TMAP, source),
                    Token::Identifier(name) => {
                        let Ok(ty) = ValueTy::from_name(name) else {
                            return errf!("<is> expression format invalid");
                        };
                        Self::build_is_node(source, ty)
                    }
                    _ => {
                        let Some(ty) = Self::parse_scalar_value_ty(&token).or_else(|| {
                            maybe!(token == Token::Keyword(KwTy::Nil), Some(ValueTy::Nil), None)
                        }) else {
                            return errf!("<is> expression format invalid");
                        };
                        Self::build_is_node(source, ty)
                    }
                };
                if is_not {
                    node = push_single(Bytecode::NOT, node);
                }
                *left = node;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    fn peek_binary_operator(&self) -> Option<OpTy> {
        match self.cursor.peek() {
            Some(Token::Operator(op)) if *op != OpTy::NOT => Some(*op),
            Some(Token::Keyword(KwTy::And)) => Some(OpTy::AND),
            Some(Token::Keyword(KwTy::Or)) => Some(OpTy::OR),
            _ => None,
        }
    }

    fn assign_target_name(node: &dyn IRNode) -> Option<String> {
        if let Some(ir) = node.as_any().downcast_ref::<IRNodeParam1>() {
            return Some(ir.as_text().clone());
        }
        node.as_any()
            .downcast_ref::<IRNodeLeaf>()
            .map(|ir| ir.as_text().clone())
    }

    fn parse_scalar_value_ty(token: &Token) -> Option<ValueTy> {
        match token {
            Token::Keyword(KwTy::Bool) => Some(ValueTy::Bool),
            Token::Keyword(KwTy::U8) => Some(ValueTy::U8),
            Token::Keyword(KwTy::U16) => Some(ValueTy::U16),
            Token::Keyword(KwTy::U32) => Some(ValueTy::U32),
            Token::Keyword(KwTy::U64) => Some(ValueTy::U64),
            Token::Keyword(KwTy::U128) => Some(ValueTy::U128),
            Token::Keyword(KwTy::Bytes) => Some(ValueTy::Bytes),
            Token::Keyword(KwTy::Address) => Some(ValueTy::Address),
            _ => None,
        }
    }

    fn parse_uint_suffix_cast(token: &Token) -> Option<(ValueTy, Bytecode)> {
        match token {
            Token::Keyword(KwTy::U8) => Some((ValueTy::U8, Bytecode::CU8)),
            Token::Keyword(KwTy::U16) => Some((ValueTy::U16, Bytecode::CU16)),
            Token::Keyword(KwTy::U32) => Some((ValueTy::U32, Bytecode::CU32)),
            Token::Keyword(KwTy::U64) => Some((ValueTy::U64, Bytecode::CU64)),
            Token::Keyword(KwTy::U128) => Some((ValueTy::U128, Bytecode::CU128)),
            _ => None,
        }
    }

    pub(crate) fn build_cast_node(left: Box<dyn IRNode>, ty: ValueTy) -> Box<dyn IRNode> {
        match ty {
            ValueTy::Bool | ValueTy::Address => {
                push_single_p1_hr(true, Bytecode::CTO, ty as u8, left)
            }
            ValueTy::U8 => push_single(Bytecode::CU8, left),
            ValueTy::U16 => push_single(Bytecode::CU16, left),
            ValueTy::U32 => push_single(Bytecode::CU32, left),
            ValueTy::U64 => push_single(Bytecode::CU64, left),
            ValueTy::U128 => push_single(Bytecode::CU128, left),
            ValueTy::Bytes => push_single(Bytecode::CBYTES, left),
            _ => never!(),
        }
    }

    fn build_is_node(subx: Box<dyn IRNode>, ty: ValueTy) -> Box<dyn IRNode> {
        match ty {
            ValueTy::Nil => push_single(Bytecode::TNIL, subx),
            _ => push_single_p1_hr(true, Bytecode::TIS, ty as u8, subx),
        }
    }

    pub(crate) fn extract_literal_value(node: &dyn IRNode) -> Ret<Option<Value>> {
        crate::lang::ir_literal_value(node)
    }

    fn check_uint_literal_overflow(n: u128, ty: ValueTy) -> Rerr {
        match ty {
            ValueTy::U8 if n > u8::MAX as u128 => {
                errf!("integer {} overflows u8 (max: {})", n, u8::MAX)
            }
            ValueTy::U16 if n > u16::MAX as u128 => {
                errf!("integer {} overflows u16 (max: {})", n, u16::MAX)
            }
            ValueTy::U32 if n > u32::MAX as u128 => {
                errf!("integer {} overflows u32 (max: {})", n, u32::MAX)
            }
            ValueTy::U64 if n > u64::MAX as u128 => {
                errf!("integer {} overflows u64 (max: {})", n, u64::MAX)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn check_literal_as_cast(node: &dyn IRNode, target_ty: ValueTy) -> Rerr {
        let Some(mut literal) = Self::extract_literal_value(node)? else {
            return Ok(());
        };
        if literal.ty().is_uint() && target_ty.is_uint() {
            let n = literal.extract_u128()?;
            Self::check_uint_literal_overflow(n, target_ty)?;
        }
        match target_ty {
            ValueTy::Bool => literal.cast_bool()?,
            ValueTy::U8 => literal.cast_u8()?,
            ValueTy::U16 => literal.cast_u16()?,
            ValueTy::U32 => literal.cast_u32()?,
            ValueTy::U64 => literal.cast_u64()?,
            ValueTy::U128 => literal.cast_u128()?,
            ValueTy::Bytes => literal.cast_bytes()?,
            ValueTy::Address => literal.cast_addr()?,
            _ => {}
        }
        Ok(())
    }
}
