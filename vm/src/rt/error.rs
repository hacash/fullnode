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

    // Canonical state backend read / persisted state decode failures. These
    // must stay fatal across the `ItrErr -> sys::Error` boundary (they map to
    // `Error::abort` with state-layer string codes) and must never be
    // downgraded to ordinary execution failures.
    StateReadFailed = 161,
    StateDecodeFailed = 162,

    // Reserved error code: returned by the codec-only entry stubs that were
    // removed when the execution engine became always-compiled. Retained so
    // the VM error-code space stays stable; no current path produces it.
    CodecOnlyUnsupported = 163,

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

impl From<ItrErr> for Error {
    fn from(e: ItrErr) -> Error {
        use ItrErrCode::*;
        let ItrErr(code, msg) = e;
        let text = format!("{:?}({}): {}", code, code as u8, msg);
        match code {
            ActCallRevert => Error::revert(text),
            StateReadFailed => Error::abort(text).with_code(base::STATE_READ_FAILED_CODE),
            StateDecodeFailed => Error::abort(text).with_code(base::STATE_DECODE_FAILED_CODE),
            _ => Error::fault(text),
        }
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

/// Map a native protocol action error (`sys::Error`) to a VM error code at the
/// `sys::Error -> ItrErr` conversion points. An `Abort` (canonical state read
/// or persisted-state decode failure) must keep its fatal classification and
/// its dedicated code instead of being downgraded to an ordinary action-call
/// failure (§7 of the error-system normalization design).
pub fn map_native_action_code(e: &sys::Error) -> ItrErrCode {
    if e.is_abort() {
        match e.code() {
            Some(base::STATE_DECODE_FAILED_CODE) => ItrErrCode::StateDecodeFailed,
            _ => ItrErrCode::StateReadFailed,
        }
    } else {
        ItrErrCode::ActCallError
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
    fn map_ires(self, ec: ItrErrCode, es: String) -> Result<T, ItrErr>;
}

impl<T> MapItrErr<T> for Ret<T> {
    fn map_ire(self, ec: ItrErrCode) -> Result<T, ItrErr> {
        self.map_err(|e| {
            // Preserve the classification of an `Abort` source error: map it
            // to the dedicated state codes instead of the caller-provided
            // diagnostic code (§7 of the error-system normalization design).
            let code = if e.is_abort() {
                map_native_action_code(&e)
            } else {
                ec
            };
            ItrErr::new(code, &e.to_string())
        })
    }
}

impl<T> MapItrStrErr<T> for Ret<T> {
    fn map_ires(self, ec: ItrErrCode, es: String) -> Result<T, ItrErr> {
        self.map_err(|e| {
            let code = if e.is_abort() {
                map_native_action_code(&e)
            } else {
                ec
            };
            ItrErr::new(code, &(es + &e.to_string()))
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The `ItrErr -> Error` choke point must keep canonical state read and
    /// persisted-state decode failures fatal (`Abort`) with their stable codes,
    /// and must not attach a code to validation-class `StorageError` (§7).
    #[test]
    fn state_read_failed_stays_abort_at_vm_boundary() {
        let err: Error = ItrErr::new(ItrErrCode::StateReadFailed, "backend down").into();
        assert!(err.is_abort(), "state read failure must be fatal");
        assert_eq!(err.code(), Some(base::STATE_READ_FAILED_CODE));
        assert!(!err.is_revert());
    }

    #[test]
    fn state_decode_failed_stays_abort_at_vm_boundary() {
        let err: Error = ItrErr::new(ItrErrCode::StateDecodeFailed, "bad bytes").into();
        assert!(err.is_abort(), "state decode failure must be fatal");
        assert_eq!(err.code(), Some(base::STATE_DECODE_FAILED_CODE));
    }

    #[test]
    fn validation_storage_error_keeps_fault_without_code() {
        let err: Error = ItrErr::new(ItrErrCode::StorageError, "status key too long").into();
        assert!(!err.is_abort(), "validation error must stay non-fatal");
        assert!(err.is_fault());
        assert_eq!(err.code(), None);
    }

    #[test]
    fn action_call_revert_stays_revert() {
        let err: Error = ItrErr::new(ItrErrCode::ActCallRevert, "user revert").into();
        assert!(err.is_revert());
        assert!(!err.is_abort());
    }

    /// The `sys::Error -> ItrErr` conversion used at native action dispatch
    /// points must select the dedicated abort code from the error code, and
    /// map ordinary faults to `ActCallError` (§7).
    #[test]
    fn native_action_code_preserves_abort_codes() {
        let read =
            sys::Error::abort("read failed").with_code(base::STATE_READ_FAILED_CODE);
        assert_eq!(
            map_native_action_code(&read),
            ItrErrCode::StateReadFailed
        );
        let decode =
            sys::Error::abort("decode failed").with_code(base::STATE_DECODE_FAILED_CODE);
        assert_eq!(
            map_native_action_code(&decode),
            ItrErrCode::StateDecodeFailed
        );
        let fault = sys::Error::fault("ordinary execution failure");
        assert_eq!(map_native_action_code(&fault), ItrErrCode::ActCallError);
        let revert = sys::Error::revert("user revert");
        assert_eq!(map_native_action_code(&revert), ItrErrCode::ActCallError);
    }

    /// `StateReadFailed`/`StateDecodeFailed` must round-trip through the VM
    /// runtime error chain back to `Abort` errors (test 8 of §10.2).
    #[test]
    fn state_read_failure_round_trips_through_vm() {
        let err: Error = ItrErr::new(ItrErrCode::StateReadFailed, "disk").into();
        assert!(err.is_abort());
        assert_eq!(err.code(), Some(base::STATE_READ_FAILED_CODE));
    }

    /// `map_ire` must preserve an `Abort` source classification instead of
    /// overwriting it with the caller-provided diagnostic code (§7).
    #[test]
    fn map_ire_preserves_abort_classification() {
        let src: Ret<u8> =
            Err(sys::Error::abort("backend down").with_code(base::STATE_READ_FAILED_CODE));
        let err = src.map_ire(ItrErrCode::ActCallError).unwrap_err();
        assert_eq!(err.0, ItrErrCode::StateReadFailed);
        assert!(err.1.contains("backend down"));

        let decode: Ret<u8> =
            Err(sys::Error::abort("bad bytes").with_code(base::STATE_DECODE_FAILED_CODE));
        let err = decode.map_ire(ItrErrCode::ActCallError).unwrap_err();
        assert_eq!(err.0, ItrErrCode::StateDecodeFailed);

        let fault: Ret<u8> = sys::errf!("ordinary failure");
        let err = fault.map_ire(ItrErrCode::NativeFuncError).unwrap_err();
        assert_eq!(err.0, ItrErrCode::NativeFuncError);
    }
}
