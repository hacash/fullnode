#[repr(u8)]
#[derive(Debug, Clone, Copy, Default)]
pub enum CodeType {
    #[default]
    Bytecode = 0,
    IRNode = 1,
}

enum_try_from_u8_by_variant!(
    CodeType,
    ItrErrCode::CodeTypeError,
    "code type {} not found",
    [Bytecode, IRNode]
);

impl CodeType {
    pub const TYPE_MASK: u8 = 0b0000_0011;

    pub fn parse(n: u8) -> VmrtRes<Self> {
        Self::try_from_u8(n & Self::TYPE_MASK)
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CodeConf(u8);

impl CodeConf {
    pub const RESERVED_MASK: u8 = 0b1111_1100;

    pub fn parse(raw: u8) -> VmrtRes<Self> {
        if raw & Self::RESERVED_MASK != 0 {
            return itr_err_code!(ItrErrCode::CodeTypeError)
        }
        let _ = CodeType::parse(raw)?;
        Ok(Self(raw))
    }

    pub const fn from_type(code_type: CodeType) -> Self {
        Self(code_type as u8)
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    pub fn code_type(self) -> CodeType {
        match CodeType::try_from_u8(self.0 & CodeType::TYPE_MASK) {
            Ok(v) => v,
            Err(_) => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodePkg {
    pub conf: u8,
    pub data: Vec<u8>,
}

impl CodePkg {
    pub fn code_conf(&self) -> VmrtRes<CodeConf> {
        CodeConf::parse(self.conf)
    }

    pub fn code_type(&self) -> VmrtRes<CodeType> {
        Ok(self.code_conf()?.code_type())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ByteView {
    bytes: Arc<[u8]>,
    offset: usize,
    limit: usize,
}

impl ByteView {
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        let bytes: Arc<[u8]> = bytes.into();
        let limit = bytes.len();
        Self { bytes, offset: 0, limit }
    }

    pub fn from_arc(bytes: Arc<[u8]>) -> Self {
        let limit = bytes.len();
        Self { bytes, offset: 0, limit }
    }

    pub fn len(&self) -> usize {
        self.limit - self.offset
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[self.offset..self.limit]
    }

    pub fn slice(&self, offset: usize, limit: usize) -> VmrtRes<Self> {
        if offset > limit || limit > self.len() {
            return itr_err_code!(ItrErrCode::CodeOverflow)
        }
        Ok(Self {
            bytes: self.bytes.clone(),
            offset: self.offset + offset,
            limit: self.offset + limit,
        })
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

impl AsRef<[u8]> for ByteView {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<Vec<u8>> for ByteView {
    fn from(value: Vec<u8>) -> Self {
        Self::from_vec(value)
    }
}

impl From<Arc<[u8]>> for ByteView {
    fn from(value: Arc<[u8]>) -> Self {
        Self::from_arc(value)
    }
}
