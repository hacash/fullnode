

/* 

#[repr(u8)]
#[derive(Debug, Clone, Copy)]
pub enum CallEntry {
    Main = 1,
    Abst = 2,
}

*/





pub const FN_SIGN_WIDTH: usize = 4;
pub type FnSign = [u8; FN_SIGN_WIDTH];

pub fn calc_func_sign(name: &str) -> FnSign {
    Hash::from(sys::sha3(name)).check().into_array()
}

pub fn checked_func_sign(s: &[u8]) -> VmrtRes<FnSign> {
    if s.len() != FN_SIGN_WIDTH {
        return itr_err!(ContractAddrErr, "fn sign size error")
    }
    Ok(s.to_vec().try_into().unwrap())
}



 
pub trait ToHex { fn hex(&self) -> String; }
impl ToHex for [u8; FN_SIGN_WIDTH] {
    fn hex(&self) -> String {
        hex::encode(self)
    }
}




//////////////////////////////////////////


#[repr(u8)]
#[derive(Debug, Clone, Copy, Default)]
pub enum CodeType {
    #[default] Bytecode = 0,
    IRNode      = 1,
}

impl CodeType {
    pub fn parse(n: u8) -> VmrtRes<Self> {
        let ct = n & 0b00000111;
        if ![
            Self::Bytecode as u8,
            Self::IRNode as u8,
        ].contains(&ct) {
            return itr_err_code!(ItrErrCode::CodeTypeError)
        }
        Ok(std_mem_transmute!(ct))
    }
}



//////////////////////////////////////////



pub enum FnKey {
    Abst(AbstCall),
    User(FnSign),
}


#[repr(u8)]
#[derive(Debug, Copy, Clone)]
pub enum FnConf {
    Public = 0b10000000,
}


#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct FnObj {
    pub confs: u8, // binary switch
    pub agvty: Option<FuncArgvTypes>,
    pub ctype: CodeType,
    pub codes: Vec<u8>,
}

impl FnObj {
    
    pub fn check_conf(&self, cnf: FnConf) -> bool {
        let cnfset = cnf as u8;
        self.confs & cnfset == cnfset
    } 

    pub fn create(mks: u8, codes: Vec<u8>, agvty: Option<FuncArgvTypes>) -> VmrtRes<Self> {
        let ctype = CodeType::parse(mks)?;
        Ok(Self {confs: mks & 0b11111000, agvty, ctype, codes })
    }

    pub fn into_array(self) -> Vec<u8> {
        self.codes
    }
}



#[derive(Debug, Clone)]
pub enum CallExit {
    Abort,          // throw nil
    Throw,          // throw <ERR>
    Finish,         // func ret nil
    Return,         // func ret <DATA>
    Call(Funcptr),  // call func
}




#[derive(Debug, Clone)]
pub enum CallTarget {
    Inner,
    Libidx(u8),
    Addr(ContractAddress),
}

impl CallTarget {
    pub fn idx(&self) -> u8 {
        match self {
            CallTarget::Libidx(i) => *i,
            _ => 0,
        }
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CallMode {
    #[default] Main,
    Abst,
    Outer,
    Inner,
    Library,
    Static,
    CodeCopy,
}


#[derive(Debug, Clone)]
pub struct Funcptr {
    pub mode: CallMode,
    pub target: CallTarget,
    pub fnsign: FnSign,
}


//////////////////////////////////////////


macro_rules! abst_call_type_define {
    ( $( $k:ident : $v:expr )+ ) => {

        #[allow(non_camel_case_types)]
        #[repr(u8)]
        #[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
        pub enum AbstCall {
        $(
            $k = $v,
        )+
        }
        impl AbstCall {
            pub fn check(n: u8) -> VmrtErr {
                if [$($v),+].contains(&n) {
                    return Ok(())
                }
                itr_err_fmt!(ItrErrCode::AbstTypeError, "AbstCall type {} not find", n)
            }
        }

    }
}

impl AbstCall {

    pub fn uint(&self) -> u8 {
        *self as u8
    }
    
}


abst_call_type_define! {
    Construct    : 0u8
    Change       : 1
    Append       : 2

    PermitHAC    : 15
    PermitSAT    : 16
    PermitHACD   : 17
    PermitAsset  : 18

    PayableHAC   : 25
    PayableSAT   : 26
    PayableHACD  : 27
    PayableAsset : 28

}


