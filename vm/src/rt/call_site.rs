use crate::value::ContractAddress;
use field::Address;

use super::{
    Bytecode, EffectMode, FN_SIGN_WIDTH, FnSign, FrameBindings, ItrErr, ItrErrCode, MapItrErr,
    VmrtRes, checked_func_sign,
};

pub const CALL_BODY_WIDTH: usize = 6;
pub const SPLICE_BODY_WIDTH: usize = 5;

const CALL_TARGET_MASK: u8 = 0b0000_0111;
const CALL_EFFECT_MASK: u8 = 0b0001_1000;
const CALL_RESERVED_MASK: u8 = 0b1110_0000;
const SHORT_CALL_BODY_WIDTH: usize = FN_SIGN_WIDTH;
const SHORT_LIB_CALL_BODY_WIDTH: usize = 1 + FN_SIGN_WIDTH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallTarget {
    This,
    Self_,
    Upper,
    Super,
    Ext(u8),
    Use(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallSpec {
    Invoke {
        target: CallTarget,
        effect: EffectMode,
        selector: FnSign,
    },
    Splice {
        lib: u8,
        selector: FnSign,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnchorSource {
    StateThis,
    CodeOwner,
    Lib(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CandidateSet {
    AnchorOnly,
    ParentsOnly,
    AnchorAndParents,
}

impl CallSpec {
    fn anchor_semantics(&self) -> (AnchorSource, CandidateSet) {
        match *self {
            Self::Invoke {
                target: CallTarget::This,
                ..
            } => (AnchorSource::StateThis, CandidateSet::AnchorAndParents),
            Self::Invoke {
                target: CallTarget::Self_,
                ..
            } => (AnchorSource::CodeOwner, CandidateSet::AnchorOnly),
            Self::Invoke {
                target: CallTarget::Upper,
                ..
            } => (AnchorSource::CodeOwner, CandidateSet::AnchorAndParents),
            Self::Invoke {
                target: CallTarget::Super,
                ..
            } => (AnchorSource::CodeOwner, CandidateSet::ParentsOnly),
            Self::Invoke {
                target: CallTarget::Ext(lib),
                ..
            } => (AnchorSource::Lib(lib), CandidateSet::AnchorAndParents),
            Self::Invoke {
                target: CallTarget::Use(lib),
                ..
            } => (AnchorSource::Lib(lib), CandidateSet::AnchorOnly),
            Self::Splice { lib, .. } => (AnchorSource::Lib(lib), CandidateSet::AnchorOnly),
        }
    }

    pub const fn invoke(target: CallTarget, effect: EffectMode, selector: FnSign) -> Self {
        Self::Invoke {
            target,
            effect,
            selector,
        }
    }

    pub const fn splice(lib: u8, selector: FnSign) -> Self {
        Self::Splice { lib, selector }
    }

    pub const fn selector(&self) -> FnSign {
        match *self {
            Self::Invoke { selector, .. } | Self::Splice { selector, .. } => selector,
        }
    }

    pub const fn callee_effect(&self, current: EffectMode) -> EffectMode {
        match *self {
            Self::Invoke { effect, .. } => effect,
            Self::Splice { .. } => current,
        }
    }

    pub fn resolve_anchor(&self, bindings: &FrameBindings) -> VmrtRes<ContractAddress> {
        self.resolve_anchor_from(
            bindings.this_contract.as_ref(),
            bindings.code_contract.as_ref(),
            &bindings.lib_table,
        )
    }

    pub fn resolve_anchor_from(
        &self,
        this_contract: Option<&ContractAddress>,
        code_contract: Option<&ContractAddress>,
        lib_table: &[Address],
    ) -> VmrtRes<ContractAddress> {
        use ItrErrCode::*;
        let (src, _) = self.anchor_semantics();
        match src {
            AnchorSource::StateThis => this_contract
                .cloned()
                .ok_or_else(|| ItrErr::code(CallInvalid)),
            AnchorSource::CodeOwner => code_contract
                .cloned()
                .ok_or_else(|| ItrErr::code(CallInvalid)),
            AnchorSource::Lib(lib) => {
                let libidx = lib as usize;
                if libidx >= lib_table.len() {
                    return itr_err_code!(CallLibIdxOverflow);
                }
                ContractAddress::from_addr(lib_table[libidx]).map_ire(ContractAddrErr)
            }
        }
    }

    pub fn resolve_candidates(
        &self,
        anchor: &ContractAddress,
        parents: &[ContractAddress],
    ) -> Vec<ContractAddress> {
        let (_, set) = self.anchor_semantics();
        match set {
            CandidateSet::AnchorOnly => vec![anchor.clone()],
            CandidateSet::ParentsOnly => parents.to_vec(),
            CandidateSet::AnchorAndParents => {
                let mut out = Vec::with_capacity(1 + parents.len());
                out.push(anchor.clone());
                out.extend(parents.iter().cloned());
                out
            }
        }
    }

    pub fn needs_inherit_chain(&self) -> bool {
        let (_, set) = self.anchor_semantics();
        matches!(
            set,
            CandidateSet::ParentsOnly | CandidateSet::AnchorAndParents
        )
    }

    pub fn lib_index(&self) -> Option<u8> {
        let (src, _) = self.anchor_semantics();
        match src {
            AnchorSource::Lib(l) => Some(l),
            _ => None,
        }
    }

    pub const fn requires_external_visibility(&self) -> bool {
        matches!(
            *self,
            Self::Invoke {
                target: CallTarget::Ext(_),
                effect: EffectMode::Edit,
                ..
            }
        )
    }

    pub const fn switches_context(&self) -> bool {
        matches!(
            *self,
            Self::Invoke {
                target: CallTarget::Ext(_),
                ..
            }
        )
    }

    pub const fn callext(libidx: u8, selector: FnSign) -> Self {
        Self::invoke(CallTarget::Ext(libidx), EffectMode::Edit, selector)
    }

    pub const fn callextview(libidx: u8, selector: FnSign) -> Self {
        Self::invoke(CallTarget::Ext(libidx), EffectMode::View, selector)
    }

    pub const fn callextpure(libidx: u8, selector: FnSign) -> Self {
        Self::invoke(CallTarget::Ext(libidx), EffectMode::Pure, selector)
    }

    pub const fn calluseview(libidx: u8, selector: FnSign) -> Self {
        Self::invoke(CallTarget::Use(libidx), EffectMode::View, selector)
    }

    pub const fn callusepure(libidx: u8, selector: FnSign) -> Self {
        Self::invoke(CallTarget::Use(libidx), EffectMode::Pure, selector)
    }

    pub const fn callthis(selector: FnSign) -> Self {
        Self::invoke(CallTarget::This, EffectMode::Edit, selector)
    }

    pub const fn callself(selector: FnSign) -> Self {
        Self::invoke(CallTarget::Self_, EffectMode::Edit, selector)
    }

    pub const fn callupper(selector: FnSign) -> Self {
        Self::invoke(CallTarget::Upper, EffectMode::Edit, selector)
    }

    pub const fn callsuper(selector: FnSign) -> Self {
        Self::invoke(CallTarget::Super, EffectMode::Edit, selector)
    }

    pub const fn callselfview(selector: FnSign) -> Self {
        Self::invoke(CallTarget::Self_, EffectMode::View, selector)
    }

    pub const fn callselfpure(selector: FnSign) -> Self {
        Self::invoke(CallTarget::Self_, EffectMode::Pure, selector)
    }

    pub const fn codecall(libidx: u8, selector: FnSign) -> Self {
        Self::splice(libidx, selector)
    }
}

fn decode_call_target(tag: u8, arg: u8) -> VmrtRes<CallTarget> {
    Ok(match tag {
        0..=3 => {
            if arg != 0 {
                return itr_err_code!(ItrErrCode::CallInvalid);
            }
            match tag {
                0 => CallTarget::This,
                1 => CallTarget::Self_,
                2 => CallTarget::Upper,
                3 => CallTarget::Super,
                _ => unreachable!(),
            }
        }
        4 => CallTarget::Ext(arg),
        5 => CallTarget::Use(arg),
        _ => return itr_err_code!(ItrErrCode::CallInvalid),
    })
}

fn encode_call_target(target: CallTarget) -> (u8, u8) {
    match target {
        CallTarget::This => (0, 0),
        CallTarget::Self_ => (1, 0),
        CallTarget::Upper => (2, 0),
        CallTarget::Super => (3, 0),
        CallTarget::Ext(idx) => (4, idx),
        CallTarget::Use(idx) => (5, idx),
    }
}

fn decode_short_call_body(s: &[u8], target: CallTarget, effect: EffectMode) -> VmrtRes<CallSpec> {
    if s.len() != SHORT_CALL_BODY_WIDTH {
        return itr_err!(ItrErrCode::CastParamFail, "call shortcut body size invalid");
    }
    Ok(CallSpec::invoke(target, effect, checked_func_sign(s)?))
}

fn decode_short_indexed_call_body(
    s: &[u8],
    effect: EffectMode,
    target: fn(u8) -> CallTarget,
) -> VmrtRes<CallSpec> {
    if s.len() != SHORT_LIB_CALL_BODY_WIDTH {
        return itr_err!(ItrErrCode::CastParamFail, "call shortcut body size invalid");
    }
    Ok(CallSpec::invoke(
        target(s[0]),
        effect,
        checked_func_sign(&s[1..])?,
    ))
}

fn encode_short_call_body(selector: FnSign) -> Vec<u8> {
    selector.to_vec()
}

fn encode_short_lib_call_body(lib: u8, selector: FnSign) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + FN_SIGN_WIDTH);
    out.push(lib);
    out.extend_from_slice(&selector);
    out
}

pub fn encode_call_body(
    target: CallTarget,
    effect: EffectMode,
    selector: FnSign,
) -> [u8; CALL_BODY_WIDTH] {
    let (tag, arg) = encode_call_target(target);
    let mut out = [0u8; CALL_BODY_WIDTH];
    out[0] = tag
        | match effect {
            EffectMode::Edit => 0,
            EffectMode::View => 0b0000_1000,
            EffectMode::Pure => 0b0001_0000,
        };
    out[1] = arg;
    out[2..6].copy_from_slice(&selector);
    out
}

pub fn encode_splice_body(lib: u8, selector: FnSign) -> [u8; SPLICE_BODY_WIDTH] {
    let mut out = [0u8; SPLICE_BODY_WIDTH];
    out[0] = lib;
    out[1..].copy_from_slice(&selector);
    out
}

pub fn decode_user_call_site(inst: Bytecode, s: &[u8]) -> VmrtRes<CallSpec> {
    match inst {
        Bytecode::CODECALL => decode_splice_body(s),
        Bytecode::CALL => decode_call_body(s),
        Bytecode::CALLEXT => decode_short_indexed_call_body(s, EffectMode::Edit, CallTarget::Ext),
        Bytecode::CALLEXTVIEW => {
            decode_short_indexed_call_body(s, EffectMode::View, CallTarget::Ext)
        }
        Bytecode::CALLUSEVIEW => {
            decode_short_indexed_call_body(s, EffectMode::View, CallTarget::Use)
        }
        Bytecode::CALLUSEPURE => {
            decode_short_indexed_call_body(s, EffectMode::Pure, CallTarget::Use)
        }
        Bytecode::CALLTHIS => decode_short_call_body(s, CallTarget::This, EffectMode::Edit),
        Bytecode::CALLSELF => decode_short_call_body(s, CallTarget::Self_, EffectMode::Edit),
        Bytecode::CALLSUPER => decode_short_call_body(s, CallTarget::Super, EffectMode::Edit),
        Bytecode::CALLSELFVIEW => decode_short_call_body(s, CallTarget::Self_, EffectMode::View),
        Bytecode::CALLSELFPURE => decode_short_call_body(s, CallTarget::Self_, EffectMode::Pure),
        _ => itr_err_code!(ItrErrCode::CallInvalid),
    }
}

pub fn encode_user_call_site(call: CallSpec) -> (Bytecode, Vec<u8>) {
    match call {
        CallSpec::Splice { lib, selector } => (
            Bytecode::CODECALL,
            encode_splice_body(lib, selector).to_vec(),
        ),
        CallSpec::Invoke {
            target,
            effect,
            selector,
        } => match (target, effect) {
            (CallTarget::This, EffectMode::Edit) => {
                (Bytecode::CALLTHIS, encode_short_call_body(selector))
            }
            (CallTarget::Self_, EffectMode::Edit) => {
                (Bytecode::CALLSELF, encode_short_call_body(selector))
            }
            (CallTarget::Super, EffectMode::Edit) => {
                (Bytecode::CALLSUPER, encode_short_call_body(selector))
            }
            (CallTarget::Self_, EffectMode::View) => {
                (Bytecode::CALLSELFVIEW, encode_short_call_body(selector))
            }
            (CallTarget::Self_, EffectMode::Pure) => {
                (Bytecode::CALLSELFPURE, encode_short_call_body(selector))
            }
            (CallTarget::Ext(lib), EffectMode::Edit) => {
                (Bytecode::CALLEXT, encode_short_lib_call_body(lib, selector))
            }
            (CallTarget::Ext(lib), EffectMode::View) => (
                Bytecode::CALLEXTVIEW,
                encode_short_lib_call_body(lib, selector),
            ),
            (CallTarget::Use(lib), EffectMode::View) => (
                Bytecode::CALLUSEVIEW,
                encode_short_lib_call_body(lib, selector),
            ),
            (CallTarget::Use(lib), EffectMode::Pure) => (
                Bytecode::CALLUSEPURE,
                encode_short_lib_call_body(lib, selector),
            ),
            _ => (
                Bytecode::CALL,
                encode_call_body(target, effect, selector).to_vec(),
            ),
        },
    }
}

pub fn decode_call_body(s: &[u8]) -> VmrtRes<CallSpec> {
    if s.len() != CALL_BODY_WIDTH {
        return itr_err!(ItrErrCode::CastParamFail, "call body size invalid");
    }
    let flags = s[0];
    if flags & CALL_RESERVED_MASK != 0 {
        return itr_err_code!(ItrErrCode::CallInvalid);
    }
    let effect = match (flags & CALL_EFFECT_MASK) >> 3 {
        0 => EffectMode::Edit,
        1 => EffectMode::View,
        2 => EffectMode::Pure,
        _ => return itr_err_code!(ItrErrCode::CallInvalid),
    };
    let call = CallSpec::invoke(
        decode_call_target(flags & CALL_TARGET_MASK, s[1])?,
        effect,
        checked_func_sign(&s[2..6])?,
    );
    Ok(call)
}

pub const fn is_user_call_inst(inst: Bytecode) -> bool {
    matches!(
        inst,
        Bytecode::CODECALL
            | Bytecode::CALL
            | Bytecode::CALLEXT
            | Bytecode::CALLEXTVIEW
            | Bytecode::CALLUSEVIEW
            | Bytecode::CALLUSEPURE
            | Bytecode::CALLTHIS
            | Bytecode::CALLSELF
            | Bytecode::CALLSUPER
            | Bytecode::CALLSELFVIEW
            | Bytecode::CALLSELFPURE
    )
}

pub fn decode_splice_body(s: &[u8]) -> VmrtRes<CallSpec> {
    if s.len() != SPLICE_BODY_WIDTH {
        return itr_err!(ItrErrCode::CastParamFail, "splice body size invalid");
    }
    Ok(CallSpec::splice(s[0], checked_func_sign(&s[1..])?))
}
