

// error define
#[repr(u8)]
#[derive(Default, PartialEq, Debug, Clone, Copy)]
pub enum ItrErrCode {
    ContractError        = 1u8,
    NotFindContract      = 2,
    AbstTypeError        = 3,
    CodeTypeError        = 4,
    InheritsError        = 5,
    LibrarysError        = 6,
    ComplieError         = 7, 
    ContractAddrErr      = 8,
    ContractUpgradeErr   = 9,

    CodeError         = 11,
    CodeTooLong       = 12, // code length
    CodeOverflow      = 13, // pc out of limit
    CodeEmpty         = 14,
    CodeNotWithEnd    = 15,
    JumpOverflow      = 16,
    JumpInDataSeg     = 17,

    IRNodeOverDepth   = 20,
    
    InstInvalid       = 21, // 
    InstDisabled      = 22, // 
    ExtActDisabled    = 23, // 
    InstNeverTouch    = 24, // 
    InstParamsErr     = 25, // 
    
    OutOfGas          = 31,
    OutOfStack        = 32,
    OutOfLocal        = 33,
    OutOfHeap         = 34,
    OutOfMemory       = 35,
    OutOfGlobal       = 36,
    OutOfCallDepth    = 37,
    OutOfLoadContract = 38,
    OutOfValueSize    = 39,
    OutOfCompoLen     = 40,
    
    GasError          = 41,
    StackError        = 42,
    LocalError        = 43,
    HeapError         = 44,
    MemoryError       = 45,
    GlobalError       = 46,
    StorageError      = 47,
    LogError          = 48,
    
    CallNotExist      = 51,
    CallLibOverflow   = 52,
    CallInvalid       = 53,
    CallExitInvalid   = 54,
    CallInCodeCopy    = 55,
    CallInAbst        = 56,
    CallOtherInMain   = 57,
    CallLocInLib      = 58,
    CallLibInStatic   = 59,
    CallOtherInP2sh   = 60,
    CallNoReturn      = 61,
    CallNotPublic     = 62,
    CallArgvTypeFail  = 63,
    
    CastFail           = 71,
    CastParamFail      = 72,
    CastBeKeyFail      = 73,
    CastBeUintFail     = 74,
    CastBeBytesFail    = 75,
    CastBeValueFail    = 76,
    CastBeFnArgvFail   = 77,
    CastBeCallDataFail = 78,

    CompoOpInvalid    = 80,
    CompoOpOverflow   = 81,
    CompoToSerialize  = 82,
    CompoOpNotMatch   = 83,
    CompoPackError    = 84,
    CompoNoFindItem   = 85,
    
    Arithmetic        = 90,
    BytesHandle       = 91,
    NativeCallError   = 92,
    ExtActCallError   = 93,
    ItemNoSize        = 94,

    StorageKeyInvalid       = 101,
    StorageKeyNotFind       = 102,
    StorageExpired          = 103,
    StorageNotExpired       = 104,
    StoragePeriodErr        = 105,
    StorageValSizeErr       = 106,
    StorageRestoreNotMatch  = 107,

    ThrowAbort = 111, // user code call

    #[default] NeverError = 255,
}

#[derive(Debug)]
pub struct ItrErr(pub ItrErrCode, pub String);


impl ToString for ItrErr {
    fn to_string(&self) -> String {
        format!("{:?}({}): {}", self.0, self.0 as u8, self.1)
    }
}

impl From<ItrErr> for Error {
    fn from(e: ItrErr) -> Error {
        e.to_string()
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
    fn map_ires(self, ec: ItrErrCode, es: Error) -> Result<T, ItrErr>;   
}


impl<T> MapItrErr<T> for Ret<T> {
    fn map_ire(self, ec: ItrErrCode) -> Result<T, ItrErr> {
        self.map_err(|e|ItrErr::new(ec, &e.to_string()))
    }
}

impl<T> MapItrStrErr<T> for Ret<T> {
    fn map_ires(self, ec: ItrErrCode, es: Error) -> Result<T, ItrErr> {
        self.map_err(|e|ItrErr::new(ec, &(es + &e.to_string())))
    }
}

#[allow(unused)]
macro_rules! itr_err {
    ($code: expr, $tip: expr) => {
        Err(ItrErr($code, $tip.to_string()))
    }
}

#[allow(unused)]
macro_rules! itr_err_code {
    ($code: expr) => {
        Err(ItrErr($code, "".to_string()))
    }
}

#[allow(unused)]
macro_rules! itr_err_fmt {
    ($code: expr, $( $v: expr),+ ) => {
        Err(ItrErr::new($code, &format!($( $v ),+)))
    }
}
