//! Binary field-stream format — the wasm core's JSON-free transport layer.
//!
//! Both the response bodies and the complex request objects travel as a stream
//! of name-tagged fields:
//!
//! ```text
//! body  := field*
//! field := name(W2 len + utf8) + tag(u8) + payload
//! ```
//!
//! Tags (ASCII):
//! - `s` str, W4 len + utf8
//! - `u` u64, 8B BE
//! - `i` u32, 4B BE
//! - `t` u8, 1B
//! - `b` bool, 1B (0/1)
//! - `a` str array, W4 n + (W4 len + utf8)*
//! - `A` u64 array, W4 n + 8B*
//! - `B` u32 array, W4 n + 4B*
//! - `o` object, W4 len + nested body
//! - `O` object array, W4 n + (W4 len + nested body)*
//! - `n` null (no payload)
//!
//! Optional fields are simply omitted when absent (the reader fills defaults),
//! mirroring the JS/JSON convention where missing keys mean `undefined`.
//! Semantic values (amounts/addresses/hex) stay strings — parsing belongs to
//! Rust's consensus layer.

use sys::{Ret, errf};

// ================================ writer ================================

pub(crate) struct Bw {
    out: Vec<u8>,
}

impl Bw {
    pub(crate) fn new() -> Self {
        Self { out: Vec::new() }
    }

    fn raw(&mut self, name: &str, tag: u8, payload: &[u8]) {
        self.out
            .extend_from_slice(&(name.len() as u16).to_be_bytes());
        self.out.extend_from_slice(name.as_bytes());
        self.out.push(tag);
        self.out.extend_from_slice(payload);
    }

    fn len_prefixed(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + data.len());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
        out
    }

    pub(crate) fn str(&mut self, name: &str, v: &str) {
        self.raw(name, b's', &Self::len_prefixed(v.as_bytes()));
    }

    pub(crate) fn opt_str(&mut self, name: &str, v: Option<&str>) {
        if let Some(v) = v {
            self.str(name, v);
        }
    }

    pub(crate) fn u64(&mut self, name: &str, v: u64) {
        self.raw(name, b'u', &v.to_be_bytes());
    }

    pub(crate) fn opt_u64(&mut self, name: &str, v: Option<u64>) {
        if let Some(v) = v {
            self.u64(name, v);
        }
    }

    pub(crate) fn u32(&mut self, name: &str, v: u32) {
        self.raw(name, b'i', &v.to_be_bytes());
    }

    pub(crate) fn opt_u32(&mut self, name: &str, v: Option<u32>) {
        if let Some(v) = v {
            self.u32(name, v);
        }
    }

    pub(crate) fn u8(&mut self, name: &str, v: u8) {
        self.raw(name, b't', &[v]);
    }

    pub(crate) fn bool(&mut self, name: &str, v: bool) {
        self.raw(name, b'b', &[v as u8]);
    }

    pub(crate) fn opt_u8(&mut self, name: &str, v: Option<u8>) {
        if let Some(v) = v {
            self.u8(name, v);
        }
    }

    pub(crate) fn opt_bool(&mut self, name: &str, v: Option<bool>) {
        if let Some(v) = v {
            self.bool(name, v);
        }
    }

    pub(crate) fn str_arr(&mut self, name: &str, items: &[String]) {
        let mut payload = Vec::with_capacity(4 + items.len() * 4);
        payload.extend_from_slice(&(items.len() as u32).to_be_bytes());
        for item in items {
            payload.extend_from_slice(&Self::len_prefixed(item.as_bytes()));
        }
        self.raw(name, b'a', &payload);
    }

    pub(crate) fn u64_arr(&mut self, name: &str, items: &[u64]) {
        let mut payload = Vec::with_capacity(4 + items.len() * 8);
        payload.extend_from_slice(&(items.len() as u32).to_be_bytes());
        for item in items {
            payload.extend_from_slice(&item.to_be_bytes());
        }
        self.raw(name, b'A', &payload);
    }

    pub(crate) fn u32_arr(&mut self, name: &str, items: &[u32]) {
        let mut payload = Vec::with_capacity(4 + items.len() * 4);
        payload.extend_from_slice(&(items.len() as u32).to_be_bytes());
        for item in items {
            payload.extend_from_slice(&item.to_be_bytes());
        }
        self.raw(name, b'B', &payload);
    }

    /// Nested object field from an already-serialized nested body (each view
    /// type has `to_binary_body`, so nesting reuses it directly).
    pub(crate) fn obj_from_body(&mut self, name: &str, body: Vec<u8>) {
        self.raw(name, b'o', &Self::len_prefixed(&body));
    }

    /// Array of objects; each item is a pre-serialized nested body.
    pub(crate) fn obj_arr(&mut self, name: &str, items: &[Vec<u8>]) {
        let mut payload = Vec::with_capacity(4 + items.len() * 4);
        payload.extend_from_slice(&(items.len() as u32).to_be_bytes());
        for item in items {
            payload.extend_from_slice(&Self::len_prefixed(item));
        }
        self.raw(name, b'O', &payload);
    }

    pub(crate) fn into_inner(self) -> Vec<u8> {
        self.out
    }

    /// Re-serialize one parsed `BVal` under `name` (used by the nested
    /// `from_binary` path: parse an object value, re-encode it, then let the
    /// typed decoder parse it — small cost, keeps the reader API simple).
    pub(crate) fn push_raw(&mut self, name: &str, value: &BVal) {
        match value {
            BVal::Str(s) => self.str(name, s),
            BVal::U64(v) => self.u64(name, *v),
            BVal::U32(v) => self.u32(name, *v),
            BVal::U8(v) => self.u8(name, *v),
            BVal::Bool(v) => self.bool(name, *v),
            BVal::StrArr(items) => {
                let owned: Vec<String> = items.iter().map(|s| s.to_string()).collect();
                self.str_arr(name, &owned);
            }
            BVal::U64Arr(items) => self.u64_arr(name, items),
            BVal::U32Arr(items) => self.u32_arr(name, items),
            BVal::Obj(fields) => {
                let mut inner = Bw::new();
                for (n, v) in fields {
                    inner.push_raw(n, v);
                }
                self.obj_from_body(name, inner.into_inner());
            }
            BVal::ObjArr(items) => {
                let bodies: Vec<Vec<u8>> = items
                    .iter()
                    .map(|fields| {
                        let mut inner = Bw::new();
                        for (n, v) in fields {
                            inner.push_raw(n, v);
                        }
                        inner.into_inner()
                    })
                    .collect();
                self.obj_arr(name, &bodies);
            }
            BVal::Null => self.raw(name, b'n', &[]),
        }
    }
}

// ================================ reader ================================

#[derive(Clone)]
pub(crate) enum BVal<'a> {
    Str(&'a str),
    U64(u64),
    U32(u32),
    U8(u8),
    Bool(bool),
    StrArr(Vec<&'a str>),
    U64Arr(Vec<u64>),
    U32Arr(Vec<u32>),
    Obj(Vec<(&'a str, BVal<'a>)>),
    ObjArr(Vec<Vec<(&'a str, BVal<'a>)>>),
    Null,
}

impl<'a> BVal<'a> {
    pub(crate) fn str(&self) -> Ret<&'a str> {
        match self {
            BVal::Str(s) => Ok(s),
            _ => errf!("binary field is not a string"),
        }
    }

    pub(crate) fn u64(&self) -> Ret<u64> {
        match self {
            BVal::U64(v) => Ok(*v),
            _ => errf!("binary field is not a u64"),
        }
    }

    pub(crate) fn u32(&self) -> Ret<u32> {
        match self {
            BVal::U32(v) => Ok(*v),
            _ => errf!("binary field is not a u32"),
        }
    }

    pub(crate) fn u8(&self) -> Ret<u8> {
        match self {
            BVal::U8(v) => Ok(*v),
            _ => errf!("binary field is not a u8"),
        }
    }

    pub(crate) fn bool(&self) -> Ret<bool> {
        match self {
            BVal::Bool(v) => Ok(*v),
            _ => errf!("binary field is not a bool"),
        }
    }

    pub(crate) fn str_arr(&self) -> Ret<Vec<&'a str>> {
        match self {
            BVal::StrArr(v) => Ok(v.clone()),
            _ => errf!("binary field is not a string array"),
        }
    }

    pub(crate) fn u64_arr(&self) -> Ret<Vec<u64>> {
        match self {
            BVal::U64Arr(v) => Ok(v.clone()),
            _ => errf!("binary field is not a u64 array"),
        }
    }

    pub(crate) fn u32_arr(&self) -> Ret<Vec<u32>> {
        match self {
            BVal::U32Arr(v) => Ok(v.clone()),
            _ => errf!("binary field is not a u32 array"),
        }
    }

    pub(crate) fn obj(&self) -> Ret<Vec<(&'a str, BVal<'a>)>> {
        match self {
            BVal::Obj(v) => Ok(v.clone()),
            _ => errf!("binary field is not an object"),
        }
    }

    pub(crate) fn obj_arr(&self) -> Ret<Vec<Vec<(&'a str, BVal<'a>)>>> {
        match self {
            BVal::ObjArr(v) => Ok(v.clone()),
            _ => errf!("binary field is not an object array"),
        }
    }
}

/// Parse a body into a named-field list. Duplicate names error (consistent
/// with the JSON reader's duplicate-key rejection).
pub(crate) fn parse<'a>(buf: &'a [u8]) -> Ret<Vec<(&'a str, BVal<'a>)>> {
    let mut r = Br { buf, pos: 0 };
    let mut fields = Vec::new();
    while r.pos < buf.len() {
        let name = r.w2_str()?;
        if fields.iter().any(|(n, _)| *n == name) {
            return errf!("binary field {} is duplicated", name);
        }
        let tag = r.u8()?;
        let value = match tag {
            b's' => BVal::Str(r.w4_str()?),
            b'u' => BVal::U64(r.u64()?),
            b'i' => BVal::U32(r.u32()?),
            b't' => BVal::U8(r.u8()?),
            b'b' => BVal::Bool(r.bool()?),
            b'a' => {
                let n = r.u32()? as usize;
                r.reserve_items(n, 4)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(r.w4_str()?);
                }
                BVal::StrArr(items)
            }
            b'A' => {
                let n = r.u32()? as usize;
                r.reserve_items(n, 8)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(r.u64()?);
                }
                BVal::U64Arr(items)
            }
            b'B' => {
                let n = r.u32()? as usize;
                r.reserve_items(n, 4)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(r.u32()?);
                }
                BVal::U32Arr(items)
            }
            b'o' => {
                let len = r.u32()? as usize;
                let inner = r.take(len)?;
                BVal::Obj(parse(inner)?)
            }
            b'O' => {
                let n = r.u32()? as usize;
                r.reserve_items(n, 4)?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    let len = r.u32()? as usize;
                    let inner = r.take(len)?;
                    items.push(parse(inner)?);
                }
                BVal::ObjArr(items)
            }
            b'n' => BVal::Null,
            other => return errf!("unknown binary field tag {}", other as char),
        };
        fields.push((name, value));
    }
    Ok(fields)
}

/// Required / optional typed field accessors over a parsed field list.
pub(crate) fn req<'a>(fields: &'a [(&'a str, BVal<'a>)], name: &str) -> Ret<&'a BVal<'a>> {
    fields
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
        .ok_or_else(|| sys::Error::fault(format!("binary field {name} missing")))
}

pub(crate) fn opt<'a>(fields: &'a [(&'a str, BVal<'a>)], name: &str) -> Option<&'a BVal<'a>> {
    fields.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
}

struct Br<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Br<'a> {
    fn take(&mut self, n: usize) -> Ret<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.buf.len())
            .ok_or_else(|| sys::Error::fault("binary body truncated"))?;
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn reserve_items(&self, count: usize, min_size: usize) -> Ret<()> {
        if min_size != 0 && count > self.buf.len().saturating_sub(self.pos) / min_size {
            return errf!("binary array count {} exceeds remaining bytes", count);
        }
        Ok(())
    }

    fn u8(&mut self) -> Ret<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Ret<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Ret<u64> {
        let b = self.take(8)?;
        Ok(u64::from_be_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn bool(&mut self) -> Ret<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => errf!("binary bool invalid"),
        }
    }

    fn w2_str(&mut self) -> Ret<&'a str> {
        let len = self.u32_half()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes).map_err(|_| sys::Error::fault("binary string not utf8"))
    }

    fn w4_str(&mut self) -> Ret<&'a str> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes).map_err(|_| sys::Error::fault("binary string not utf8"))
    }

    /// W2 length (name prefix).
    fn u32_half(&mut self) -> Ret<u32> {
        let b = self.take(2)?;
        Ok(u32::from(u16::from_be_bytes([b[0], b[1]])))
    }
}
