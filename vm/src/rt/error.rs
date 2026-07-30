// error define
#[repr(u8)]
#[derive(Default, PartialEq, Debug, Clone, Copy)]
pub enum ItrErrCode {
    ContractError = 1u8,
    NotFindContract = 2,
    AbstTypeError = 3,
    CodeTypeError = 4,
    InheritError = 5,
    LibraryError = 6,
    CompileError = 7,
    ContractAddrErr = 8,
    ContractUpgradeErr = 9,

    CodeError = 11,
    CodeTooLong = 12,  // code length
    CodeOverflow = 13, // pc out of limit
    CodeEmpty = 14,
    CodeNotWithEnd = 15,
    JumpOverflow = 16,
    JumpInDataSeg = 17,

    IRNodeOverDepth = 20,

    InstInvalid = 21,    //
    InstDisabled = 22,   //
    ActDisabled = 23,    //
    InstNeverTouch = 24, //
    InstParamsErr = 25,  //

    OutOfGas = 31,
    OutOfStack = 32,
    OutOfLocal = 33,
    OutOfHeap = 34,
    OutOfMemory = 35,
    OutOfGlobal = 36,
    OutOfCallDepth = 37,
    OutOfLoadContract = 38,
    OutOfValueSize = 39,
    OutOfCompoLen = 40,

    GasError = 41,
    StackError = 42,
    LocalError = 43,
    HeapError = 44,
    MemoryError = 45,
    GlobalError = 46,
    StorageError = 47,
    OutOfLogSize = 48,
    LogError = 49,

    CallNotExist = 51,
    CallLibIdxOverflow = 52,
    CallInvalid = 53,
    CallExitInvalid = 54,
    CallInAbst = 56,
    CallOtherInMain = 57,
    CallLocInView = 58,
    CallInPure = 59,
    CallOtherInP2sh = 60,
    CallNoReturn = 61,
    CallNotExternal = 62,
    CallArgvTypeFail = 63,

    CastFail = 71,
    CastParamFail = 72,
    CastBeKeyFail = 73,
    CastBeUintFail = 74,
    CastBeBytesFail = 75,
    CastBeValueFail = 76,
    CastBeFnArgvFail = 77,
    CastBeCallDataFail = 78,
    CastBeFnRetvFail = 79,

    CompoOpInvalid = 80,
    CompoOpOverflow = 81,
    CompoToSerialize = 82,
    CompoOpNotMatch = 83,
    CompoPackError = 84,
    CompoNoFindItem = 85,

    Arithmetic = 90,
    BytesHandle = 91,
    NativeFuncError = 92,
    NativeEnvError = 93,
    NativeCtlError = 94,
    ActCallError = 95,  // unrecoverable action call failure
    ActCallRevert = 96, // recoverable action call failure
    ItemNoSize = 97,

    StorageKeyInvalid = 101,
    StorageKeyNotFind = 102,
    StorageExpired = 103,
    StorageNotExpired = 104,
    StoragePeriodErr = 105,
    StorageValSizeErr = 106,
    StorageRestoreNotMatch = 107,
    StorageNotActive = 108,
    StorageKeyExists = 109,
    StorageNilNotAllowed = 110,

    ThrowAbort = 151,    // user code call
    DeferredError = 152, // defer callback error
    IntentError = 153,
    ExecutionDeadline = 154,

    #[default]
    NeverError = 255,
}

#[derive(Debug)]
pub struct ItrErr(pub ItrErrCode, pub String);

impl std::fmt::Display for ItrErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}({}): {}", self.0, self.0 as u8, self.1)
    }
}

impl From<ItrErr> for TextError {
    fn from(e: ItrErr) -> TextError {
        let ee: ExecError = e.into();
        ee.to_string()
    }
}

impl From<ItrErr> for ExecError {
    fn from(e: ItrErr) -> ExecError {
        use ItrErrCode::*;
        let ItrErr(code, msg) = e;
        let text = format!("{:?}({}): {}", code, code as u8, msg);
        let is_revert = matches!(code, ActCallRevert);
        maybe!(is_revert, Error::revert(text), Error::fault(text))
    }
}

impl ItrErr {
    pub fn new(n: ItrErrCode, tip: &str) -> ItrErr {
        ItrErr(n, tip.to_string())
    }
    pub fn code(n: ItrErrCode) -> ItrErr {
        ItrErr(n, "".to_string())
    }
}

// VM Runtime Error
pub type VmrtRes<T> = Result<T, ItrErr>;
pub type VmrtErr = Result<(), ItrErr>;

pub trait IntoVmrt {
    fn into_vmrt(self) -> VmrtRes<Vec<u8>>;
}

impl IntoVmrt for Vec<u8> {
    fn into_vmrt(self) -> Result<Vec<u8>, ItrErr> {
        Ok(self)
    }
}

pub trait MapItrErr<T> {
    fn map_ire(self, ec: ItrErrCode) -> Result<T, ItrErr>;
}

pub trait MapItrStrErr<T> {
    fn map_ires(self, ec: ItrErrCode, es: TextError) -> Result<T, ItrErr>;
}

impl<T> MapItrErr<T> for Ret<T> {
    fn map_ire(self, ec: ItrErrCode) -> Result<T, ItrErr> {
        self.map_err(|e| ItrErr::new(ec, &e.to_string()))
    }
}

impl<T> MapItrStrErr<T> for Ret<T> {
    fn map_ires(self, ec: ItrErrCode, es: TextError) -> Result<T, ItrErr> {
        self.map_err(|e| ItrErr::new(ec, &(es + &e.to_string())))
    }
}

#[allow(unused)]
macro_rules! itr_err {
    ($code: expr, $tip: expr) => {
        Err(ItrErr($code, $tip.to_string()))
    };
}

#[allow(unused)]
macro_rules! itr_err_code {
    ($code: expr) => {
        Err(ItrErr($code, "".to_string()))
    };
}

#[allow(unused)]
macro_rules! itr_err_fmt {
    ($code: expr, $( $v: expr),+ ) => {
        Err(ItrErr::new($code, &format!($( $v ),+)))
    }
}
