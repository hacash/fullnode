use super::*;

enum ArgPackMode {
    Concat,
    Packed,
}

impl Syntax {
    pub(super) fn parse_free_call(&mut self, id: String) -> Ret<Box<dyn IRNode>> {
        if let Some((inst, fin_id, argc, hrtv)) = parse_fin_func_spec(&id)? {
            let argvs = self.parse_value_container('(', ')', "call argv format error")?;
            if argvs.len() != argc {
                return errf!(
                    "fin func call argv length must {} but got {}",
                    argc,
                    argvs.len()
                );
            }
            return build_fin_ir_func(inst, fin_id, argvs, hrtv);
        }

        if id == "tuple" {
            let args = self.parse_value_container('(', ')', "call argv format error")?;
            return pack_explicit_tuple(args);
        }

        if let Some((_, inst, pms, args, rs)) = pick_ir_func(&id) {
            let argvs = self.parse_value_container('(', ')', "call argv format error")?;
            self.track_special_ir_func(inst, &argvs)?;
            if pms + args != argvs.len() {
                return errf!(
                    "ir func call argv length must {} but got {}",
                    pms + args,
                    argvs.len()
                );
            }
            if rs > 1 {
                return errf!(
                    "ir func '{}' has unsupported multi-value return ({})",
                    id,
                    rs
                );
            }
            return build_ir_func(inst, pms, args, rs, argvs);
        }

        if let Some(idx) = NativeFunc::from_name(&id).map(|v| v.0) {
            let arg_nodes = self.parse_value_container('(', ')', "call argv format error")?;
            let Some(need) = NativeFunc::argv_len(idx) else {
                return errf!("unknown native func idx {}", idx);
            };
            if arg_nodes.len() != need {
                return errf!(
                    "native func '{}' requires {} argument(s) but got {}",
                    id,
                    need,
                    arg_nodes.len()
                );
            }
            let arg_nodes = self.apply_native_tar_uint_literal_coercion(&id, idx, arg_nodes)?;
            let argvs = concat_func_args(arg_nodes)?;
            return Ok(push_single_p1_hr(true, Bytecode::NTFUNC, idx, argvs));
        }

        if let Some(idx) = NativeCtl::from_name(&id).map(|v| v.0) {
            let (num, argvs) = self.parse_call_args(ArgPackMode::Packed)?;
            let Some(need) = NativeCtl::argv_len(idx) else {
                return errf!("unknown native ctl idx {}", idx);
            };
            if num != need {
                return errf!(
                    "native ctl '{}' requires {} argument(s) but got {}",
                    id,
                    need,
                    num
                );
            }
            return Ok(push_single_p1_hr(true, Bytecode::NTCTL, idx, argvs));
        }

        if let Some(idx) = NativeEnv::from_name(&id).map(|v| v.0) {
            let (num, _) = self.parse_call_args(ArgPackMode::Concat)?;
            if num != 0 {
                return errf!("native env '{}' takes no arguments but got {}", id, num);
            }
            return Ok(Box::new(IRNodeParam1 {
                hrtv: true,
                inst: Bytecode::NTENV,
                para: idx,
                text: s!(""),
            }));
        }

        if let Some((hrtv, inst, para, arg_len)) = pick_action_func(&id) {
            let (num, argvs) = self.parse_call_args(ArgPackMode::Concat)?;
            if num != arg_len {
                return errf!(
                    "action function '{}' argv length must {} but got {}",
                    id,
                    arg_len,
                    num
                );
            }
            if inst.metadata().input == 0 {
                return Ok(Box::new(IRNodeParam1 {
                    hrtv,
                    inst,
                    para,
                    text: s!(""),
                }));
            }
            return Ok(push_single_p1_hr(hrtv, inst, para, argvs));
        }

        errf!("unknown function '{}'", id)
    }

    fn apply_native_tar_uint_literal_coercion(
        &self,
        name: &str,
        idx: u8,
        argvs: Vec<Box<dyn IRNode>>,
    ) -> Ret<Vec<Box<dyn IRNode>>> {
        let Some(tys) = NativeFunc::tar_uint_tys(idx) else {
            return Ok(argvs);
        };
        if tys.len() != argvs.len() {
            return errf!(
                "native func '{}' tar_uint_tys length {} does not match arity {}",
                name,
                tys.len(),
                argvs.len()
            );
        }
        let mut out = Vec::with_capacity(argvs.len());
        for (i, (arg, &ty)) in argvs.into_iter().zip(tys.iter()).enumerate() {
            out.push(self.apply_native_tar_uint_literal_one(name, i, arg, ty)?);
        }
        Ok(out)
    }

    fn apply_native_tar_uint_literal_one(
        &self,
        name: &str,
        pos: usize,
        arg: Box<dyn IRNode>,
        ty: ValueTy,
    ) -> Ret<Box<dyn IRNode>> {
        if !ty.is_uint() {
            return Ok(arg);
        }
        let Some(literal) = Self::extract_literal_value(arg.as_ref())? else {
            return Ok(arg);
        };
        if !literal.ty().is_uint() {
            return Ok(arg);
        }
        if crate::lang::ir_node_effective_ty(arg.as_ref()) == Some(ty) {
            return Ok(arg);
        }
        if let Err(e) = Self::check_literal_as_cast(arg.as_ref(), ty) {
            return errf!(
                "native func '{}' argument {} literal requires type {}, {}",
                name,
                pos + 1,
                ty.name(),
                e
            );
        }
        Ok(Self::build_cast_node(arg, ty))
    }

    fn track_special_ir_func(&mut self, inst: Bytecode, argvs: &[Box<dyn IRNode>]) -> Rerr {
        if inst != Bytecode::ALLOC || argvs.len() != 1 {
            return Ok(());
        }
        let Some(value) = Self::extract_literal_value(argvs[0].as_ref())? else {
            return Ok(());
        };
        let n = value.extract_u128()?;
        if n > u8::MAX as u128 {
            return errf!("ir func call param error");
        }
        let slots = n as u8;
        if slots > self.local_alloc {
            self.local_alloc = slots;
        }
        Ok(())
    }

    fn parse_call_args(&mut self, mode: ArgPackMode) -> Ret<(usize, Box<dyn IRNode>)> {
        let argvs = self.parse_value_container('(', ')', "call argv format error")?;
        let len = argvs.len();
        let node = match mode {
            ArgPackMode::Concat => concat_func_args(argvs)?,
            ArgPackMode::Packed => pack_call_args(argvs)?,
        };
        Ok((len, node))
    }

    fn parse_packed_call_args(&mut self) -> Ret<Box<dyn IRNode>> {
        self.parse_call_args(ArgPackMode::Packed)
            .map(|(_, argv)| argv)
    }

    fn build_call_node(
        &mut self,
        call: CallSpec,
        allow_implicit_nil: bool,
    ) -> Ret<Box<dyn IRNode>> {
        let argv =
            if allow_implicit_nil && !matches!(self.cursor.peek(), Some(Token::Partition('('))) {
                push_nil()
            } else {
                self.parse_packed_call_args()?
            };
        match call {
            CallSpec::Invoke { .. } => push_user_invoke(call, argv),
            CallSpec::Splice { .. } => push_user_splice(call, argv),
        }
    }

    pub(super) fn parse_generic_call_expr(&mut self) -> Ret<Box<dyn IRNode>> {
        let call = self.parse_generic_call_spec()?;
        self.build_call_node(call, false)
    }

    fn parse_generic_call_spec(&mut self) -> Ret<CallSpec> {
        let first = self.cursor.next()?;
        if let Ok(body) = Self::parse_fixed_body_token::<CALL_BODY_WIDTH>(&first, "call body") {
            return decode_call_body(&body).map_err(|e| e.to_string());
        }
        let effect = Self::parse_call_effect_token(&first, "call effect format invalid")?;
        let head = self.cursor.next()?;
        let (target, selector) =
            self.parse_generic_call_target_selector(head, "call target format invalid")?;
        Ok(CallSpec::invoke(target, effect, selector))
    }

    pub(super) fn parse_codecall_expr(&mut self) -> Ret<Box<dyn IRNode>> {
        let call = self.parse_codecall_spec()?;
        self.build_call_node(call, true)
    }

    fn parse_codecall_spec(&mut self) -> Ret<CallSpec> {
        let first = self.cursor.next()?;
        if let Ok(body) = Self::parse_codecall_body_token(&first) {
            return decode_splice_body(&body).map_err(|e| e.to_string());
        }
        let (idx, selector) =
            self.parse_codecall_target_selector(first, "codecall target format invalid")?;
        Ok(CallSpec::codecall(idx, selector))
    }

    pub(super) fn parse_ext_receiver_call(
        &mut self,
        err_msg: &'static str,
    ) -> Ret<Box<dyn IRNode>> {
        let call = self.parse_ext_receiver_call_spec(err_msg)?;
        self.build_call_node(call, false)
    }

    fn parse_ext_receiver_call_spec(&mut self, err_msg: &'static str) -> Ret<CallSpec> {
        let idx = self.parse_lib_ctor_index(err_msg)?;
        let (target, effect) = match self.cursor.next()? {
            Token::Keyword(KwTy::Dot) => (CallTarget::Ext(idx), EffectMode::Edit),
            Token::Keyword(KwTy::Colon) => (CallTarget::Ext(idx), EffectMode::View),
            Token::Keyword(KwTy::DColon) => (CallTarget::Use(idx), EffectMode::Pure),
            _ => return errf!("{}", err_msg),
        };
        let selector = self.cursor.next()?;
        let sig = self.parse_call_selector_token(&selector)?;
        Ok(CallSpec::invoke(target, effect, sig))
    }

    pub(super) fn parse_identifier_receiver_call(
        &mut self,
        id: String,
        sep: KwTy,
    ) -> Ret<Box<dyn IRNode>> {
        let call = self.parse_identifier_receiver_call_spec(id, sep)?;
        self.build_call_node(call, false)
    }

    fn parse_identifier_receiver_call_spec(&mut self, id: String, sep: KwTy) -> Ret<CallSpec> {
        let selector = self.cursor.next()?;
        let sig = self.parse_call_selector_token(&selector)?;
        Ok(match sep {
            KwTy::Dot => match id.as_str() {
                "this" => CallSpec::invoke(CallTarget::This, EffectMode::Edit, sig),
                "self" => CallSpec::invoke(CallTarget::Self_, EffectMode::Edit, sig),
                "upper" => CallSpec::invoke(CallTarget::Upper, EffectMode::Edit, sig),
                "super" => CallSpec::invoke(CallTarget::Super, EffectMode::Edit, sig),
                _ => CallSpec::callext(self.link_lib(&id)?, sig),
            },
            KwTy::Colon | KwTy::DColon => {
                let effect = maybe!(sep == KwTy::DColon, EffectMode::Pure, EffectMode::View);
                match id.as_str() {
                    "self" => CallSpec::invoke(CallTarget::Self_, effect, sig),
                    "upper" => CallSpec::invoke(CallTarget::Upper, effect, sig),
                    "this" | "super" => {
                        return errf!("call expression after identifier format invalid");
                    }
                    _ => {
                        let idx = self.link_lib(&id)?;
                        let target = maybe!(
                            sep == KwTy::DColon,
                            CallTarget::Use(idx),
                            CallTarget::Ext(idx)
                        );
                        CallSpec::invoke(target, effect, sig)
                    }
                }
            }
            _ => never!(),
        })
    }

    fn parse_call_selector_token(&mut self, token: &Token) -> Ret<[u8; 4]> {
        match token {
            Token::Identifier(name) => self.parse_named_selector(name),
            _ => Self::parse_raw_selector_token(token),
        }
    }

    fn parse_named_selector(&mut self, name: &str) -> Ret<[u8; 4]> {
        if Self::is_ambiguous_hex_selector_ident(name) {
            return errf!(
                "ambiguous selector '{}', use 0x{} for raw selector",
                name,
                name
            );
        }
        let sig = calc_func_sign(name);
        self.emit.source_map.register_func(sig, name.to_string())?;
        Ok(sig)
    }

    fn is_ambiguous_hex_selector_ident(name: &str) -> bool {
        name.len() == 8 && name.bytes().all(|b| b.is_ascii_hexdigit())
    }

    fn parse_raw_selector_token(token: &Token) -> Ret<[u8; 4]> {
        match token {
            Token::Bytes(bytes) if bytes.len() == 4 => bytes
                .as_slice()
                .try_into()
                .map_err(|_| "function signature must be 4 bytes".into()),
            Token::Integer(n) if *n <= u32::MAX as u128 => Ok((*n as u32).to_be_bytes()),
            Token::Bytes(..) => errf!("function signature bytes must be exactly 4 bytes"),
            _ => {
                errf!("function signature must be function name, decimal <u32>, or 4-byte literal")
            }
        }
    }

    fn parse_fixed_body_token<const N: usize>(token: &Token, label: &str) -> Ret<[u8; N]> {
        match token {
            Token::Bytes(bytes) if bytes.len() == N => bytes
                .as_slice()
                .try_into()
                .map_err(|_| format!("{} must be fixed width", label).into()),
            Token::Bytes(..) => errf!("{} must be {} bytes", label, N),
            _ => errf!("{} must be {}-byte literal", label, N),
        }
    }

    fn parse_codecall_body_token(token: &Token) -> Ret<[u8; SPLICE_BODY_WIDTH]> {
        Self::parse_fixed_body_token(token, "codecall body")
    }

    fn parse_lib_index_token(token: &Token) -> Ret<u8> {
        let Token::Integer(n) = token else {
            return errf!("call index must be an integer");
        };
        if *n > u8::MAX as u128 {
            return errf!("call index overflow");
        }
        Ok(*n as u8)
    }

    fn parse_lib_ctor_index(&mut self, err_msg: &'static str) -> Ret<u8> {
        self.cursor.expect_partition('(', err_msg)?;
        let token = self.cursor.next()?;
        let idx = Self::parse_lib_index_token(&token)?;
        self.cursor.expect_partition(')', err_msg)?;
        Ok(idx)
    }

    fn parse_call_effect_token(token: &Token, err_msg: &'static str) -> Ret<EffectMode> {
        match token {
            Token::Keyword(KwTy::Edit) => Ok(EffectMode::Edit),
            Token::Keyword(KwTy::View) => Ok(EffectMode::View),
            Token::Keyword(KwTy::Pure) => Ok(EffectMode::Pure),
            _ => errf!("{}", err_msg),
        }
    }

    fn parse_call_target_head(&mut self, head: Token, err_msg: &'static str) -> Ret<CallTarget> {
        match head {
            Token::Keyword(KwTy::This) => Ok(CallTarget::This),
            Token::Keyword(KwTy::Self_) => Ok(CallTarget::Self_),
            Token::Keyword(KwTy::Upper) => Ok(CallTarget::Upper),
            Token::Keyword(KwTy::Super) => Ok(CallTarget::Super),
            Token::Keyword(KwTy::Ext) => Ok(CallTarget::Ext(self.parse_lib_ctor_index(err_msg)?)),
            Token::Keyword(KwTy::Use) => Ok(CallTarget::Use(self.parse_lib_ctor_index(err_msg)?)),
            Token::Identifier(id) => Ok(CallTarget::Ext(self.link_lib(&id)?)),
            Token::Integer(..) => Ok(CallTarget::Ext(Self::parse_lib_index_token(&head)?)),
            _ => errf!("{}", err_msg),
        }
    }

    fn parse_generic_call_target_selector(
        &mut self,
        head: Token,
        err_msg: &'static str,
    ) -> Ret<(CallTarget, [u8; 4])> {
        let target = self.parse_call_target_head(head, err_msg)?;
        self.cursor.expect_keyword(KwTy::Dot, err_msg)?;
        let selector = self.cursor.next()?;
        Ok((target, self.parse_call_selector_token(&selector)?))
    }

    fn parse_codecall_target_selector(
        &mut self,
        head: Token,
        err_msg: &'static str,
    ) -> Ret<(u8, [u8; 4])> {
        let idx = match head {
            Token::Identifier(id) => self.link_lib(&id)?,
            Token::Integer(..) => Self::parse_lib_index_token(&head)?,
            Token::Keyword(KwTy::Ext) => self.parse_lib_ctor_index(err_msg)?,
            _ => return errf!("{}", err_msg),
        };
        self.cursor.expect_keyword(KwTy::Dot, err_msg)?;
        let selector = self.cursor.next()?;
        Ok((idx, self.parse_call_selector_token(&selector)?))
    }

    fn parse_shortcut_lib_selector(
        &mut self,
        head: Token,
        err_msg: &'static str,
    ) -> Ret<(u8, [u8; 4])> {
        let idx = match head {
            Token::Identifier(id) => self.link_lib(&id)?,
            Token::Integer(..) => Self::parse_lib_index_token(&head)?,
            Token::Keyword(KwTy::Ext) => self.parse_lib_ctor_index(err_msg)?,
            _ => return errf!("{}", err_msg),
        };
        self.cursor.expect_keyword(KwTy::DColon, err_msg)?;
        let selector = self.cursor.next()?;
        Ok((idx, self.parse_call_selector_token(&selector)?))
    }

    fn parse_short_lib_call_spec<F>(
        &mut self,
        inst: Bytecode,
        body_label: &'static str,
        err_msg: &'static str,
        build: F,
    ) -> Ret<CallSpec>
    where
        F: FnOnce(u8, FnSign) -> CallSpec,
    {
        let first = self.cursor.next()?;
        if let Ok(body) = Self::parse_fixed_body_token::<5>(&first, body_label) {
            return decode_user_call_site(inst, &body).map_err(|e| e.to_string());
        }
        let (idx, selector) = self.parse_shortcut_lib_selector(first, err_msg)?;
        Ok(build(idx, selector))
    }

    pub(super) fn parse_short_lib_call_invoke<F>(
        &mut self,
        inst: Bytecode,
        body_label: &'static str,
        err_msg: &'static str,
        build: F,
    ) -> Ret<Box<dyn IRNode>>
    where
        F: FnOnce(u8, FnSign) -> CallSpec,
    {
        let call = self.parse_short_lib_call_spec(inst, body_label, err_msg, build)?;
        self.build_call_node(call, false)
    }

    fn parse_short_local_call_spec<F>(
        &mut self,
        inst: Bytecode,
        body_label: &'static str,
        err_msg: &'static str,
        build: F,
    ) -> Ret<CallSpec>
    where
        F: FnOnce(FnSign) -> CallSpec,
    {
        let first = self.cursor.next()?;
        if let Ok(body) = Self::parse_fixed_body_token::<4>(&first, body_label) {
            return decode_user_call_site(inst, &body).map_err(|e| e.to_string());
        }
        let idx = Self::parse_lib_index_token(&first).map_err(|_| err_msg.to_string())?;
        if idx != 0 {
            return errf!("{} must use 0::selector", body_label);
        }
        self.cursor.expect_keyword(KwTy::DColon, err_msg)?;
        let selector = self.cursor.next()?;
        Ok(build(self.parse_call_selector_token(&selector)?))
    }

    pub(super) fn parse_short_local_call_invoke<F>(
        &mut self,
        inst: Bytecode,
        body_label: &'static str,
        err_msg: &'static str,
        build: F,
    ) -> Ret<Box<dyn IRNode>>
    where
        F: FnOnce(FnSign) -> CallSpec,
    {
        let call = self.parse_short_local_call_spec(inst, body_label, err_msg, build)?;
        self.build_call_node(call, false)
    }
}

fn parse_fin_func_spec(id: &str) -> Ret<Option<(Bytecode, u8, usize, bool)>> {
    let Some(spec) = fin_source_call_spec(id).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    Ok(Some((
        spec.family,
        spec.id,
        spec.argc().map_err(|e| e.to_string())? as usize,
        true,
    )))
}

fn build_fin_ir_func(
    inst: Bytecode,
    fin_id: u8,
    argvs: Vec<Box<dyn IRNode>>,
    hrtv: bool,
) -> Ret<Box<dyn IRNode>> {
    build_param1_multi_node(hrtv, inst, fin_id, argvs).map_err(|e| e.to_string())
}

pub(super) fn build_log_irnode(
    inst: Bytecode,
    argvs: Vec<Box<dyn IRNode>>,
) -> Ret<Box<dyn IRNode>> {
    let mut argvs = std::collections::VecDeque::from(argvs);
    let take = |argvs: &mut std::collections::VecDeque<Box<dyn IRNode>>| argvs.pop_front().unwrap();
    Ok(match inst {
        Bytecode::LOG1 => Box::new(IRNodeDouble {
            hrtv: false,
            inst,
            subx: take(&mut argvs),
            suby: take(&mut argvs),
        }),
        Bytecode::LOG2 => Box::new(IRNodeTriple {
            hrtv: false,
            inst,
            subx: take(&mut argvs),
            suby: take(&mut argvs),
            subz: take(&mut argvs),
        }),
        Bytecode::LOG3 => Box::new(IRNodeQuad {
            hrtv: false,
            inst,
            subx: take(&mut argvs),
            suby: take(&mut argvs),
            subz: take(&mut argvs),
            subw: take(&mut argvs),
        }),
        Bytecode::LOG4 => Box::new(IRNodeQuint {
            hrtv: false,
            inst,
            suba: take(&mut argvs),
            subb: take(&mut argvs),
            subc: take(&mut argvs),
            subd: take(&mut argvs),
            sube: take(&mut argvs),
        }),
        _ => return errf!("log opcode invalid"),
    })
}

fn build_ir_func(
    inst: Bytecode,
    pms: usize,
    args: usize,
    rs: usize,
    argvs: Vec<Box<dyn IRNode>>,
) -> Ret<Box<dyn IRNode>> {
    fn take_immediate_u8(arg: &dyn IRNode) -> Ret<u8> {
        use Bytecode::*;
        if let Some(node) = arg.as_any().downcast_ref::<IRNodeWrapOne>() {
            return take_immediate_u8(&*node.node);
        }
        if let Some(node) = arg.as_any().downcast_ref::<IRNodeLeaf>() {
            return match node.inst {
                P0 => Ok(0),
                P1 => Ok(1),
                P2 => Ok(2),
                P3 => Ok(3),
                _ => errf!("ir func call param must be an immediate u8 literal"),
            };
        }
        if let Some(node) = arg.as_any().downcast_ref::<IRNodeParam1>() {
            if node.inst == PU8 {
                return Ok(node.para);
            }
        }
        errf!("ir func call param must be an immediate u8 literal")
    }

    let mut argvs = std::collections::VecDeque::from(argvs);
    let hrtv = rs == 1;
    let take = |argvs: &mut std::collections::VecDeque<Box<dyn IRNode>>| argvs.pop_front().unwrap();
    let take_param = |argvs: &mut std::collections::VecDeque<Box<dyn IRNode>>| -> Ret<u8> {
        let arg = take(argvs);
        take_immediate_u8(arg.as_ref())
    };
    if pms == 0 {
        return Ok(match args {
            0 => Box::new(IRNodeLeaf::notext(hrtv, inst)),
            1 => Box::new(IRNodeSingle {
                hrtv,
                inst,
                subx: take(&mut argvs),
            }),
            2 => Box::new(IRNodeDouble {
                hrtv,
                inst,
                subx: take(&mut argvs),
                suby: take(&mut argvs),
            }),
            3 => Box::new(IRNodeTriple {
                hrtv,
                inst,
                subx: take(&mut argvs),
                suby: take(&mut argvs),
                subz: take(&mut argvs),
            }),
            4 => Box::new(IRNodeQuad {
                hrtv,
                inst,
                subx: take(&mut argvs),
                suby: take(&mut argvs),
                subz: take(&mut argvs),
                subw: take(&mut argvs),
            }),
            5 => Box::new(IRNodeQuint {
                hrtv,
                inst,
                suba: take(&mut argvs),
                subb: take(&mut argvs),
                subc: take(&mut argvs),
                subd: take(&mut argvs),
                sube: take(&mut argvs),
            }),
            _ => return errf!("cannot match ir call type: params({}), args({})", pms, args),
        });
    }
    if pms == 1 {
        let para = take_param(&mut argvs)?;
        return Ok(match args {
            0 => Box::new(IRNodeParam1 {
                hrtv,
                inst,
                para,
                text: s!(""),
            }),
            1 => push_single_p1_hr(hrtv, inst, para, take(&mut argvs)),
            _ => return errf!("cannot match ir call type: params({}), args({})", pms, args),
        });
    }
    if pms == 2 {
        let p1 = take_param(&mut argvs)?;
        let p2 = take_param(&mut argvs)?;
        return Ok(match args {
            0 => Box::new(IRNodeParam2 {
                hrtv,
                inst,
                para: [p1, p2],
            }),
            1 => Box::new(IRNodeParam2Single {
                hrtv,
                inst,
                para: [p1, p2],
                subx: take(&mut argvs),
            }),
            _ => return errf!("cannot match ir call type: params({}), args({})", pms, args),
        });
    }
    errf!("cannot match ir call type: params({}), args({})", pms, args)
}

fn concat_func_args(mut args: Vec<Box<dyn IRNode>>) -> Ret<Box<dyn IRNode>> {
    let Some(mut res) = args.pop() else {
        return Ok(push_inst(Bytecode::PNBUF));
    };
    while let Some(arg) = args.pop() {
        res = Box::new(IRNodeDouble {
            hrtv: true,
            inst: Bytecode::CAT,
            subx: arg,
            suby: res,
        });
    }
    Ok(res)
}

fn pack_call_args(mut subs: Vec<Box<dyn IRNode>>) -> Ret<Box<dyn IRNode>> {
    match crate::value::classify_call_args_len(subs.len()).map_err(|e| e.to_string())? {
        crate::value::CallArgsPack::Nil => Ok(push_nil()),
        crate::value::CallArgsPack::Raw => Ok(subs.pop().unwrap()),
        crate::value::CallArgsPack::Tuple => {
            let len = subs.len();
            subs.push(push_num(len as u128));
            subs.push(push_inst(Bytecode::PACKTUPLE));
            Ok(Box::new(Syntax::build_irlist(subs)?))
        }
    }
}

fn pack_explicit_tuple(mut subs: Vec<Box<dyn IRNode>>) -> Ret<Box<dyn IRNode>> {
    let len = subs.len();
    if len == 0 {
        return errf!("tuple() cannot be empty");
    }
    subs.push(push_num(len as u128));
    subs.push(push_inst(Bytecode::PACKTUPLE));
    Ok(Box::new(Syntax::build_irlist(subs)?))
}

fn pick_action_func(id: &str) -> Option<(bool, Bytecode, u8, usize)> {
    if let Some(x) = ACTION_ENV_DEFS.iter().find(|f| f.1 == id) {
        return Some((true, Bytecode::ACTENV, x.0, x.3));
    }
    if let Some(x) = ACTION_VIEW_DEFS.iter().find(|f| f.1 == id) {
        return Some((true, Bytecode::ACTVIEW, x.0, x.3));
    }
    if let Some(x) = ACTION_DEFS.iter().find(|f| f.1 == id) {
        return Some((false, Bytecode::ACTION, x.0, x.3));
    }
    None
}
