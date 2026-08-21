use std::usize;

use Value::*;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum Value {
    #[default] Nil,          // type_id = 0
    Bool(bool),              //           1
    U8(u8),                  //           2
    U16(u16),                //           3
    U32(u32),                //           4
    U64(u64),                //           5
    U128(u128),              //           6
    Bytes(Vec<u8>),          //           8
    Address(field::Address), //           9
    // reserved              //           10
    // reserved              //           11  (formerly HeapSlice)
    // reserved              //           12
    Tuple(TupleItem),        //           13
    Compo(CompoItem),        //           14
    Handle(HandleItem),      //           15
}


impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CallArgsPack {
    Nil,
    Raw,
    Tuple,
}

pub fn classify_call_args_len(len: usize) -> VmrtRes<CallArgsPack> {
    if len > crate::MAX_FUNC_PARAM_LEN {
        return itr_err_fmt!(
            CastBeFnArgvFail,
            "func argv length cannot more than {}",
            crate::MAX_FUNC_PARAM_LEN
        );
    }
    Ok(match len {
        0 => CallArgsPack::Nil,
        1 => CallArgsPack::Raw,
        _ => CallArgsPack::Tuple,
    })
}

#[inline(always)]
pub(crate) fn add_size_saturating(total: usize, add: usize) -> usize {
    total.checked_add(add).unwrap_or(usize::MAX)
}

// VM semantic equality (contract-visible comparison, NOT Rust `==`): uints by value across widths,
// tuple/compo by content, cross-type errors. Map keys value-normalized; `scalar_bytes` serialization-only.
pub(crate) fn value_content_eq(lhs: &Value, rhs: &Value) -> VmrtRes<bool> {
    if lhs.is_uint() && rhs.is_uint() {
        return Ok(lhs.extract_u128()? == rhs.extract_u128()?);
    }
    if lhs.ty() != rhs.ty() {
        return itr_err_fmt!(
            Arithmetic,
            "cannot compare different types {:?} and {:?}",
            lhs,
            rhs
        );
    }
    match (lhs, rhs) {
        (Nil, Nil) => Ok(true),
        (Bool(l), Bool(r)) => Ok(l == r),
        (Bytes(l), Bytes(r)) => Ok(l == r),
        (Address(l), Address(r)) => Ok(l == r),
        (Tuple(l), Tuple(r)) => l.content_eq(r),
        (Compo(l), Compo(r)) => l.content_eq(r),
        (Handle(..), Handle(..)) => {
            itr_err_fmt!(Arithmetic, "cannot compare {:?} and {:?}", lhs, rhs)
        }
        (U8(l), U8(r)) => Ok(l == r),
        (U16(l), U16(r)) => Ok(l == r),
        (U32(l), U32(r)) => Ok(l == r),
        (U64(l), U64(r)) => Ok(l == r),
        (U128(l), U128(r)) => Ok(l == r),
        _ => itr_err_fmt!(Arithmetic, "cannot compare {:?} and {:?}", lhs, rhs),
    }
}

pub(crate) fn value_compare_fee(lhs: &Value, rhs: &Value, container_header_fee: usize) -> usize {
    match (lhs, rhs) {
        (Tuple(l), Tuple(r)) => l.compare_fee(r, container_header_fee),
        (Compo(l), Compo(r)) => l.compare_fee(r, container_header_fee),
        _ => add_size_saturating(lhs.dup_size(), rhs.dup_size()),
    }
}

impl Value {

    pub fn ty(&self) -> ValueTy {
        match self {
            Nil           => ValueTy::Nil,
            Bool(..)      => ValueTy::Bool,
            U8(..)        => ValueTy::U8,
            U16(..)       => ValueTy::U16,
            U32(..)       => ValueTy::U32,
            U64(..)       => ValueTy::U64,
            U128(..)      => ValueTy::U128,
            Bytes(..)     => ValueTy::Bytes,
            Address(..)   => ValueTy::Address,
            Tuple(..)     => ValueTy::Tuple,
            Compo(..)     => ValueTy::Compo,
            Handle(..)    => ValueTy::Handle,
        }
    }

    pub fn nil() -> Self {
        Nil
    }

    pub fn bool(b: bool) -> Self {
        Bool(b)
    }

    pub fn u8(n: u8) -> Self {
        U8(n)
    }

    pub fn empty_bytes() -> Self {
        Bytes(vec![])
    }
    
    pub fn bytes(b: Vec<u8>) -> Self {
        Bytes(b)
    }

    pub fn is_nil(&self) -> bool {
        match self {
            Nil => true,
            _ => false,
        }
    }
    
    pub fn is_bool(&self) -> bool {
        match self {
            Bool(..) => true,
            _ => false,
        }
    }


    pub fn is_uint(&self) -> bool {
        self.ty().is_uint()
    }

    pub fn is_bytes(&self) -> bool {
        match self {
            Bytes(..) => true,
            _ => false,
        }
    }

    pub fn is_addr(&self) -> bool {
        match self {
            Address(..) => true,
            _ => false,
        }
    }

    pub fn match_bytes(&self) -> Option<&[u8]> {
        match self {
            Bytes(buf) => Some(buf.as_slice()),
            _ => None,
        }
    }

    pub fn match_bytes_mut(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            Bytes(buf) => Some(buf),
            _ => None,
        }
    }

    pub fn match_compo(&self) -> Option<&CompoItem> {
        match self {
            Compo(compo) => Some(compo),
            _ => None,
        }
    }

    pub fn match_compo_mut(&mut self) -> Option<&mut CompoItem> {
        match self {
            Compo(compo) => Some(compo),
            _ => None,
        }
    }

    pub fn match_tuple(&self) -> Option<&TupleItem> {
        match self {
            Tuple(tuple) => Some(tuple),
            _ => None,
        }
    }

    pub fn container_len(&self) -> VmrtRes<usize> {
        if let Some(tuple) = self.match_tuple() {
            return Ok(tuple.len())
        }
        if let Some(compo) = self.match_compo() {
            return Ok(compo.len())
        }
        itr_err_code!(CompoOpNotMatch)
    }

    pub fn length(&self, cap: &SpaceCap) -> VmrtRes<Value> {
        if let Some(tuple) = self.match_tuple() {
            return tuple.length(cap)
        }
        if let Some(compo) = self.match_compo() {
            return compo.length(cap)
        }
        itr_err_code!(CompoOpNotMatch)
    }

    pub fn haskey(&self, k: Value) -> VmrtRes<Value> {
        if let Some(tuple) = self.match_tuple() {
            return tuple.haskey(k)
        }
        if let Some(compo) = self.match_compo() {
            return compo.haskey(k)
        }
        itr_err_code!(CompoOpNotMatch)
    }

    pub fn itemget(&self, k: Value) -> VmrtRes<Value> {
        if let Some(tuple) = self.match_tuple() {
            return tuple.itemget(k)
        }
        if let Some(compo) = self.match_compo() {
            return compo.itemget(k)
        }
        itr_err_code!(CompoOpNotMatch)
    }

    pub fn clone_unpack_items(&self) -> VmrtRes<Vec<Value>> {
        // UNPACK is a neutral sequence op: it can unpack Tuple or a plain Compo::List value.
        if let Some(tuple) = self.match_tuple() {
            return Ok(tuple.to_vec())
        }
        if let Some(compo) = self.match_compo() {
            return Ok(compo.list_ref()?.iter().cloned().collect())
        }
        itr_err_code!(CompoOpNotMatch)
    }

    pub fn pack_call_args<I>(items: I) -> VmrtRes<Self>
    where
        I: IntoIterator<Item = Value>,
    {
        Self::pack_tuple(items)
    }

    pub fn pack_tuple<I>(items: I) -> VmrtRes<Self>
    where
        I: IntoIterator<Item = Value>,
    {
        let mut items: Vec<_> = items.into_iter().collect();
        Ok(match classify_call_args_len(items.len())? {
            CallArgsPack::Nil => Self::Nil,
            CallArgsPack::Raw => items.pop().unwrap(),
            CallArgsPack::Tuple => Self::Tuple(TupleItem::new(items)?),
        })
    }

    pub fn into_compo(self) -> Option<CompoItem> {
        match self {
            Compo(compo) => Some(compo),
            _ => None,
        }
    }

    pub fn compo_ref(&self) -> VmrtRes<&CompoItem> {
        let Some(compo) = self.match_compo() else {
            return itr_err_code!(CompoOpNotMatch)
        };
        Ok(compo)
    }

    pub fn compo(&mut self) -> VmrtRes<&mut CompoItem> {
        let Some(compo) = self.match_compo_mut() else {
            return itr_err_code!(CompoOpNotMatch)
        };
        Ok(compo)
    }

    pub fn take_compo(self) -> VmrtRes<CompoItem> {
        let Some(compo) = self.into_compo() else {
            return itr_err_code!(CompoOpNotMatch)
        };
        Ok(compo)
    }


    /// Representation bytes for field serialization (fixed-width uint BE).
    /// Map keys use `extract_key_bytes` / `uint_key_bytes` instead — see `vm/doc/value-cast.md` §9.
    pub(crate) fn scalar_bytes(&self) -> Option<Vec<u8>> {
        match self {
            Nil => Some(vec![]),
            Bool(n) => Some(vec![encode_canonical_bool_byte(*n)]),
            U8(n) => Some(n.to_be_bytes().into()),
            U16(n) => Some(n.to_be_bytes().into()),
            U32(n) => Some(n.to_be_bytes().into()),
            U64(n) => Some(n.to_be_bytes().into()),
            U128(n) => Some(n.to_be_bytes().into()),
            Bytes(buf) => Some(buf.clone()),
            Address(a) => Some(a.encode()),
            _ => None,
        }
    }

    pub fn raw(&self) -> Vec<u8> {
        if let Some(bytes) = self.scalar_bytes() {
            return bytes;
        }
        match &self {
            // not support
            Tuple(..) => "{tuple value ...}".to_owned().into_bytes(),
            Compo(..) => "{compo value ...}".to_owned().into_bytes(),
            Handle(..) => "{handle value ...}".to_owned().into_bytes(),
            _ => unreachable!(),

        }
    }


    pub fn ty_num(&self) -> u8 {
        self.ty() as u8
    }

    pub fn val_size(&self) -> usize {
        match self {
            Nil      => 0,
            Bool(..) => 1,
            U8(..)   => 1,
            U16(..)  => 2,
            U32(..)  => 4,
            U64(..)  => 8,
            U128(..) => 16,
            Bytes(b) => b.len(),
            Address(..) => field::Address::SIZE,
            Tuple(a) => a.val_size(),
            Compo(c) => c.val_size(),
            Handle(..) => REF_DUP_SIZE,
        }
    }

    pub fn dup_size(&self) -> usize {
        match self {
            Nil      => 0,
            Bool(..) => 1,
            U8(..)   => 1,
            U16(..)  => 2,
            U32(..)  => 4,
            U64(..)  => 8,
            U128(..) => 16,
            Bytes(b) => b.len(),
            Address(..) => field::Address::SIZE,
            Tuple(..) | Compo(..) | Handle(..) => REF_DUP_SIZE,
        }
    }

    pub fn can_get_size(&self) -> VmrtRes<u16> {
        if let Tuple(..) | Compo(..) | Handle(..) = self {
            return itr_err_code!(ItemNoSize)
        }
        let n = self.val_size();
        if n >= u16::MAX as usize {
            return itr_err_code!(OutOfValueSize)
        }
        Ok(n as u16)
    }

    pub fn valid(self, cap: &SpaceCap) -> VmrtRes<Self> {
        match self.can_get_size() {
            Ok(n) => {
                if (n as usize) > cap.value_size {
                    itr_err_code!(OutOfValueSize)
                } else {
                    Ok(self)
                }
            }
            Err(ItrErr(ItrErrCode::ItemNoSize, _)) => Ok(self),
            Err(e) => Err(e),
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Nil =>          s!("nil"),
            Bool(true) =>   s!("true"),
            Bool(false) =>  s!("false"),
            U8(n) =>   format!("{}u8", n),
            U16(n) =>  format!("{}u16", n),
            U32(n) =>  format!("{}u32", n),
            U64(n) =>  format!("{}u64", n),
            U128(n) => format!("{}u128", n),
            Bytes(b) => match ascii_show_string(b) {
                Some(s) => format!("\"{}\"", s),
                _ => "0x".to_owned() + &hex::encode(b),
            },
            Address(a) => a.to_readable(),
            Tuple(a) => a.to_string(),
            Compo(a) => format!("compo({}){}", a.len(), a.to_string()),
            Handle(..) => s!("handle"),
        }
    }

    pub fn to_json(&self) -> String {
        match self {
            Nil =>          s!("null"),
            Bool(true) =>   s!("true"),
            Bool(false) =>  s!("false"),
            U8(n) =>   format!("{}", n),
            U16(n) =>  format!("{}", n),
            U32(n) =>  format!("{}", n),
            U64(n) =>  format!("{}", n),
            U128(n) => format!("{}", n),
            Bytes(b) => json_quote(&to_readable_or_base64(b)),
            Address(a) =>  format!("\"{}\"", a.to_readable()),
            Tuple(a) => a.to_json(),
            Compo(a) => a.to_json(),
            Handle(..) => s!(r#"{"$handle":true}"#),
        }
    }

    pub fn to_debug_json(&self) -> String {
        match self {
            Nil => s!("null"),
            Bool(true) => s!("true"),
            Bool(false) => s!("false"),
            U8(n) => format!("{}", n),
            U16(n) => format!("{}", n),
            U32(n) => format!("{}", n),
            U64(n) => format!("{}", n),
            U128(n) => format!("{}", n),
            Bytes(b) => match bytes_try_to_readable_string(b) {
                Some(s) => json_quote(&s),
                None => format!(r#"{{"$bytes_hex":"{}"}}"#, b.to_hex()),
            },
            Address(a) => json_quote(&a.to_readable()),
            Tuple(a) => a.to_debug_json(),
            Compo(a) => a.to_debug_json(),
            Handle(..) => s!(r#"{"$handle":true}"#),
        }
    }


}

